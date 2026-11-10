//! 小型边界测试与显式启用的 30 MiB 合成基准，不使用用户数据。
use super::{
    crypto::{self, CompressedPayload},
    predict,
};
use std::time::Instant;

pub(super) fn fixture(
    width: usize,
    height: i32,
    depth: usize,
    header: usize,
    bitfields: bool,
) -> Vec<u8> {
    let rows = height.unsigned_abs() as usize;
    let stride = (width * depth).div_ceil(32) * 4;
    let offset = header + if header == 40 && bitfields { 12 } else { 0 };
    let mut out = vec![0; offset + stride * rows];
    out[..4].copy_from_slice(&(header as u32).to_le_bytes());
    if header == 12 {
        out[4..6].copy_from_slice(&(width as u16).to_le_bytes());
        out[6..8].copy_from_slice(&(height as u16).to_le_bytes());
        out[8..10].copy_from_slice(&1u16.to_le_bytes());
        out[10..12].copy_from_slice(&(depth as u16).to_le_bytes());
    } else {
        out[4..8].copy_from_slice(&(width as i32).to_le_bytes());
        out[8..12].copy_from_slice(&height.to_le_bytes());
        out[12..14].copy_from_slice(&1u16.to_le_bytes());
        out[14..16].copy_from_slice(&(depth as u16).to_le_bytes());
        out[16..20].copy_from_slice(&(if bitfields { 3u32 } else { 0 }).to_le_bytes());
        out[20..24].copy_from_slice(&((stride * rows) as u32).to_le_bytes());
        if bitfields {
            for (i, mask) in [0xff0000u32, 0xff00, 0xff].iter().enumerate() {
                out[40 + i * 4..44 + i * 4].copy_from_slice(&mask.to_le_bytes());
            }
        }
    }
    for (i, value) in out[offset..].iter_mut().enumerate() {
        *value = (i.wrapping_mul(17) ^ (i / 113)) as u8;
    }
    out
}
fn roundtrip(plain: &[u8]) {
    let predicted = predict::encode(plain).unwrap();
    assert_eq!(predict::decode(predicted, plain.len()).unwrap(), plain);
    let blob = crypto::encrypt_blob(&[87; 32], "fixture", plain).unwrap();
    assert_eq!(crypto::decrypt(&[87; 32], "fixture", &blob).unwrap(), plain);
}
#[test]
fn rgb24_positive_negative_and_padding_roundtrip() {
    for height in [1, 2, 7, -1, -2, -7] {
        roundtrip(&fixture(3, height, 24, 40, false));
    }
}
#[test]
fn rgb32_positive_negative_roundtrip() {
    for height in [1, 9, -1, -9] {
        roundtrip(&fixture(19, height, 32, 124, false));
    }
}
#[test]
fn bitfields_external_and_embedded_masks_roundtrip() {
    for header in [40, 52, 56, 108, 124] {
        for depth in [24, 32] {
            for height in [5, -5] {
                roundtrip(&fixture(3, height, depth, header, true));
            }
        }
    }
}
#[test]
fn one_pixel_and_wide_single_row_roundtrip() {
    for width in [1, 32769] {
        for depth in [24, 32] {
            roundtrip(&fixture(width, 1, depth, 40, false));
        }
    }
    roundtrip(&fixture(3, 2, 24, 12, false));
}
#[test]
fn palette_padding_declared_extra_and_icc_tail_are_unchanged() {
    let mut plain = fixture(3, 4, 24, 124, true);
    plain[32..36].copy_from_slice(&2u32.to_le_bytes());
    plain.splice(124..124, [51, 72, 29, 0, 65, 85, 95, 0]);
    plain.extend([7; 9]);
    plain[20..24].copy_from_slice(&57u32.to_le_bytes());
    let profile = plain.len() as u32;
    plain[112..116].copy_from_slice(&profile.to_le_bytes());
    plain[116..120].copy_from_slice(&11u32.to_le_bytes());
    plain.extend([8; 11]);
    plain.extend([91; 17]);
    let encoded = predict::encode(&plain).unwrap();
    assert_eq!(&encoded[..132], &plain[..132]);
    for row in 0..4 {
        assert_eq!(
            &encoded[132 + row * 12 + 9..132 + row * 12 + 12],
            &plain[132 + row * 12 + 9..132 + row * 12 + 12]
        );
    }
    assert_eq!(&encoded[180..plain.len()], &plain[180..]);
    roundtrip(&plain);
}
#[test]
fn unsupported_or_incomplete_dib_uses_baseline() {
    let normal = fixture(3, 2, 24, 40, false);
    for end in 0..normal.len() {
        assert!(predict::encode(&normal[..end]).is_none());
    }
    for (offset, value) in [
        (0, 41u32),
        (4, u32::MAX),
        (8, 0),
        (16, 6),
        (20, 1),
        (32, u32::MAX),
    ] {
        let mut plain = normal.clone();
        plain[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(predict::encode(&plain).is_none());
        assert!(!matches!(
            crypto::select_payload(&plain).unwrap(),
            CompressedPayload::Predicted(_)
        ));
    }
    assert!(predict::encode(&fixture(3, 2, 16, 40, false)).is_none());
}
#[test]
fn inverse_rejects_unknown_filter_and_length_without_panicking() {
    let plain = fixture(3, 2, 24, 40, false);
    let predicted = predict::encode(&plain).unwrap();
    for filter in [0, 1, 3, 4, 255] {
        let mut changed = predicted.clone();
        *changed.last_mut().unwrap() = filter;
        assert!(predict::decode(changed, plain.len()).is_err());
    }
    for end in 0..predicted.len() {
        assert!(predict::decode(predicted[..end].to_vec(), plain.len()).is_err());
    }
    let mut changed = predicted;
    changed[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(predict::decode(changed, plain.len()).is_err());
}

pub(super) fn sample(kind: &str, width: usize, height: usize) -> Vec<u8> {
    let mut plain = fixture(width, height as i32, 32, 124, true);
    let mut random = 0x12345678u32;
    for (i, pixel) in plain[124..].as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i % width, i / width);
        for (channel, value) in pixel[..3].iter_mut().enumerate() {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            *value = match kind {
                "screenshot" => {
                    if x % 160 < 2 || y % 80 < 2 {
                        32
                    } else {
                        ((x / 16 + y / 8 + channel * 64) % 256) as u8
                    }
                }
                "photo" => {
                    ((x / (3 + channel)
                        + y / (2 + channel)
                        + x * y / (width + 1)
                        + (random % 9) as usize)
                        % 256) as u8
                }
                "gradient" => ((x / (channel + 1) + y / 2) % 256) as u8,
                "noise" => random as u8,
                _ => 192,
            };
        }
        pixel[3] = 255;
    }
    plain
}
#[test]
fn correlated_images_select_smaller_cm03() {
    for kind in ["screenshot", "photo", "gradient"] {
        let plain = sample(kind, 640, 480);
        let old = crypto::compress_payload(&plain).unwrap().unwrap();
        let selected = crypto::select_payload(&plain).unwrap();
        let CompressedPayload::Predicted(new) = selected else {
            panic!("{kind} 未选择 CM03")
        };
        assert!(
            new.len() * 100 < old.len() * 85,
            "{kind}: {} -> {}",
            old.len(),
            new.len()
        );
        let decoded = zstd::bulk::decompress(&new, plain.len() + 1).unwrap();
        assert_eq!(predict::decode(decoded, plain.len()).unwrap(), plain);
    }
}
#[test]
fn selection_is_deterministic_and_never_grows_baseline() {
    for kind in ["solid", "noise", "photo"] {
        let plain = sample(kind, 96, 65);
        let first = crypto::select_payload(&plain).unwrap();
        assert_eq!(first, crypto::select_payload(&plain).unwrap());
        let baseline = crypto::compress_payload(&plain).unwrap();
        match first {
            CompressedPayload::Predicted(new) => {
                assert!(baseline.is_none_or(|old| new.len() < old.len()))
            }
            CompressedPayload::Zstd(new) => assert_eq!(Some(new), baseline),
            CompressedPayload::Raw => assert!(baseline.is_none()),
        }
    }
}
#[test]
fn three_image_classes_roundtrip_and_optional_benchmark() {
    let bench = std::env::var_os("CLIPBOARD_PREDICT_BENCH").is_some();
    let (width, height, repeats) = if bench { (3840, 2048, 7) } else { (96, 64, 1) };
    for kind in ["screenshot", "photo", "noise"] {
        let plain = sample(kind, width, height);
        let old = crypto::compress_payload(&plain).unwrap().unwrap();
        let predicted = predict::encode(&plain).unwrap();
        let new = zstd::bulk::compress(&predicted, 3).unwrap();
        let blob = crypto::encrypt_blob(&[81; 32], "bench", &plain).unwrap();
        println!(
            "predict_size kind={kind} raw={} cm02={} cm03={} selected={}",
            plain.len(),
            old.len() + 52,
            new.len() + 52,
            String::from_utf8_lossy(&blob[..4])
        );
        for run in 0..repeats {
            let now = Instant::now();
            let compressed = crypto::compress_payload(&plain).unwrap().unwrap();
            let old_encode = now.elapsed().as_secs_f64() * 1000.0;
            std::hint::black_box(compressed);
            let now = Instant::now();
            let decoded = zstd::bulk::decompress(&old, plain.len()).unwrap();
            let old_decode = now.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(decoded, plain);
            drop(decoded);
            let now = Instant::now();
            let filtered = predict::encode(&plain).unwrap();
            let compressed = zstd::bulk::compress(&filtered, 3).unwrap();
            let new_encode = now.elapsed().as_secs_f64() * 1000.0;
            drop(filtered);
            std::hint::black_box(compressed);
            let now = Instant::now();
            let decoded = zstd::bulk::decompress(&new, plain.len() + 1).unwrap();
            let decoded = predict::decode(decoded, plain.len()).unwrap();
            let new_decode = now.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(decoded, plain);
            drop(decoded);
            let now = Instant::now();
            let selected = crypto::select_payload(&plain).unwrap();
            let selection = now.elapsed().as_secs_f64() * 1000.0;
            std::hint::black_box(selected);
            let now = Instant::now();
            let decoded = crypto::decrypt(&[81; 32], "bench", &blob).unwrap();
            let read = now.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(decoded, plain);
            println!("predict_time kind={kind} run={run} cm02_encode={old_encode:.3} cm02_decode={old_decode:.3} cm03_encode={new_encode:.3} cm03_decode={new_decode:.3} selection={selection:.3} authenticated_read={read:.3}");
        }
    }
}

#[test]
fn mutated_headers_and_random_inputs_never_panic_or_change_roundtrip() {
    let original = fixture(7, -5, 32, 124, true);
    for offset in 0..124 {
        for value in [0, 1, 127, 255] {
            let mut changed = original.clone();
            changed[offset] = value;
            if let Some(encoded) = predict::encode(&changed) {
                assert_eq!(predict::decode(encoded, changed.len()).unwrap(), changed);
            }
        }
    }
    for length in 0..256 {
        let data = vec![length as u8; length];
        assert!(predict::encode(&data).is_none());
        assert!(predict::decode(data, length.saturating_sub(1)).is_err());
    }
}
#[test]
fn uniform_screenshot_keeps_smaller_baseline_when_prediction_cannot_help() {
    let plain = sample("solid", 640, 480);
    let baseline = crypto::compress_payload(&plain).unwrap().unwrap();
    let candidate = zstd::bulk::compress(&predict::encode(&plain).unwrap(), 3).unwrap();
    let selected = crypto::select_payload(&plain).unwrap();
    let size = match selected {
        CompressedPayload::Zstd(ref bytes) | CompressedPayload::Predicted(ref bytes) => bytes.len(),
        CompressedPayload::Raw => panic!("纯色位图应当可压缩"),
    };
    assert_eq!(size, baseline.len().min(candidate.len()));
    assert!(size < plain.len() / 100);
}
