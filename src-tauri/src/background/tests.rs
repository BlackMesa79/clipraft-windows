use super::*;
use crate::storage::{unix_millis, Storage};
use image::{Rgb, RgbImage};
use std::{fs, path::PathBuf};

const MASTER: [u8; 32] = [91; 32];

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "background-{}-{}",
                std::process::id(),
                unix_millis()
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("背景测试目录清理失败：{error}");
        }
    }
}

fn png(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb(color)))
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    encoded.into_inner()
}

#[test]
fn background_roundtrips_after_source_removal_and_in_portable_snapshot() {
    use BackgroundLayout::{Compact, Regular};
    let fixture = Fixture::new();
    let source = fixture.0.join("source.png");
    let data = fixture.0.join("data");
    fs::write(&source, png(2200, 180, [90, 40, 20])).unwrap();
    let store = Storage::open(&data, &MASTER).unwrap();
    assert!(store.load_main_background(Regular).unwrap().is_none());
    let initial = import_file(&source).unwrap();
    let image = image::load_from_memory(&STANDARD.decode(&initial.jpeg).unwrap()).unwrap();
    assert_eq!(image.width(), MAX_DISPLAY_SIDE);
    store.save_main_background(Regular, &initial).unwrap();
    assert!(store
        .test_settings_rows()
        .contains_key("main_background_asset"));
    assert!(store.load_main_background(Compact).unwrap().is_none());
    fs::remove_file(source).unwrap();
    let options: BackgroundOptions =
        serde_json::from_str(r#"{"fit":"contain","position":"bottom","scrim":69}"#).unwrap();
    store
        .update_main_background_options(Regular, options)
        .unwrap();
    let portrait = MainBackground {
        jpeg: STANDARD.encode(normalize(&png(3, 9, [30, 43, 57])).unwrap()),
        options: BackgroundOptions::default(),
    };
    store.save_main_background(Compact, &portrait).unwrap();
    drop(store);

    let mut store = Storage::open(&data, &MASTER).unwrap();
    let restored = store.load_main_background(Regular).unwrap().unwrap();
    assert_eq!(restored.jpeg, initial.jpeg);
    assert_eq!(restored.options, options);
    let invalid =
        serde_json::from_str(r#"{"fit":"contain","position":"bottom","scrim":30}"#).unwrap();
    assert!(store
        .update_main_background_options(Regular, invalid)
        .is_err());
    assert_eq!(
        store
            .load_main_background(Regular)
            .unwrap()
            .unwrap()
            .options,
        options
    );
    assert_eq!(
        store.load_settings().unwrap(),
        crate::settings::Settings::default()
    );

    let snapshot = fixture.0.join("snapshot");
    store.create_portable_snapshot(&snapshot, &MASTER).unwrap();
    let snapshot = Storage::open(&snapshot, &MASTER).unwrap();
    let copied = snapshot.load_main_background(Regular).unwrap().unwrap();
    assert_eq!(copied.jpeg, initial.jpeg);
    assert_eq!(copied.options, options);
    let copied_portrait = snapshot.load_main_background(Compact).unwrap().unwrap();
    assert_eq!(copied_portrait.jpeg, portrait.jpeg);
    assert_eq!(copied_portrait.options, portrait.options);

    let replacement = MainBackground {
        jpeg: STANDARD.encode(normalize(&png(2, 2, [2, 4, 9])).unwrap()),
        options,
    };
    store.save_main_background(Regular, &replacement).unwrap();
    assert_eq!(
        store.load_main_background(Regular).unwrap().unwrap().jpeg,
        replacement.jpeg
    );
    store.remove_main_background(Regular).unwrap();
    assert!(store.load_main_background(Regular).unwrap().is_none());
    let remaining = store.load_main_background(Compact).unwrap().unwrap();
    assert_eq!(remaining.jpeg, portrait.jpeg);
    assert_eq!(remaining.options, portrait.options);
}

#[test]
fn invalid_and_oversized_images_are_rejected() {
    assert!(normalize(b"not an image").is_err());
    assert!(normalize(b"\x89PNG\r\n\x1a\ninvalid").is_err());
    assert!(normalize(&png(MAX_SOURCE_SIDE + 1, 1, [0, 0, 0])).is_err());
}
