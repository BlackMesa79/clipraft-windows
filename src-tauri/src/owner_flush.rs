//! 退出固化识别。仅内存上下文，不参与历史记录身份、touch 或存储去重。
use crate::{clipboard_owner::ClipboardOwner, model::Clip};
use image::GenericImageView;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Cursor};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BitmapFingerprint {
    width: usize,
    height: usize,
    rgb: [[u8; 3]; 256],
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fingerprint {
    Bitmap(Box<BitmapFingerprint>),
    Text([u8; 32]),
}
#[derive(Debug)]
pub(crate) struct CaptureContext {
    // None 表示枚举失败，不使用不完整集合进行判断。
    pub formats: Option<BTreeSet<u32>>,
    pub owner: ClipboardOwner,
    pub fingerprint: Option<Fingerprint>,
    pub bitmap_offered: bool,
    // 成功保存后填入实际记录 id；去重时关联已有记录。
    pub record_id: Option<String>,
}

/// 只让实际被删除的记录对应的上下文失效；空报告或删除其他记录均保留。
pub(crate) fn invalidate_deleted(context: &mut Option<CaptureContext>, deleted_ids: &[String]) {
    if context
        .as_ref()
        .and_then(|c| c.record_id.as_ref())
        .is_some_and(|id| deleted_ids.contains(id))
    {
        *context = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MissReason {
    CurrentNotOwnerless,
    NoPrevious,
    PreviousNotReal,
    SameOwner,
    FormatsNotSubset,
    FingerprintMissing,
    FingerprintMismatch,
}
impl MissReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CurrentNotOwnerless => "current_not_ownerless",
            Self::NoPrevious => "no_previous",
            Self::PreviousNotReal => "previous_not_real",
            Self::SameOwner => "same_owner",
            Self::FormatsNotSubset => "formats_not_subset",
            Self::FingerprintMissing => "fingerprint_missing",
            Self::FingerprintMismatch => "fingerprint_mismatch",
        }
    }
}

/// 纯判定：返回第一个未满足的前提；None 表示可以跳过，集合相等也属于子集。
pub(crate) fn miss_reason(
    previous: Option<&CaptureContext>,
    current: &CaptureContext,
) -> Option<MissReason> {
    if !current.owner.is_ownerless() {
        return Some(MissReason::CurrentNotOwnerless);
    }
    let Some(previous) = previous else {
        return Some(MissReason::NoPrevious);
    };
    if !previous.owner.is_real() {
        return Some(MissReason::PreviousNotReal);
    }
    if previous.owner.same_process_window(&current.owner) {
        return Some(MissReason::SameOwner);
    }
    let (Some(old), Some(new)) = (&previous.formats, &current.formats) else {
        return Some(MissReason::FormatsNotSubset);
    };
    let had_dib = [2, 8, 17].iter().any(|id| old.contains(id));
    if !new
        .iter()
        .all(|id| old.contains(id) || (matches!(id, 2 | 17) && had_dib))
    {
        return Some(MissReason::FormatsNotSubset);
    }
    match (&previous.fingerprint, &current.fingerprint) {
        (Some(a), Some(b)) if a == b => None,
        (Some(_), Some(_)) => Some(MissReason::FingerprintMismatch),
        _ => Some(MissReason::FingerprintMissing),
    }
}

pub(crate) fn should_skip(previous: Option<&CaptureContext>, current: &CaptureContext) -> bool {
    miss_reason(previous, current).is_none()
}

fn sample(
    width: usize,
    height: usize,
    mut pixel: impl FnMut(usize, usize) -> Option<[u8; 3]>,
) -> Option<BitmapFingerprint> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut rgb = [[0; 3]; 256];
    for (i, value) in rgb.iter_mut().enumerate() {
        // 16×16 归一化网格，包含四边；1×1 等小图允许重复坐标。
        let x = (width - 1).checked_mul(i % 16)? / 15;
        let y = (height - 1).checked_mul(i / 16)? / 15;
        *value = pixel(x, y)?;
    }
    Some(BitmapFingerprint { width, height, rgb })
}
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn channel(value: u32, mask: u32) -> u8 {
    let shift = mask.trailing_zeros();
    let maximum = u64::from(mask >> shift);
    ((u64::from((value & mask) >> shift) * 255 + maximum / 2) / maximum) as u8
}

/// 复用 DIB 校验，不分配像素副本、不扫描或哈希整个位图。
pub(crate) fn sample_dib(bytes: &[u8]) -> Option<BitmapFingerprint> {
    let layout = crate::image_normalize::dib_pixels(bytes)?;
    let header = u32_at(bytes, 0)?;
    let depth_offset = if header == 12 { 10 } else { 14 };
    let depth =
        u16::from_le_bytes(bytes.get(depth_offset..depth_offset + 2)?.try_into().ok()?) as usize;
    let pixel_bytes = depth / 8;
    let width = layout.row_bytes / pixel_bytes;
    let top_down = header != 12 && (u32_at(bytes, 8)? as i32) < 0;
    let bitfields = header != 12 && u32_at(bytes, 16)? == 3;
    let masks = if bitfields {
        [u32_at(bytes, 40)?, u32_at(bytes, 44)?, u32_at(bytes, 48)?]
    } else {
        [0xff0000, 0xff00, 0xff]
    };
    sample(width, layout.rows, |x, y| {
        let row = if top_down { y } else { layout.rows - 1 - y };
        let offset = layout
            .offset
            .checked_add(row.checked_mul(layout.stride)?)?
            .checked_add(x.checked_mul(pixel_bytes)?)?;
        let pixel = bytes.get(offset..offset.checked_add(pixel_bytes)?)?;
        let value = u32::from(pixel[0])
            | (u32::from(pixel[1]) << 8)
            | (u32::from(pixel[2]) << 16)
            | (u32::from(*pixel.get(3).unwrap_or(&0)) << 24);
        Some(masks.map(|mask| channel(value, mask)))
    })
}
fn sample_png(bytes: &[u8]) -> Option<BitmapFingerprint> {
    // PNG 必须解码；优先 DIB，避免大位图走整图解码。限制解码内存。
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(crate::storage::crypto::MAX_PLAIN_BYTES as u64);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    sample(image.width() as usize, image.height() as usize, |x, y| {
        let pixel = image.get_pixel(x as u32, y as u32).0;
        Some([pixel[0], pixel[1], pixel[2]])
    })
}
pub(crate) fn fingerprint(clip: &Clip, bitmap_offered: bool) -> Option<Fingerprint> {
    for name in ["CF_DIBV5", "CF_DIB", "PNG"] {
        if let Some(bytes) = clip.get(name) {
            let sampled = if name == "PNG" {
                sample_png(bytes)
            } else {
                sample_dib(bytes)
            };
            if let Some(sampled) = sampled {
                return Some(Fingerprint::Bitmap(Box::new(sampled)));
            }
        }
    }
    if bitmap_offered
        || ["CF_DIBV5", "CF_DIB", "PNG"]
            .iter()
            .any(|name| clip.get(name).is_some())
    {
        return None; // 有图却读不懂时，不用附带文本误判图像相同。
    }
    let bytes = clip.get("CF_UNICODETEXT")?;
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let end = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .position(|word| *word == [0, 0])?
        * 2;
    Some(Fingerprint::Text(Sha256::digest(&bytes[..end]).into()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{clipboard_owner::OwnerStatus, model::FormatData};

    pub(crate) fn dib(
        width: usize,
        height: usize,
        depth: usize,
        top_down: bool,
        masks: Option<[u32; 3]>,
    ) -> Vec<u8> {
        let header = if masks.is_some() { 124 } else { 40 };
        let stride = (width * depth).div_ceil(32) * 4;
        let mut bytes = vec![0; header + stride * height];
        bytes[..4].copy_from_slice(&(header as u32).to_le_bytes());
        bytes[4..8].copy_from_slice(&(width as i32).to_le_bytes());
        bytes[8..12].copy_from_slice(
            &(if top_down {
                -(height as i32)
            } else {
                height as i32
            })
            .to_le_bytes(),
        );
        bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&(depth as u16).to_le_bytes());
        if let Some(masks) = masks {
            bytes[16..20].copy_from_slice(&3u32.to_le_bytes());
            for (i, mask) in masks.iter().enumerate() {
                bytes[40 + i * 4..44 + i * 4].copy_from_slice(&mask.to_le_bytes());
            }
        }
        for y in 0..height {
            for x in 0..width {
                let rgb = color(x, y);
                let masks = masks.unwrap_or([0xff0000, 0xff00, 0xff]);
                let mut value = if depth == 32 { 0xc0000000 } else { 0 };
                for (channel, mask) in rgb.iter().zip(masks) {
                    let shift = mask.trailing_zeros();
                    let maximum = u64::from(mask >> shift);
                    value = (value & !mask)
                        | ((((u64::from(*channel) * maximum + 127) / 255) as u32) << shift);
                }
                let row = if top_down { y } else { height - 1 - y };
                let offset = header + row * stride + x * (depth / 8);
                bytes[offset..offset + depth / 8]
                    .copy_from_slice(&value.to_le_bytes()[..depth / 8]);
            }
        }
        bytes
    }
    fn color(x: usize, y: usize) -> [u8; 3] {
        [
            (x * 11 + y * 3) as u8,
            (y * 13 + x) as u8,
            (x * 7 + y * 17) as u8,
        ]
    }
    fn bitmap(bytes: &[u8]) -> Option<Fingerprint> {
        sample_dib(bytes).map(|v| Fingerprint::Bitmap(Box::new(v)))
    }
    pub(crate) fn owned() -> ClipboardOwner {
        ClipboardOwner {
            window: Some(123),
            pid: Some(456),
            exe: "xnviewmp.exe".into(),
            status: OwnerStatus::Live,
        }
    }
    fn context(
        owner: ClipboardOwner,
        formats: &[u32],
        fingerprint: Option<Fingerprint>,
    ) -> CaptureContext {
        CaptureContext {
            record_id: None,
            formats: Some(formats.iter().copied().collect()),
            owner,
            fingerprint,
            bitmap_offered: formats.iter().any(|id| matches!(id, 2 | 8 | 17)),
        }
    }
    fn pair() -> (CaptureContext, CaptureContext) {
        (
            context(
                owned(),
                &[8, 15, 13, 49161, 49560],
                bitmap(&dib(19, 17, 24, false, None)),
            ),
            context(
                ClipboardOwner::absent(),
                &[8, 2, 17],
                bitmap(&dib(19, 17, 32, false, None)),
            ),
        )
    }
    #[test]
    fn miss_reason_no_previous() {
        let (_, new) = pair();
        assert_eq!(miss_reason(None, &new), Some(MissReason::NoPrevious));
        assert_eq!(MissReason::NoPrevious.as_str(), "no_previous");
    }
    #[test]
    fn miss_reason_current_not_ownerless() {
        let (old, mut new) = pair();
        for status in [OwnerStatus::Live, OwnerStatus::Unknown] {
            new.owner = owned();
            new.owner.status = status;
            assert_eq!(
                miss_reason(Some(&old), &new),
                Some(MissReason::CurrentNotOwnerless)
            );
        }
    }
    #[test]
    fn miss_reason_previous_not_real() {
        let (mut old, new) = pair();
        old.owner = ClipboardOwner::absent();
        assert_eq!(
            miss_reason(Some(&old), &new),
            Some(MissReason::PreviousNotReal)
        );
        assert_eq!(MissReason::PreviousNotReal.as_str(), "previous_not_real");
    }
    #[test]
    fn miss_reason_same_owner() {
        let (old, mut new) = pair();
        new.owner = owned();
        new.owner.status = OwnerStatus::Exited;
        assert_eq!(miss_reason(Some(&old), &new), Some(MissReason::SameOwner));
        assert_eq!(MissReason::SameOwner.as_str(), "same_owner");
    }
    #[test]
    fn miss_reason_formats_not_subset_or_unknown() {
        let (mut old, mut new) = pair();
        new.formats.as_mut().unwrap().insert(777);
        assert_eq!(
            miss_reason(Some(&old), &new),
            Some(MissReason::FormatsNotSubset)
        );
        new.formats = None;
        assert_eq!(
            miss_reason(Some(&old), &new),
            Some(MissReason::FormatsNotSubset)
        );
        new.formats = Some([8].into_iter().collect());
        old.formats = None;
        assert_eq!(
            miss_reason(Some(&old), &new),
            Some(MissReason::FormatsNotSubset)
        );
        assert_eq!(MissReason::FormatsNotSubset.as_str(), "formats_not_subset");
    }
    #[test]
    fn miss_reason_fingerprint_missing_on_either_side() {
        let (mut old, mut new) = pair();
        old.fingerprint = None;
        assert_eq!(
            miss_reason(Some(&old), &new),
            Some(MissReason::FingerprintMissing)
        );
        old.fingerprint = new.fingerprint.take();
        assert_eq!(
            miss_reason(Some(&old), &new),
            Some(MissReason::FingerprintMissing)
        );
        old.fingerprint = None;
        assert_eq!(
            miss_reason(Some(&old), &new),
            Some(MissReason::FingerprintMissing)
        );
        assert_eq!(
            MissReason::FingerprintMissing.as_str(),
            "fingerprint_missing"
        );
    }
    #[test]
    fn miss_reason_fingerprint_mismatch_and_success() {
        let (old, mut new) = pair();
        assert_eq!(miss_reason(Some(&old), &new), None);
        new.fingerprint = bitmap(&dib(20, 17, 32, false, None));
        assert_eq!(
            miss_reason(Some(&old), &new),
            Some(MissReason::FingerprintMismatch)
        );
        assert_eq!(
            MissReason::FingerprintMismatch.as_str(),
            "fingerprint_mismatch"
        );
    }
    #[test]
    fn xnview_subset_24_to_32_skips() {
        let (old, new) = pair();
        assert!(should_skip(Some(&old), &new));
    }
    #[test]
    fn current_real_owner_does_not_skip() {
        let (old, mut new) = pair();
        new.owner = owned();
        new.owner.pid = Some(789);
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn unknown_owner_does_not_skip() {
        let (old, mut new) = pair();
        new.owner.status = OwnerStatus::Unknown;
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn exited_different_owner_can_skip() {
        let (old, mut new) = pair();
        new.owner = owned();
        new.owner.pid = Some(789);
        new.owner.status = OwnerStatus::Exited;
        assert!(should_skip(Some(&old), &new));
    }
    #[test]
    fn same_process_window_never_skips() {
        let (old, mut new) = pair();
        new.owner = owned();
        new.owner.status = OwnerStatus::Exited;
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn previous_requires_real_owner() {
        let (mut old, new) = pair();
        for status in [
            OwnerStatus::Absent,
            OwnerStatus::Unknown,
            OwnerStatus::Exited,
        ] {
            old.owner.status = status;
            assert!(!should_skip(Some(&old), &new));
        }
    }
    #[test]
    fn additional_format_does_not_skip() {
        let (old, mut new) = pair();
        new.formats.as_mut().unwrap().insert(777);
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn synthesis_only_expands_bitmap_and_dibv5() {
        let (mut old, mut new) = pair();
        for source in [2, 8, 17] {
            old.formats = Some([source].into_iter().collect());
            new.formats = Some([2, 17].into_iter().collect());
            assert!(should_skip(Some(&old), &new));
        }
        new.formats = Some([8].into_iter().collect());
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn different_pixels_do_not_skip() {
        let (old, mut new) = pair();
        let mut bytes = dib(19, 17, 32, false, None);
        bytes[40] ^= 1;
        new.fingerprint = bitmap(&bytes);
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn different_dimensions_do_not_skip() {
        let (old, mut new) = pair();
        new.fingerprint = bitmap(&dib(20, 17, 32, false, None));
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn opposite_row_orders_skip() {
        let (old, mut new) = pair();
        new.fingerprint = bitmap(&dib(19, 17, 32, true, None));
        assert!(should_skip(Some(&old), &new));
    }
    #[test]
    fn missing_fingerprint_does_not_skip() {
        let (mut old, mut new) = pair();
        new.fingerprint = None;
        assert!(!should_skip(Some(&old), &new));
        old.fingerprint = None;
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn failed_format_enumeration_does_not_skip() {
        let (old, mut new) = pair();
        new.formats = None;
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn cleared_context_does_not_skip() {
        let (_, new) = pair();
        assert!(!should_skip(None, &new));
    }
    fn text(value: &str) -> Clip {
        Clip {
            formats: vec![FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: value
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            }],
        }
    }
    #[test]
    fn identical_text_skips_different_text_does_not() {
        let old = context(
            owned(),
            &[13, 1, 7, 16],
            fingerprint(&text("合成文本"), false),
        );
        let mut new = context(
            ClipboardOwner::absent(),
            &[13],
            fingerprint(&text("合成文本"), false),
        );
        assert!(should_skip(Some(&old), &new));
        new.fingerprint = fingerprint(&text("另一段文本"), false);
        assert!(!should_skip(Some(&old), &new));
    }
    #[test]
    fn unreadable_bitmap_never_falls_back_to_text() {
        assert!(fingerprint(&text("相同标题"), true).is_none());
        let mut clip = text("相同标题");
        clip.formats.push(FormatData {
            name: "CF_DIB".into(),
            bytes: vec![],
        });
        assert!(fingerprint(&clip, false).is_none());
        assert!(fingerprint(&Clip::default(), false).is_none());
    }
    #[test]
    fn sample_24_bit_rgb_and_padding() {
        let sampled = sample_dib(&dib(19, 17, 24, false, None)).unwrap();
        assert_eq!(sampled.rgb[0], color(0, 0));
        assert_eq!(sampled.rgb[255], color(18, 16));
        assert_eq!(sampled.rgb[39], color(18 * 7 / 15, 16 * 2 / 15));
    }
    #[test]
    fn sample_32_bit_ignores_alpha() {
        let first = dib(19, 17, 32, false, None);
        let mut second = first.clone();
        for pixel in second[40..].as_chunks_mut::<4>().0 {
            pixel[3] = 17;
        }
        assert_eq!(sample_dib(&first), sample_dib(&second));
        assert_eq!(
            sample_dib(&first),
            sample_dib(&dib(19, 17, 24, false, None))
        );
    }
    #[test]
    fn sample_bitfields_swizzled_and_ten_bit_channels() {
        let expected = sample_dib(&dib(19, 17, 24, false, None));
        for masks in [
            [0xff, 0xff0000, 0xff00],
            [0x3ff00000, 0x000ffc00, 0x000003ff],
        ] {
            assert_eq!(sample_dib(&dib(19, 17, 32, false, Some(masks))), expected);
        }
    }
    #[test]
    fn sample_external_bitfield_masks() {
        let mut bytes = dib(19, 17, 32, false, Some([0xff, 0xff0000, 0xff00]));
        bytes.drain(52..124);
        bytes[..4].copy_from_slice(&40u32.to_le_bytes());
        assert_eq!(
            sample_dib(&bytes),
            sample_dib(&dib(19, 17, 24, false, None))
        );
    }
    #[test]
    fn sample_negative_height() {
        assert_eq!(
            sample_dib(&dib(19, 17, 24, true, None)),
            sample_dib(&dib(19, 17, 32, false, None))
        );
    }
    #[test]
    fn sample_one_by_one() {
        let mut bytes = dib(1, 1, 24, true, None);
        bytes[40..43].copy_from_slice(&[3, 2, 1]);
        let sampled = sample_dib(&bytes).unwrap();
        assert_eq!((sampled.width, sampled.height), (1, 1));
        assert_eq!(sampled.rgb, [[1, 2, 3]; 256]);
    }
    #[test]
    fn unsupported_or_incomplete_dib_has_no_fingerprint() {
        assert!(sample_dib(&[]).is_none());
        let mut bytes = dib(19, 17, 24, false, None);
        bytes.truncate(40);
        assert!(sample_dib(&bytes).is_none());
    }
    #[test]
    fn png_and_dib_use_same_sample_coordinates() {
        let image = image::RgbaImage::from_fn(19, 17, |x, y| {
            let [r, g, b] = color(x as usize, y as usize);
            image::Rgba([r, g, b, 29])
        });
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        assert_eq!(
            sample_png(bytes.get_ref()),
            sample_dib(&dib(19, 17, 24, false, None))
        );
        let clip = Clip {
            formats: vec![FormatData {
                name: "PNG".into(),
                bytes: bytes.into_inner(),
            }],
        };
        assert!(matches!(
            fingerprint(&clip, true),
            Some(Fingerprint::Bitmap(_))
        ));
    }
    #[test]
    fn large_dib_sampling_timing() {
        use std::{hint::black_box, time::Instant};
        // 与实测 4835×7253、32bpp、124 字节头完全等长；初始化不计入抽样计时。
        let mut bytes = vec![0u8; 140_273_144];
        bytes[..4].copy_from_slice(&124u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&4835i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&7253i32.to_le_bytes());
        bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
        let clip = Clip {
            formats: vec![FormatData {
                name: "CF_DIBV5".into(),
                bytes,
            }],
        };
        let start = Instant::now();
        assert!(fingerprint(black_box(&clip), true).is_some());
        let cold = start.elapsed();
        let start = Instant::now();
        for _ in 0..10000 {
            black_box(fingerprint(black_box(&clip), true).unwrap());
        }
        eprintln!("owner_flush benchmark bytes=140273144 samples=256 first_us={:.3} warm_mean_us={:.3} iterations=10000", cold.as_secs_f64() * 1e6, start.elapsed().as_secs_f64() * 1e6 / 10000.0);
    }
}
