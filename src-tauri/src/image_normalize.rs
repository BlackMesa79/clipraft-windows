//! 采集阶段去除 HGLOBAL 尾部填充；解析不确定时原样保留。
pub fn normalize(name: &str, bytes: &mut Vec<u8>) {
    let length = match name {
        "PNG" => png_length(bytes),
        "CF_DIB" | "CF_DIBV5" => dib_length(bytes),
        _ => None,
    };
    if let Some(length) = length {
        bytes.truncate(length);
    }
}
fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn png_length(bytes: &[u8]) -> Option<usize> {
    if bytes.get(..8)? != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let mut offset = 8usize;
    let mut data_seen = false;
    loop {
        let size = u32::from_be_bytes(bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?)
            as usize;
        let end = offset.checked_add(12)?.checked_add(size)?;
        let chunk = bytes.get(offset..end)?;
        let kind = &chunk[4..8];
        if offset == 8 && (kind != b"IHDR" || size != 13) {
            return None;
        }
        if offset != 8 && kind == b"IHDR" {
            return None;
        }
        if kind == b"IDAT" {
            data_seen = true;
        }
        if kind == b"IEND" {
            return (size == 0 && data_seen && chunk[8..12] == [174, 66, 96, 130]).then_some(end);
        }
        offset = end;
    }
}
fn dib_length(bytes: &[u8]) -> Option<usize> {
    let header = u32_at(bytes, 0)? as usize;
    if !matches!(header, 12 | 40 | 52 | 56 | 108 | 124) || header > bytes.len() {
        return None;
    }
    let (width, height, depth, compression, declared, colors, masks) = if header == 12 {
        let width = u16_at(bytes, 4)? as usize;
        let height = u16_at(bytes, 6)? as usize;
        let depth = u16_at(bytes, 10)? as usize;
        if u16_at(bytes, 8)? != 1 || !matches!(depth, 1 | 4 | 8 | 24) {
            return None;
        }
        (
            width,
            height,
            depth,
            0,
            0,
            if depth <= 8 { 1usize << depth } else { 0 },
            0,
        )
    } else {
        let width = u32_at(bytes, 4)? as i32;
        let height = u32_at(bytes, 8)? as i32;
        let depth = u16_at(bytes, 14)? as usize;
        let compression = u32_at(bytes, 16)?;
        let declared = u32_at(bytes, 20)? as usize;
        let used = u32_at(bytes, 32)? as usize;
        if width <= 0 || height == 0 || height == i32::MIN || u16_at(bytes, 12)? != 1 {
            return None;
        }
        let valid = match compression {
            0 => matches!(depth, 1 | 4 | 8 | 16 | 24 | 32),
            1 => depth == 8 && height > 0 && declared > 0,
            2 => depth == 4 && height > 0 && declared > 0,
            3 | 6 => matches!(depth, 16 | 32),
            4 | 5 => depth == 0 && height > 0 && declared > 0,
            _ => false,
        };
        if !valid || (depth > 0 && depth <= 8 && used > 1usize << depth) {
            return None;
        }
        let masks = if compression == 3 {
            3
        } else if compression == 6 {
            4
        } else {
            0
        };
        if masks > 0 {
            if header != 40 && header < 40 + masks * 4 {
                return None;
            }
            let mut seen = 0u32;
            for i in 0..masks {
                let mask = u32_at(bytes, 40 + i * 4)?;
                let shifted = mask.checked_shr(mask.trailing_zeros()).unwrap_or(0);
                if mask == 0
                    || mask & seen != 0
                    || (depth == 16 && mask > 0xffff)
                    || shifted & shifted.wrapping_add(1) != 0
                {
                    return None;
                }
                seen |= mask;
            }
        }
        let colors = if used != 0 {
            used
        } else if depth > 0 && depth <= 8 {
            1usize << depth
        } else {
            0
        };
        (
            width as usize,
            height.unsigned_abs() as usize,
            depth,
            compression,
            declared,
            colors,
            if header == 40 { masks * 4 } else { 0 },
        )
    };
    if width == 0 || height == 0 {
        return None;
    }
    let offset = header
        .checked_add(masks)?
        .checked_add(colors.checked_mul(if header == 12 { 3 } else { 4 })?)?;
    let payload = if matches!(compression, 0 | 3 | 6) {
        let stride = width
            .checked_mul(depth)?
            .checked_add(31)?
            .checked_div(32)?
            .checked_mul(4)?;
        let expected = stride.checked_mul(height)?;
        if declared != 0 && declared < expected {
            return None;
        }
        // 非零 sizeImage 可能包含调用者声明的有效数据，保守保留。
        declared.max(expected)
    } else {
        declared
    };
    let mut end = offset.checked_add(payload)?;
    if header == 124 {
        let profile = u32_at(bytes, 112)? as usize;
        let size = u32_at(bytes, 116)? as usize;
        let color_space = u32_at(bytes, 56)?;
        if profile != 0 || size != 0 || matches!(color_space, 0x4c494e4b | 0x4d424544) {
            // 剪贴板 packed DIB 的 ICC 数据位于像素之后，不能当成尾部填充丢弃。
            if profile < end || size == 0 {
                return None;
            }
            end = profile.checked_add(size)?;
        }
    }
    (end <= bytes.len()).then_some(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, FormatData};
    fn dib(header: usize, depth: u16, compression: u32, used: u32, declared: u32) -> Vec<u8> {
        let mut b = vec![0; header];
        b[..4].copy_from_slice(&(header as u32).to_le_bytes());
        b[4..8].copy_from_slice(&3i32.to_le_bytes());
        b[8..12].copy_from_slice(&2i32.to_le_bytes());
        b[12..14].copy_from_slice(&1u16.to_le_bytes());
        b[14..16].copy_from_slice(&depth.to_le_bytes());
        b[16..20].copy_from_slice(&compression.to_le_bytes());
        b[20..24].copy_from_slice(&declared.to_le_bytes());
        b[32..36].copy_from_slice(&used.to_le_bytes());
        let count = if compression == 3 {
            3
        } else if compression == 6 {
            4
        } else {
            0
        };
        if count > 0 {
            if header == 40 {
                b.resize(40 + count * 4, 0);
            }
            for (i, value) in [0xff0000u32, 0xff00, 0xff, 0xff000000]
                .iter()
                .take(count)
                .enumerate()
            {
                b[40 + i * 4..44 + i * 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        let colors = if used > 0 {
            used as usize
        } else if depth > 0 && depth <= 8 {
            1 << depth
        } else {
            0
        };
        b.resize(b.len() + colors * 4, 0);
        let size = if declared > 0 {
            declared as usize
        } else {
            (3 * depth as usize).div_ceil(32) * 4 * 2
        };
        b.resize(b.len() + size, 0x5a);
        b
    }
    fn equal_after_padding(name: &str, bytes: Vec<u8>) {
        let mut padded = bytes.clone();
        padded.extend([9; 97]);
        normalize(name, &mut padded);
        assert_eq!(padded, bytes);
        let clip = |bytes| Clip {
            formats: vec![FormatData {
                name: name.into(),
                bytes,
            }],
        };
        assert_eq!(clip(padded).hash(), clip(bytes).hash());
    }
    fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = (payload.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out.extend_from_slice(if kind == b"IEND" {
            &[174, 66, 96, 130]
        } else {
            &[0, 0, 0, 0]
        });
        out
    }
    fn png_signature() -> Vec<u8> {
        b"\x89PNG\r\n\x1a\n".to_vec()
    }
    fn ihdr() -> Vec<u8> {
        let mut payload = vec![0u8; 13];
        payload[..4].copy_from_slice(&1u32.to_be_bytes());
        payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        payload[8] = 8;
        payload[9] = 6;
        chunk(b"IHDR", &payload)
    }
    /// 断言原样保留（连尾部填充也不动）。
    fn preserved(name: &str, bytes: &[u8], label: &str) {
        let mut out = bytes.to_vec();
        out.extend([3; 64]);
        let old = out.clone();
        normalize(name, &mut out);
        assert_eq!(old, out, "{label}");
    }
    /// 任意原始数据的结果只能是它的前缀。
    fn prefix_only(bytes: &[u8], label: &str) {
        for name in ["PNG", "CF_DIB", "CF_DIBV5"] {
            let mut out = bytes.to_vec();
            normalize(name, &mut out);
            assert!(out.len() <= bytes.len(), "{label}/{name} 不能增长");
            assert_eq!(&bytes[..out.len()], &out[..], "{label}/{name} 只能尾部截断");
        }
    }
    /// 手写 packed DIB 头部，字段可以任意伪造后取字节。
    struct Dib {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        depth: u16,
        compression: u32,
        declared: u32,
        used: u32,
        payload: usize,
    }
    impl Dib {
        /// 3×2、平面数 1、无压缩、无调色板，尾部带 64 字节载荷。
        fn new(size: u32, depth: u16) -> Self {
            Self {
                size,
                width: 3,
                height: 2,
                planes: 1,
                depth,
                compression: 0,
                declared: 0,
                used: 0,
                payload: 64,
            }
        }
        fn bytes(&self) -> Vec<u8> {
            let mut b = vec![0u8; 124];
            b[0..4].copy_from_slice(&self.size.to_le_bytes());
            b[4..8].copy_from_slice(&self.width.to_le_bytes());
            b[8..12].copy_from_slice(&self.height.to_le_bytes());
            b[12..14].copy_from_slice(&self.planes.to_le_bytes());
            b[14..16].copy_from_slice(&self.depth.to_le_bytes());
            b[16..20].copy_from_slice(&self.compression.to_le_bytes());
            b[20..24].copy_from_slice(&self.declared.to_le_bytes());
            b[32..36].copy_from_slice(&self.used.to_le_bytes());
            b.resize(124 + self.payload, 0x5a);
            b
        }
    }

    #[test]
    fn png_chunks_ignore_padding_and_embedded_iend_text() {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        let png = out.into_inner();
        equal_after_padding("PNG", png.clone());
        let mut missing = png[..png.len() - 12].to_vec();
        missing.extend(b"IEND");
        let original = missing.clone();
        normalize("PNG", &mut missing);
        assert_eq!(missing, original);
        let mut malformed = png.clone();
        malformed[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        let original = malformed.clone();
        normalize("PNG", &mut malformed);
        assert_eq!(malformed, original);
    }
    #[test]
    fn dib_headers_masks_palettes_and_row_alignment_deduplicate() {
        for (header, depth, compression, colors, size) in [
            (40, 24, 0, 0, 0),
            (40, 1, 0, 0, 0),
            (40, 4, 0, 3, 0),
            (40, 8, 0, 0, 0),
            (40, 16, 0, 0, 0),
            (40, 32, 3, 0, 0),
            (52, 32, 3, 0, 0),
            (56, 32, 6, 0, 0),
            (108, 32, 3, 0, 0),
            (124, 32, 3, 0, 0),
            (40, 24, 0, 2, 24),
            (40, 8, 1, 2, 8),
            (40, 4, 2, 2, 8),
            (124, 0, 4, 0, 8),
            (124, 0, 5, 0, 8),
            (40, 32, 6, 0, 24),
        ] {
            equal_after_padding("CF_DIB", dib(header, depth, compression, colors, size));
        }
        let mut top_down = dib(124, 32, 0, 0, 0);
        top_down[8..12].copy_from_slice(&(-2i32).to_le_bytes());
        equal_after_padding("CF_DIBV5", top_down);
    }
    #[test]
    fn core_header_and_v5_profile_are_preserved() {
        let mut core = vec![0; 12 + 6 + 8];
        core[..4].copy_from_slice(&12u32.to_le_bytes());
        core[4..6].copy_from_slice(&3u16.to_le_bytes());
        core[6..8].copy_from_slice(&2u16.to_le_bytes());
        core[8..10].copy_from_slice(&1u16.to_le_bytes());
        core[10..12].copy_from_slice(&1u16.to_le_bytes());
        equal_after_padding("CF_DIB", core);
        let mut profile = dib(124, 32, 0, 0, 0);
        let offset = profile.len() as u32;
        profile[56..60].copy_from_slice(&0x4d424544u32.to_le_bytes());
        profile[112..116].copy_from_slice(&offset.to_le_bytes());
        profile[116..120].copy_from_slice(&7u32.to_le_bytes());
        profile.extend(b"profile");
        equal_after_padding("CF_DIBV5", profile);
    }
    #[test]
    fn malformed_headers_and_truncated_payloads_are_not_trimmed() {
        let good = dib(40, 24, 0, 0, 0);
        for len in 0..good.len() {
            let mut bytes = good[..len].to_vec();
            let old = bytes.clone();
            normalize("CF_DIB", &mut bytes);
            assert_eq!(old, bytes);
        }
        for (offset, value) in [
            (0, 0),
            (0, 39),
            (0, 41),
            (0, u32::MAX),
            (4, 0),
            (4, u32::MAX),
            (8, 0),
            (8, 0x80000000),
            (12, 0),
            (16, 99),
            (20, 1),
            (20, u32::MAX),
            (32, u32::MAX),
        ] {
            let mut bytes = good.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            bytes.extend([7; 30]);
            let old = bytes.clone();
            normalize("CF_DIB", &mut bytes);
            assert_eq!(old, bytes, "offset={offset} value={value}");
        }
        for header in [40, 124] {
            let mut bytes = dib(header, 32, 3, 0, 0);
            bytes[40..44].fill(0);
            let old = bytes.clone();
            normalize("CF_DIB", &mut bytes);
            assert_eq!(old, bytes);
        }
    }
    #[test]
    fn png_adversarial_headers_never_panic_and_preserve_incomplete_streams() {
        let mut good = png_signature();
        good.extend(ihdr());
        good.extend(chunk(b"IDAT", &[0x78, 0x9c, 0x01]));
        good.extend(chunk(b"IEND", &[]));
        equal_after_padding("PNG", good.clone());
        // 任意前缀都缺少完整的 IEND，必须原样保留。
        for len in 0..good.len() {
            preserved("PNG", &good[..len], &format!("截断到 {len}"));
            prefix_only(&good[..len], &format!("截断到 {len}"));
        }
        // 零长度块链不应死循环，且仍能在 IEND 处精确截断。
        let mut zeros = png_signature();
        zeros.extend(ihdr());
        for _ in 0..5000 {
            zeros.extend(chunk(b"IDAT", &[]));
        }
        zeros.extend(chunk(b"IEND", &[]));
        let mut padded = zeros.clone();
        padded.extend([0xee; 48]);
        normalize("PNG", &mut padded);
        assert_eq!(padded, zeros, "零长度块链应在 IEND 末尾终止");
        // IEND 的 CRC 错误 / 长度非零：保留。
        let mut bad_crc = good.clone();
        let tail = bad_crc.len() - 4;
        bad_crc[tail..].copy_from_slice(&[0, 0, 0, 0]);
        preserved("PNG", &bad_crc, "IEND CRC 错误");
        let mut sized_iend = png_signature();
        sized_iend.extend(ihdr());
        sized_iend.extend(chunk(b"IDAT", &[1]));
        sized_iend.extend(chunk(b"IEND", &[0]));
        preserved("PNG", &sized_iend, "IEND 长度非零");
        // IDAT 之前出现 IEND、重复 IHDR、首块不是 IHDR、IHDR 长度错误：保留。
        let mut no_data = png_signature();
        no_data.extend(ihdr());
        no_data.extend(chunk(b"IEND", &[]));
        preserved("PNG", &no_data, "缺少 IDAT");
        let mut twice = png_signature();
        twice.extend(ihdr());
        twice.extend(ihdr());
        twice.extend(chunk(b"IDAT", &[1]));
        twice.extend(chunk(b"IEND", &[]));
        preserved("PNG", &twice, "重复 IHDR");
        let mut first = good.clone();
        first[12..16].copy_from_slice(b"tEXt");
        preserved("PNG", &first, "首块不是 IHDR");
        let mut ihdr_size = good.clone();
        ihdr_size[8..12].copy_from_slice(&14u32.to_be_bytes());
        preserved("PNG", &ihdr_size, "IHDR 长度不是 13");
        // 块长度越界 / 块头被截断：保留。
        let mut over = png_signature();
        over.extend(0xffff_fff0u32.to_be_bytes());
        over.extend_from_slice(b"IDAT");
        over.extend([0xaa; 32]);
        preserved("PNG", &over, "块长度越界");
        let mut short_header = png_signature();
        short_header.extend(ihdr());
        short_header.extend(chunk(b"IDAT", &[9; 8]));
        let at = short_header.len() as u32;
        short_header.extend(at.to_be_bytes());
        short_header.extend_from_slice(b"IEND");
        preserved("PNG", &short_header, "块头缺 CRC");
        // 空输入与只有签名。
        preserved("PNG", &[], "空输入");
        preserved("PNG", &png_signature(), "只有签名");
        // 携带大量垃圾块的数据不应拖慢到超时，且结果只能是前缀。
        let mut junk = png_signature();
        junk.extend(ihdr());
        for n in 0..2000 {
            junk.extend(chunk(b"tEXt", &[(n % 251) as u8; 7]));
        }
        junk.extend(chunk(b"IDAT", &[1]));
        junk.extend(chunk(b"IEND", &[]));
        prefix_only(&junk, "大量垃圾块");
        equal_after_padding("PNG", junk);
    }
    #[test]
    fn adversarial_dib_fields_never_panic_and_never_grow() {
        let mut cases: Vec<Vec<u8>> = Vec::new();
        // 异常 biSize。
        for size in [
            0u32,
            11,
            12,
            13,
            16,
            40,
            52,
            56,
            108,
            124,
            125,
            200,
            u32::MAX,
        ] {
            cases.push(
                Dib {
                    size,
                    ..Dib::new(40, 24)
                }
                .bytes(),
            );
        }
        // 负高度 / 零高度 / 极端高度（i32::MIN 的负值不能用 abs 取）。
        for height in [0i32, -1, -2, -3, i32::MIN, i32::MIN + 1, i32::MAX] {
            cases.push(
                Dib {
                    height,
                    ..Dib::new(40, 24)
                }
                .bytes(),
            );
        }
        for width in [0i32, -1, i32::MIN, i32::MAX] {
            cases.push(
                Dib {
                    width,
                    ..Dib::new(40, 24)
                }
                .bytes(),
            );
        }
        // 各种位深（含不支持的值）。
        for depth in [0u16, 2, 3, 5, 6, 7, 9, 15, 16, 17, 31, 32, 33, 64, u16::MAX] {
            cases.push(Dib::new(40, depth).bytes());
        }
        // 各种压缩方式（含 RLE、BI_BITFIELDS、BI_JPEG/BI_PNG 与非法值）。
        for compression in [1u32, 2, 3, 4, 5, 6, 7, 8, u32::MAX] {
            cases.push(
                Dib {
                    compression,
                    declared: 24,
                    ..Dib::new(40, 24)
                }
                .bytes(),
            );
        }
        for declared in [1u32, 7, 0xffff, u32::MAX] {
            cases.push(
                Dib {
                    declared,
                    ..Dib::new(40, 24)
                }
                .bytes(),
            );
        }
        // 极大的 biClrUsed。
        for used in [1u32, 256, 257, 0xffff, u32::MAX] {
            cases.push(
                Dib {
                    used,
                    ..Dib::new(40, 8)
                }
                .bytes(),
            );
            cases.push(
                Dib {
                    used,
                    ..Dib::new(40, 24)
                }
                .bytes(),
            );
        }
        for planes in [0u16, 2, u16::MAX] {
            cases.push(
                Dib {
                    planes,
                    ..Dib::new(40, 24)
                }
                .bytes(),
            );
        }
        // 乘法 / 加法溢出候选。
        for (size, depth, compression) in
            [(40u32, 32u16, 0u32), (40, 1, 0), (124, 32, 3), (40, 32, 6)]
        {
            for height in [i32::MAX, i32::MIN + 1] {
                cases.push(
                    Dib {
                        size,
                        width: i32::MAX,
                        height,
                        depth,
                        compression,
                        declared: 0,
                        ..Dib::new(size, depth)
                    }
                    .bytes(),
                );
                cases.push(
                    Dib {
                        size,
                        width: i32::MAX,
                        height,
                        depth,
                        compression,
                        declared: u32::MAX,
                        used: u32::MAX,
                        payload: 0,
                        ..Dib::new(size, depth)
                    }
                    .bytes(),
                );
            }
        }
        // 确定性伪随机扫描，覆盖字段组合。
        let mut seed = 0x1234_5678u32;
        for _ in 0..2000 {
            let mut next = || {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                seed
            };
            let size = [12u32, 40, 52, 56, 108, 124][(next() % 6) as usize];
            let width = (next() % 9) as i32 - 2;
            let height = (next() % 9) as i32 - 2;
            let planes = if next() % 7 == 0 { next() as u16 } else { 1 };
            let depth = [0u16, 1, 4, 8, 16, 24, 32, 33][(next() % 8) as usize];
            let compression = next() % 9;
            let declared = if next() % 4 == 0 { next() } else { 0 };
            let used = if next() % 5 == 0 { next() } else { 0 };
            let payload = (next() % 200) as usize;
            cases.push(
                Dib {
                    size,
                    width,
                    height,
                    planes,
                    depth,
                    compression,
                    declared,
                    used,
                    payload,
                }
                .bytes(),
            );
        }
        for (index, bytes) in cases.iter().enumerate() {
            prefix_only(bytes, &format!("case {index}"));
        }
    }
    #[test]
    fn dib_invalid_field_combinations_are_preserved_verbatim() {
        // 不支持的位深。
        for depth in [0u16, 2, 3, 5, 6, 7, 9, 15, 17, 31, 33, 64, u16::MAX] {
            preserved("CF_DIB", &dib(40, depth, 0, 0, 0), &format!("位深 {depth}"));
        }
        // 平面数不是 1。
        for planes in [0u16, 2] {
            let mut bytes = dib(40, 24, 0, 0, 0);
            bytes[12..14].copy_from_slice(&planes.to_le_bytes());
            preserved("CF_DIB", &bytes, &format!("planes {planes}"));
        }
        // 非法压缩方式。
        for compression in [7u32, 8, u32::MAX] {
            preserved(
                "CF_DIB",
                &dib(40, 24, compression, 0, 24),
                &format!("压缩 {compression}"),
            );
        }
        // BI_RLE4 必须是 4 位深，BI_RLE8 必须有非零 sizeImage。
        preserved("CF_DIB", &dib(40, 8, 2, 0, 8), "BI_RLE4 位深必须是 4");
        preserved("CF_DIB", &dib(40, 8, 1, 0, 0), "BI_RLE8 缺少 sizeImage");
        // 极大的 biClrUsed 超过位深允许的调色板容量。
        for used in [257u32, 0xffff] {
            preserved("CF_DIB", &dib(40, 8, 0, used, 0), &format!("调色板 {used}"));
        }
        // BI_BITFIELDS 掩码：不连续、重复、16 位深超过 16 位、为零。
        for mask in [0xf0f0_0000u32, 0x00ff_00ff, u32::MAX] {
            let mut bytes = dib(40, 32, 3, 0, 0);
            bytes[40..44].copy_from_slice(&mask.to_le_bytes());
            preserved("CF_DIB", &bytes, &format!("掩码 {mask:#x}"));
        }
        let mut duplicate = dib(40, 32, 3, 0, 0);
        duplicate[44..48].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
        preserved("CF_DIB", &duplicate, "掩码重复");
        let mut wide = dib(40, 16, 3, 0, 0);
        wide[40..44].copy_from_slice(&0xffff_0000u32.to_le_bytes());
        preserved("CF_DIB", &wide, "16 位深掩码超过 16 位");
        // V5 的 ICC 数据落在像素区之前，无法安全判定尾部填充。
        let mut profile = dib(124, 32, 0, 0, 0);
        profile[112..116].copy_from_slice(&40u32.to_le_bytes());
        profile[116..120].copy_from_slice(&4u32.to_le_bytes());
        preserved("CF_DIBV5", &profile, "ICC 描述落在头部内部");
        // 核心头部的非法位深与平面数。
        for (depth, planes) in [(2u16, 1u16), (16, 1), (32, 1), (8, 2)] {
            let mut bytes = vec![0u8; 12 + 6 + 8];
            bytes[..4].copy_from_slice(&12u32.to_le_bytes());
            bytes[4..6].copy_from_slice(&3u16.to_le_bytes());
            bytes[6..8].copy_from_slice(&2u16.to_le_bytes());
            bytes[8..10].copy_from_slice(&planes.to_le_bytes());
            bytes[10..12].copy_from_slice(&depth.to_le_bytes());
            preserved(
                "CF_DIB",
                &bytes,
                &format!("核心头部位深 {depth} 平面 {planes}"),
            );
        }
        // 未知格式名绝不裁剪。
        let mut unknown = dib(40, 24, 0, 0, 0);
        unknown.extend([0x5a; 96]);
        let old = unknown.clone();
        normalize("HTML Format", &mut unknown);
        assert_eq!(old, unknown);
        normalize("CF_TEXT", &mut unknown);
        assert_eq!(old, unknown);
        normalize("png", &mut unknown);
        assert_eq!(old, unknown);
    }
}
