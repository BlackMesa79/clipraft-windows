//! 显式运行的大图基准；使用私有窗口站的剪贴板，不改变用户桌面剪贴板。
use crate::test_support::PrivateClipboard;
use crate::{
    clipboard,
    model::{Clip, FormatData},
    storage::Storage,
};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ImageEncoder,
};
use std::{
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::Foundation::HWND;

fn fixture() -> Clip {
    let (width, height) = (3200u32, 3200u32);
    let mut random = 0x1234_5678u32;
    let rgb: Vec<u8> = (0..width as usize * height as usize * 3)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            random as u8
        })
        .collect();
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::NoFilter)
        .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .unwrap();
    let mut dib = vec![0u8; 40];
    dib[0..4].copy_from_slice(&40u32.to_le_bytes());
    dib[4..8].copy_from_slice(&width.to_le_bytes());
    dib[8..12].copy_from_slice(&(-(height as i32)).to_le_bytes());
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&24u16.to_le_bytes());
    dib[20..24].copy_from_slice(&(width * height * 3).to_le_bytes());
    let mut v5 = vec![0u8; 124];
    v5[..40].copy_from_slice(&dib);
    v5[..4].copy_from_slice(&124u32.to_le_bytes());
    v5[14..16].copy_from_slice(&32u16.to_le_bytes());
    v5[16..20].copy_from_slice(&3u32.to_le_bytes());
    v5[20..24].copy_from_slice(&(width * height * 4).to_le_bytes());
    for (offset, mask) in [
        (40, 0x00ff0000u32),
        (44, 0x0000ff00),
        (48, 0x000000ff),
        (52, 0xff000000),
        (56, 0x73524742),
    ] {
        v5[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
    }
    v5[108..112].copy_from_slice(&4u32.to_le_bytes());
    dib.reserve(rgb.len());
    v5.reserve(width as usize * height as usize * 4);
    for pixel in rgb.as_chunks::<3>().0 {
        dib.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        v5.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }
    Clip {
        formats: vec![
            FormatData {
                name: "PNG".into(),
                bytes: png,
            },
            FormatData {
                name: "CF_DIBV5".into(),
                bytes: v5,
            },
            FormatData {
                name: "CF_DIB".into(),
                bytes: dib,
            },
        ],
    }
}
#[test]
#[ignore = "仅显式单独运行：合成大图性能测量，使用私有窗口站"]
fn large_image_benchmark() {
    let _isolated = PrivateClipboard::new();
    clipboard::initialize().unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.test-data")
        .join(format!(
            "paste-bench-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    let mut storage = Storage::open(&root, &[73; 32]).unwrap();
    let clip = fixture();
    for format in &clip.formats {
        println!("format={} bytes={}", format.name, format.bytes.len());
    }
    let id = clip.hash();
    storage.save(&clip).unwrap();
    drop(clip);
    let mut csv = String::from("sample,load_ms,write_ms,total_ms,promote_ms\n");
    for sample in 0..6 {
        let start = Instant::now();
        let clip = storage.load(&id).unwrap();
        let loaded = Instant::now();
        clipboard::write(HWND::default(), &clip, false).unwrap();
        let written = Instant::now();
        storage.touch(&id).unwrap();
        let end = Instant::now();
        let line = format!(
            "{sample},{:.3},{:.3},{:.3},{:.3}\n",
            (loaded - start).as_secs_f64() * 1000.0,
            (written - loaded).as_secs_f64() * 1000.0,
            (written - start).as_secs_f64() * 1000.0,
            (end - written).as_secs_f64() * 1000.0
        );
        print!("{line}");
        csv.push_str(&line);
    }
    // 计时外核验系统确实能提供 DIB，且读取合成格式后仍能识别自写回。
    unsafe {
        use windows::Win32::System::DataExchange::*;
        OpenClipboard(None).unwrap();
        let dib = GetClipboardData(8);
        CloseClipboard().unwrap();
        assert!(dib.is_ok(), "系统应从 DIBV5 提供兼容 DIB");
    }
    assert!(clipboard::capture(HWND::default()).unwrap().is_none());
    std::fs::write(root.join("timings.csv"), csv).unwrap();
    println!("benchmark_root={}", root.display());
}
