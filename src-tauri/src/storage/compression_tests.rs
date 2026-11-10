//! 全部数据位于项目内的独立目录，不读取真实剪贴板或用户数据。
use super::*;
use crate::model::FormatData;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

struct TestRoot(PathBuf);
impl TestRoot {
    fn new(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "blob-{name}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn text_blob(value: u8) -> Clip {
    Clip {
        formats: vec![FormatData {
            name: "HTML Format".into(),
            bytes: vec![value; 70000],
        }],
    }
}
fn dibv5(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; 124 + (width * height * 4) as usize];
    for (offset, value) in [
        (0, 124),
        (4, width),
        (8, height),
        (16, 3),
        (20, width * height * 4),
        (40, 0xff0000),
        (44, 0xff00),
        (48, 0xff),
        (52, 0xff000000),
        (56, 0x73524742),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[12] = 1;
    bytes[14] = 32;
    // 渐变背景、重复窗口色块、细线与透明像素；并非全零缓冲区。
    for (index, pixel) in bytes[124..].as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (index as u32 % width, index as u32 / width);
        pixel.copy_from_slice(&[
            (x / 16 % 256) as u8,
            (y / 8 % 256) as u8,
            if x % 160 < 2 || y % 80 < 2 { 32 } else { 192 },
            if x < width / 8 { 128 } else { 255 },
        ]);
    }
    bytes
}
#[test]
fn old_cm01_blob_is_read_and_never_rewritten() {
    let root = TestRoot::new("old");
    let mut store = Storage::open(&root.0, &[41; 32]).unwrap();
    let clip = text_blob(65);
    let id = model::digest(&clip.formats[0].bytes);
    let legacy = crypto::encrypt(&store.key, &id, &clip.formats[0].bytes).unwrap();
    fs::write(store.blobs.join(&id), &legacy).unwrap();
    store.save(&clip).unwrap();
    assert_eq!(fs::read(store.blobs.join(&id)).unwrap(), legacy);
    assert_eq!(store.load(&clip.hash()).unwrap().formats, clip.formats);
    drop(store);
    let store = Storage::open(&root.0, &[41; 32]).unwrap();
    assert_eq!(store.load(&clip.hash()).unwrap().formats, clip.formats);
    assert_eq!(store.list().unwrap()[0].size, 70000);
}
#[test]
fn cm02_delete_and_orphan_scan_preserve_referenced_objects() {
    let root = TestRoot::new("maintenance");
    let mut store = Storage::open(&root.0, &[42; 32]).unwrap();
    let clip = text_blob(66);
    store.save(&clip).unwrap();
    let blob = model::digest(&clip.formats[0].bytes);
    assert_eq!(&fs::read(store.blobs.join(&blob)).unwrap()[..4], b"CM02");
    let orphan = store.write_blob(&vec![67; 70000]).unwrap();
    assert_eq!(&fs::read(store.blobs.join(&orphan)).unwrap()[..4], b"CM02");
    assert_eq!(store.scan_orphan_blobs().unwrap().reclaimed_blobs, 1);
    assert!(!store.blobs.join(&orphan).exists());
    assert_eq!(store.load(&clip.hash()).unwrap().formats, clip.formats);
    let report = store.delete(&[clip.hash()]).unwrap();
    assert_eq!(report.deleted_items, 1);
    assert_eq!(report.reclaimed_blobs, 1);
    assert_eq!(fs::read_dir(&store.blobs).unwrap().count(), 0);
}
#[test]
fn shared_save_deduplicates_before_preparing_any_objects() {
    let root = TestRoot::new("shared");
    let storage = Mutex::new(Storage::open(&root.0, &[43; 32]).unwrap());
    let clip = text_blob(68);
    assert!(Storage::save_shared(&storage, &clip).unwrap());
    let before: Vec<_> = fs::read_dir(root.0.join("data/blobs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let mut extra = clip.clone();
    // 文件去重规则已有完整回归；这里验证普通记录走同一提前命中入口。
    extra.formats.reverse();
    assert!(!Storage::save_shared(&storage, &extra).unwrap());
    assert_eq!(storage.lock().unwrap().list().unwrap().len(), 1);
    assert_eq!(
        fs::read_dir(root.0.join("data/blobs")).unwrap().count(),
        before.len()
    );
}
#[test]
fn prepared_objects_are_memory_only_and_commit_rechecks_duplicates() {
    let root = TestRoot::new("race");
    let mut store = Storage::open(&root.0, &[44; 32]).unwrap();
    let clip = text_blob(69);
    let prepared = PreparedSave::new(&clip, SaveIdentity::new(&clip), &store.key).unwrap();
    assert_eq!(fs::read_dir(&store.blobs).unwrap().count(), 0);
    assert_eq!(store.scan_orphan_blobs().unwrap().reclaimed_blobs, 0);
    assert!(store.save_with_outcome(&clip).unwrap());
    let blob = model::digest(&clip.formats[0].bytes);
    let original = fs::read(store.blobs.join(&blob)).unwrap();
    assert!(!store.commit_prepared(prepared).unwrap());
    assert_eq!(fs::read(store.blobs.join(&blob)).unwrap(), original);
    assert_eq!(store.list().unwrap().len(), 1);
}
#[test]
fn prepared_save_survives_reclamation_during_preparation() {
    let root = TestRoot::new("reclaim");
    let mut store = Storage::open(&root.0, &[45; 32]).unwrap();
    let clip = text_blob(70);
    store.save(&clip).unwrap();
    let prepared = PreparedSave::new(&clip, SaveIdentity::new(&clip), &store.key).unwrap();
    assert_eq!(store.delete(&[clip.hash()]).unwrap().reclaimed_blobs, 1);
    assert!(store.commit_prepared(prepared).unwrap());
    assert_eq!(store.load(&clip.hash()).unwrap().formats, clip.formats);
}
#[test]
fn png_original_thumbnail_and_preview_stay_cm01() {
    let root = TestRoot::new("png");
    let mut store = Storage::open(&root.0, &[46; 32]).unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        300,
        200,
        image::Rgba([1, 2, 3, 128]),
    ))
    .write_to(&mut png, image::ImageFormat::Png)
    .unwrap();
    let clip = Clip {
        formats: vec![FormatData {
            name: "PNG".into(),
            bytes: png.into_inner(),
        }],
    };
    store.save(&clip).unwrap();
    for entry in fs::read_dir(&store.blobs).unwrap() {
        assert_eq!(&fs::read(entry.unwrap().path()).unwrap()[..4], b"CM01");
    }
    let view = store.list().unwrap().remove(0);
    for id in [view.thumbnail.unwrap(), view.preview.unwrap()] {
        let decoded = image::load_from_memory(&store.read_thumbnail(&id).unwrap())
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [1, 2, 3, 128]);
    }
}
#[test]
fn large_dib_cm02_roundtrip_and_timings() {
    let root = TestRoot::new("large");
    let storage = Mutex::new(Storage::open(&root.0, &[47; 32]).unwrap());
    let clip = Clip {
        formats: vec![FormatData {
            name: "CF_DIBV5".into(),
            bytes: dibv5(3840, 2048),
        }],
    };
    let plain = &clip.formats[0].bytes;
    let id = clip.hash();
    let blob = model::digest(plain);
    let start = Instant::now();
    assert!(Storage::save_shared(&storage, &clip).unwrap());
    let save_ms = start.elapsed().as_secs_f64() * 1000.0;
    let store = storage.lock().unwrap();
    let bytes = fs::read(store.blobs.join(&blob)).unwrap();
    assert!(matches!(&bytes[..4], b"CM02" | b"CM03"));
    assert!(bytes.len() < plain.len() / 2);
    assert_eq!(store.list().unwrap()[0].size, plain.len() as i64);
    let view = store.list().unwrap().remove(0);
    for id in [view.thumbnail.unwrap(), view.preview.unwrap()] {
        assert!(store
            .read_thumbnail(&id)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
    }
    let benchmark = std::env::var_os("CLIPBOARD_BLOB_BENCH").is_some();
    let samples = if benchmark { 7 } else { 1 };
    println!(
        "blob_bench plain_bytes={} blob_bytes={} ratio={:.6} save_ms={save_ms:.3}",
        plain.len(),
        bytes.len(),
        bytes.len() as f64 / plain.len() as f64
    );
    for sample in 0..samples {
        let start = Instant::now();
        let compressed = crypto::compress_payload(plain).unwrap().unwrap();
        let compress_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let decompressed = zstd::bulk::decompress(&compressed, plain.len()).unwrap();
        let decompress_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(&decompressed, plain);
        drop(decompressed);
        let start = Instant::now();
        let loaded = store.load(&id).unwrap();
        let load_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(loaded.formats, clip.formats);
        println!("blob_bench sample={sample} compress_ms={compress_ms:.3} decompress_ms={decompress_ms:.3} load_ms={load_ms:.3}");
    }
    drop(store);
    drop(storage);
    drop(clip);
    if benchmark {
        benchmark_dense_dib();
    }
}

fn benchmark_dense_dib() {
    let mut plain = dibv5(3840, 2048);
    let mut state = 0x12345678u32;
    for pixel in plain[124..].as_chunks_mut::<4>().0 {
        for component in &mut pixel[..3] {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *component = state as u8;
        }
        pixel[3] = 255;
    }
    let id = model::digest(&plain);
    let bytes = crypto::encrypt_blob(&[48; 32], &id, &plain).unwrap();
    let compressed = crypto::compress_payload(&plain).unwrap().unwrap();
    println!(
        "dense_bench plain_bytes={} blob_bytes={} ratio={:.6}",
        plain.len(),
        bytes.len(),
        bytes.len() as f64 / plain.len() as f64
    );
    for sample in 0..7 {
        let start = Instant::now();
        let attempt = crypto::compress_payload(&plain).unwrap().unwrap();
        let compress_ms = start.elapsed().as_secs_f64() * 1000.0;
        drop(attempt);
        let start = Instant::now();
        let decompressed = zstd::bulk::decompress(&compressed, plain.len()).unwrap();
        let decompress_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(decompressed, plain);
        drop(decompressed);
        let start = Instant::now();
        let decoded = crypto::decrypt(&[48; 32], &id, &bytes).unwrap();
        let decrypt_decompress_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(decoded, plain);
        println!("dense_bench sample={sample} compress_ms={compress_ms:.3} decompress_ms={decompress_ms:.3} decrypt_decompress_ms={decrypt_decompress_ms:.3}");
    }
}

#[test]
fn shared_concurrent_saves_keep_one_row_and_one_blob() {
    let root = TestRoot::new("concurrent");
    let storage = std::sync::Arc::new(Mutex::new(Storage::open(&root.0, &[49; 32]).unwrap()));
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let storage = storage.clone();
            let gate = gate.clone();
            std::thread::spawn(move || {
                gate.wait();
                Storage::save_shared(&storage, &text_blob(71)).unwrap()
            })
        })
        .collect();
    let inserted = handles
        .into_iter()
        .map(|h| usize::from(h.join().unwrap()))
        .sum::<usize>();
    assert_eq!(inserted, 1);
    let store = storage.lock().unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(fs::read_dir(&store.blobs).unwrap().count(), 1);
    assert_eq!(store.scan_orphan_blobs().unwrap().reclaimed_blobs, 0);
    assert_eq!(
        store.load(&text_blob(71).hash()).unwrap().formats,
        text_blob(71).formats
    );
}

#[test]
fn cm03_delete_and_orphan_scan_preserve_referenced_objects() {
    let root = TestRoot::new("cm03-maintenance");
    let mut store = Storage::open(&root.0, &[83; 32]).unwrap();
    let clip = Clip {
        formats: vec![FormatData {
            name: "CF_DIBV5".into(),
            bytes: super::prediction_tests::sample("photo", 256, 128),
        }],
    };
    store.save(&clip).unwrap();
    let hash = model::digest(&clip.formats[0].bytes);
    assert_eq!(&fs::read(store.blobs.join(&hash)).unwrap()[..4], b"CM03");
    let orphan = store
        .write_blob(&super::prediction_tests::sample("photo", 128, 128))
        .unwrap();
    assert_eq!(&fs::read(store.blobs.join(&orphan)).unwrap()[..4], b"CM03");
    assert_eq!(store.scan_orphan_blobs().unwrap().reclaimed_blobs, 1);
    assert!(!store.blobs.join(&orphan).exists());
    assert_eq!(store.load(&clip.hash()).unwrap().formats, clip.formats);
    let report = store.delete(&[clip.hash()]).unwrap();
    assert_eq!(report.deleted_items, 1);
    assert!(report.failed_blobs.is_empty());
    assert_eq!(fs::read_dir(&store.blobs).unwrap().count(), 0);
}
#[test]
fn legacy_png_thumbnail_is_read_without_conversion() {
    let root = TestRoot::new("legacy-png");
    let mut store = Storage::open(&root.0, &[84; 32]).unwrap();
    let clip = text_blob(82);
    store.save(&clip).unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        7,
        9,
        image::Rgba([20, 40, 60, 255]),
    ))
    .write_to(&mut png, image::ImageFormat::Png)
    .unwrap();
    let original = png.into_inner();
    let hash = model::digest(&original);
    let old = crypto::encrypt(&store.key, &hash, &original).unwrap();
    fs::write(store.blobs.join(&hash), &old).unwrap();
    store
        .conn
        .execute(
            "UPDATE items SET thumbnail=?1,preview=?1 WHERE id=?2",
            rusqlite::params![hash, clip.hash()],
        )
        .unwrap();
    assert_eq!(store.read_thumbnail(&hash).unwrap(), original);
    assert_eq!(
        crate::thumbnail_response(Some(store.read_thumbnail(&hash).unwrap())).headers()
            ["Content-Type"],
        "image/png"
    );
    assert_eq!(image::load_from_memory(&original).unwrap().width(), 7);
    assert_eq!(fs::read(store.blobs.join(&hash)).unwrap(), old);
}

#[test]
fn jpeg_previews_are_registered_and_raw_image_is_not_protocol_accessible() {
    let root = TestRoot::new("jpeg-access");
    let mut store = Storage::open(&root.0, &[85; 32]).unwrap();
    let clip = Clip {
        formats: vec![FormatData {
            name: "CF_DIBV5".into(),
            bytes: super::prediction_tests::sample("photo", 800, 600),
        }],
    };
    store.save(&clip).unwrap();
    assert!(store
        .read_thumbnail(&model::digest(&clip.formats[0].bytes))
        .is_err());
    let view = store.list().unwrap().remove(0);
    for id in [view.thumbnail.unwrap(), view.preview.unwrap()] {
        let bytes = store.read_thumbnail(&id).unwrap();
        assert_eq!(thumbnail::preview_mime(&bytes), Some("image/jpeg"));
        assert_eq!(&fs::read(store.blobs.join(&id)).unwrap()[..4], b"CM01");
        assert_eq!(
            crate::thumbnail_response(Some(bytes)).headers()["Content-Type"],
            "image/jpeg"
        );
    }
}
