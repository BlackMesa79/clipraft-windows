//! 图片解码与缩略图测试：透明 PNG、DIBV5 位域、24 位 DIB、自上而下 DIB、
//! 损坏数据与超大尺寸声明。要求：失败必须给出占位信息，绝不 panic（README「已实现范围」第 8 条）。

use clipraft_windows::model::{Clip, FormatData};
use clipraft_windows::thumbnail::{decode_dib, dimensions, generate};
use image::RgbaImage;
use std::io::Cursor;

fn put_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn f(name: &str, bytes: Vec<u8>) -> FormatData {
    FormatData {
        name: name.into(),
        bytes,
    }
}

fn encode_png(img: &RgbaImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("测试用 PNG 编码失败");
    out.into_inner()
}

fn thumbnail_pixels(png: &[u8]) -> RgbaImage {
    image::load_from_memory(png)
        .expect("缩略图应为可解码的 PNG")
        .to_rgba8()
}

/// BITMAPINFOHEADER 及扩展头共用的头部字段。
fn dib_header(
    header_size: u32,
    w: u32,
    h: i32,
    bits: u16,
    compression: u32,
    clr_used: u32,
) -> Vec<u8> {
    let mut b = vec![0u8; header_size as usize];
    put_u32(&mut b, 0, header_size);
    put_u32(&mut b, 4, w);
    put_u32(&mut b, 8, h as u32);
    put_u16(&mut b, 12, 1); // biPlanes
    put_u16(&mut b, 14, bits); // biBitCount
    put_u32(&mut b, 16, compression);
    put_u32(&mut b, 32, clr_used);
    b
}

fn dib32(w: u32, h: i32, pixels: &[u32]) -> Vec<u8> {
    let mut b = dib_header(40, w, h, 32, 0, 0);
    for p in pixels {
        b.extend_from_slice(&p.to_le_bytes());
    }
    b
}

/// 40 字节头 + BI_ALPHABITFIELDS(6)：4 个掩码紧跟头部，共 16 字节。
fn dib32_alphabitfields(w: u32, h: i32, masks: [u32; 4], pixels: &[u32]) -> Vec<u8> {
    let mut b = dib_header(40, w, h, 32, 6, 0);
    for m in masks {
        b.extend_from_slice(&m.to_le_bytes());
    }
    for p in pixels {
        b.extend_from_slice(&p.to_le_bytes());
    }
    b
}

/// 124 字节 BITMAPV5HEADER，掩码写在头内 40..56。
fn dibv5(w: u32, h: i32, compression: u32, masks: [u32; 4], pixels: &[u32]) -> Vec<u8> {
    let mut b = dib_header(124, w, h, 32, compression, 0);
    for (i, m) in masks.iter().enumerate() {
        put_u32(&mut b, 40 + i * 4, *m);
    }
    for p in pixels {
        b.extend_from_slice(&p.to_le_bytes());
    }
    b
}

/// 24 位 DIB：每行按 4 字节对齐，行顺序由 height 的正负决定。
fn dib24(w: u32, h: i32, rows: &[&[u8]]) -> Vec<u8> {
    let mut b = dib_header(40, w, h, 24, 0, 0);
    let stride = (w as usize * 3).div_ceil(4) * 4;
    for row in rows {
        let mut padded = vec![0u8; stride];
        padded[..row.len()].copy_from_slice(row);
        b.extend(padded);
    }
    b
}

/// 8 位调色板 DIB。
fn dib8(w: u32, h: i32, palette: &[[u8; 4]], indices: &[u8]) -> Vec<u8> {
    let mut b = dib_header(40, w, h, 8, 0, 0);
    let mut pal = vec![0u8; 256 * 4];
    for (i, entry) in palette.iter().enumerate() {
        pal[i * 4..i * 4 + 4].copy_from_slice(entry);
    }
    b.extend(pal);
    let stride = (w as usize).div_ceil(4) * 4;
    for row in indices.chunks((w as usize).max(1)) {
        let mut padded = vec![0u8; stride];
        padded[..row.len()].copy_from_slice(row);
        b.extend(padded);
    }
    b
}

fn lcg_blob(seed: u64, len: usize) -> Vec<u8> {
    let mut s = seed | 1;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.push((s >> 33) as u8);
    }
    out
}

#[test]
fn transparent_png_keeps_alpha_channel() {
    let mut img = RgbaImage::new(512, 512);
    for (x, _y, p) in img.enumerate_pixels_mut() {
        *p = if x < 256 {
            image::Rgba([32, 128, 96, 0])
        } else {
            image::Rgba([32, 128, 96, 255])
        };
    }
    let clip = Clip {
        formats: vec![f("PNG", encode_png(&img))],
    };
    let preview = generate(&clip);
    let png = preview.png.expect("透明 PNG 应能生成缩略图");
    assert!(preview.placeholder.is_none());

    let thumb = thumbnail_pixels(&png);
    assert!(thumb.width() <= 256 && thumb.height() <= 160);
    let left = thumb.get_pixel(1, thumb.height() / 2).0[3];
    let right = thumb.get_pixel(thumb.width() - 2, thumb.height() / 2).0[3];
    assert!(left < 16, "全透明区域应保持透明，实际 alpha={left}");
    assert!(right > 240, "不透明区域应保持不透明，实际 alpha={right}");
}

#[test]
fn png_with_huge_declared_size_is_placeholder() {
    // 只有 IHDR、声明 20000×100 的 PNG：不能真的去分配，也不能 panic。
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&20000u32.to_be_bytes());
    b.extend_from_slice(&100u32.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0]);
    b.extend_from_slice(&[0, 0, 0, 0]);

    assert_eq!(dimensions(&b, "PNG"), Some((20000, 100)));
    let preview = generate(&Clip {
        formats: vec![f("PNG", b)],
    });
    assert!(preview.png.is_none());
    let placeholder = preview.placeholder.expect("失败必须给出占位信息");
    assert!(
        placeholder.contains("PNG"),
        "占位应包含格式名：{placeholder}"
    );
    assert!(
        placeholder.contains("20000"),
        "占位应包含声明的尺寸：{placeholder}"
    );
}

#[test]
fn dibv5_alpha_channel_is_applied() {
    let alpha_mask = 0xFF00_0000u32;
    let pixels = [0xFF_FF_00_00, 0x00_00_FF_00]; // 红(不透明)、绿(全透明)
    let dib = dibv5(
        2,
        1,
        3,
        [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, alpha_mask],
        &pixels,
    );
    let img = decode_dib(&dib).expect("V5 位域 DIB 应可解码").to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(
        img.get_pixel(1, 0).0,
        [0, 255, 0, 0],
        "alpha 位为 0 时应全透明"
    );
}

#[test]
fn dibv5_alpha_zero_pixel_is_transparent() {
    let alpha_mask = 0xFF00_0000u32;
    let pixels = [0x00_00_00_00, 0x80_00_FF_00];
    let dib = dibv5(
        2,
        1,
        3,
        [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, alpha_mask],
        &pixels,
    );
    let img = decode_dib(&dib).expect("V5 位域 DIB 应可解码").to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 0], "alpha=0 必须是全透明");
    assert_eq!(
        img.get_pixel(1, 0).0,
        [0, 255, 0, 128],
        "alpha=0x80 应保留半透明"
    );
}

#[test]
fn alpha_bitfields_with_40_byte_header() {
    let dib = dib32_alphabitfields(
        1,
        1,
        [0x0000_FFFF, 0x00FF_0000, 0xFF00_0000, 0x0000_0001],
        &[0x0000_0001],
    );
    // alpha 掩码只有最低位：值 1 → 最大值 255。
    let img = decode_dib(&dib)
        .expect("ALPHABITFIELDS DIB 应可解码")
        .to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0[3], 255);
}

#[test]
fn dibv5_custom_bit_widths_are_scaled() {
    // 10 位绿色（偏移 10）、10 位红色（偏移 20）等非常规布局：按掩码位宽缩放，不 panic。
    let masks = [0x3FF0_0000, 0x000F_FC00, 0x0000_03FF, 0];
    let pixel = 0x3FF0_0000u32; // 仅红色分量满值
    let dib = dibv5(1, 1, 3, masks, &[pixel]);
    let img = decode_dib(&dib).expect("非常规掩码应可解码").to_rgba8();
    let p = img.get_pixel(0, 0).0;
    assert_eq!(p[0], 255, "10 位红色分量应缩放到 255");
    assert_eq!(p[3], 255, "alpha 掩码为 0 时视为不透明");
}

#[test]
fn dib24_bottom_up_row_order() {
    // 内存顺序为 BGR；height>0 表示自下而上，第一行是图像底边。
    let bottom = [255, 0, 0, 255, 255, 255]; // 蓝、白
    let top = [0, 0, 255, 0, 255, 0]; // 红、绿
    let dib = dib24(2, 2, &[&bottom, &top]);
    let img = decode_dib(&dib).expect("24 位 DIB 应可解码").to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255], "左上应为红");
    assert_eq!(img.get_pixel(1, 0).0, [0, 255, 0, 255], "右上应为绿");
    assert_eq!(img.get_pixel(0, 1).0, [0, 0, 255, 255], "左下应为蓝");
    assert_eq!(img.get_pixel(1, 1).0, [255, 255, 255, 255], "右下应为白");
}

#[test]
fn dib24_top_down_row_order() {
    let top = [0, 0, 255, 0, 255, 0]; // 红、绿
    let bottom = [255, 0, 0, 255, 255, 255]; // 蓝、白
    let dib = dib24(2, -2, &[&top, &bottom]);
    assert_eq!(dimensions(&dib, "CF_DIB"), Some((2, 2)), "负高度应取绝对值");
    let img = decode_dib(&dib).expect("自上而下 DIB 应可解码").to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255], "左上应为红");
    assert_eq!(img.get_pixel(0, 1).0, [0, 0, 255, 255], "左下应为蓝");
}

#[test]
fn dib32_top_down_row_order() {
    let red = 0x00FF_0000u32;
    let blue = 0x0000_00FFu32;
    let dib = dib32(1, -2, &[red, blue]);
    let img = decode_dib(&dib)
        .expect("32 位上而下 DIB 应可解码")
        .to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(img.get_pixel(0, 1).0, [0, 0, 255, 255]);
}

#[test]
fn dib8_palette_is_decoded() {
    let palette = [[0, 0, 0, 0], [255, 0, 0, 0]]; // 黑、蓝（BGR）
    let dib = dib8(1, 1, &palette, &[1]);
    let img = decode_dib(&dib)
        .expect("8 位调色板 DIB 应可解码")
        .to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [0, 0, 255, 255]);
}

#[test]
fn dimensions_reports_declared_size_or_unknown() {
    assert_eq!(dimensions(&[0u8; 8], "CF_DIB"), None);
    assert_eq!(dimensions(&[0u8; 64], "PNG"), None);
    // BITMAPCOREHEADER(12) 不在支持列表中，视为尺寸未知。
    let mut core = vec![0u8; 12];
    put_u32(&mut core, 0, 12);
    put_u32(&mut core, 4, 3);
    put_u32(&mut core, 8, 4);
    assert_eq!(dimensions(&core, "CF_DIB"), None);
    // 声明宽度/高度为整数边界值时不 panic。
    let huge = dib_header(40, 0x8000_0000, i32::MIN, 32, 0, 0);
    assert_eq!(
        dimensions(&huge, "CF_DIB"),
        Some((2_147_483_648, 2_147_483_648))
    );
}

#[test]
fn oversize_declarations_are_rejected() {
    // 超过 16384 边长限制。
    let wide = dib32(16385, 1, &[0]);
    assert!(decode_dib(&wide).is_none());
    let preview = generate(&Clip {
        formats: vec![f("CF_DIB", wide)],
    });
    assert!(preview.png.is_none());
    let placeholder = preview.placeholder.expect("超大尺寸也要给占位");
    assert!(placeholder.contains("CF_DIB"), "{placeholder}");
    assert!(placeholder.contains("16385"), "{placeholder}");

    // 像素总量超过 3200 万限制。
    let many = dib32(5000, 7000, &[0]);
    assert!(decode_dib(&many).is_none());
    let preview = generate(&Clip {
        formats: vec![f("CF_DIB", many)],
    });
    assert!(preview.placeholder.unwrap().contains("5000"));

    // 声明 4000×4000 但只有 4 字节像素：应在分配前被长度检查拦下。
    let truncated = dib32(4000, 4000, &[0]);
    assert!(decode_dib(&truncated).is_none());
}

#[test]
fn unsupported_or_absurd_headers_are_rejected() {
    for header_size in [0u32, 1, 12, 16, 39, 41, 63, 64, 100, 128, 4096, u32::MAX] {
        let mut b = vec![0u8; 128];
        put_u32(&mut b, 0, header_size);
        put_u32(&mut b, 4, 1);
        put_u32(&mut b, 8, 1);
        put_u16(&mut b, 14, 32);
        assert!(decode_dib(&b).is_none(), "header={header_size} 应被拒绝");
    }

    // 声明 0xFFFFFFFF 个调色板项：不得溢出、不得巨大分配。
    let mut palette_bomb = dib_header(40, 1, 1, 8, 0, u32::MAX);
    palette_bomb.resize(64, 0);
    assert!(decode_dib(&palette_bomb).is_none());

    // 高度为 i32::MIN（取绝对值会溢出）必须被拒绝。
    let mut min_height = dib_header(40, 1, i32::MIN, 32, 0, 0);
    min_height.resize(44, 0);
    assert!(decode_dib(&min_height).is_none());

    // 宽度为 0 或负值。
    let mut zero_width = dib_header(40, 0, 1, 32, 0, 0);
    zero_width.resize(44, 0);
    assert!(decode_dib(&zero_width).is_none());
    let mut neg_width = dib_header(40, 0xFFFF_FFFF, 1, 32, 0, 0);
    neg_width.resize(44, 0);
    assert!(decode_dib(&neg_width).is_none());
}

#[test]
fn truncated_and_random_dib_never_panic() {
    let valid = dibv5(
        4,
        4,
        3,
        [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
        &[0x80FF_0000; 16],
    );

    for cut in 0..valid.len() {
        let preview = generate(&Clip {
            formats: vec![f("CF_DIBV5", valid[..cut].to_vec())],
        });
        if preview.png.is_none() {
            assert!(
                preview.placeholder.is_some(),
                "截断到 {cut} 字节时既没有缩略图也没有占位"
            );
        }
    }

    for seed in 0..64u64 {
        let blob = lcg_blob(seed, 256);
        let preview = generate(&Clip {
            formats: vec![
                f("PNG", blob.clone()),
                f("CF_DIBV5", blob.clone()),
                f("CF_DIB", blob),
            ],
        });
        assert!(
            preview.png.is_some() || preview.placeholder.is_some(),
            "随机数据 seed={seed} 时既没有缩略图也没有占位"
        );
    }
}

#[test]
fn png_has_priority_over_dib() {
    let png = encode_png(&RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255])));
    let blue_dib = dib32(2, 2, &[0x0000_00FF; 4]);
    let preview = generate(&Clip {
        formats: vec![f("CF_DIB", blue_dib), f("PNG", png)],
    });
    let thumb = thumbnail_pixels(&preview.png.expect("PNG 优先"));
    assert_eq!(
        thumb.get_pixel(0, 0).0,
        [255, 0, 0, 255],
        "应使用 PNG 而不是 DIB"
    );
}

#[test]
fn corrupt_priority_format_falls_back_to_next() {
    let mut dib = dib32(1, 1, &[0x0000_00FF]);
    dib[0..4].copy_from_slice(&40u32.to_le_bytes());
    let preview = generate(&Clip {
        formats: vec![
            f("PNG", vec![0x89, b'P', b'N', b'G', 0, 0, 0, 0]),
            f("CF_DIBV5", vec![0; 8]),
            f("CF_DIB", dib),
        ],
    });
    assert!(preview.png.is_some(), "损坏的优先格式应回退到下一种");
}

#[test]
fn placeholder_names_the_failed_format_and_size() {
    let mut dib = dib_header(40, 100, 50, 32, 0, 0);
    dib.resize(40, 0); // 没有像素数据
    let preview = generate(&Clip {
        formats: vec![f("CF_DIB", dib)],
    });
    assert!(preview.png.is_none());
    let placeholder = preview.placeholder.expect("必须给出占位");
    assert!(placeholder.contains("CF_DIB"), "{placeholder}");
    assert!(placeholder.contains("100"), "{placeholder}");
    assert!(placeholder.contains("50"), "{placeholder}");
    assert!(placeholder.contains("无法解码"), "{placeholder}");

    // 头部都不成立时退化为「尺寸未知」而不是空白。
    let preview = generate(&Clip {
        formats: vec![f("CF_DIB", vec![0; 4])],
    });
    let placeholder = preview.placeholder.expect("必须给出占位");
    assert!(placeholder.contains("尺寸未知"), "{placeholder}");
}

#[test]
fn non_image_clip_has_no_preview() {
    let preview = generate(&Clip::default());
    assert!(preview.png.is_none());
    assert!(preview.placeholder.is_none());
    let preview = generate(&Clip {
        formats: vec![f("CF_UNICODETEXT", vec![0x41, 0, 0, 0])],
    });
    assert!(preview.png.is_none());
    assert!(preview.placeholder.is_none());
}
