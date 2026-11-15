use super::*;
use crate::model::FormatData;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{fs, path::PathBuf};
fn clip(html: &str) -> Clip {
    Clip {
        formats: vec![
            FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: "第一段\n第二段"
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            },
            FormatData {
                name: "HTML Format".into(),
                bytes: html.as_bytes().to_vec(),
            },
        ],
    }
}
fn png() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(32, 20, image::Rgb([20, 130, 110]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}
fn embedded() -> String {
    format!("data:image/png;base64,{}", STANDARD.encode(png()))
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "b10-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn source(&self) -> String {
        let path = self.0.join("测试 图片.png");
        fs::write(&path, png()).unwrap();
        tauri::Url::from_file_path(std::path::absolute(path).unwrap())
            .unwrap()
            .to_string()
    }
}
#[test]
fn cf_html_byte_offsets_support_chinese_and_reject_invalid_boundaries() {
    let fragment = "标题<img src='data:image/png;base64,AA=='>之后";
    let template = "Version:1.0\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let prefix = "<html><body><!--StartFragment-->";
    let suffix = "<!--EndFragment--></body></html>";
    let start = template.len() + prefix.len();
    let end = start + fragment.len();
    let header = format!("Version:1.0\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{start:010}\r\nEndFragment:{end:010}\r\n", template.len(), end + suffix.len());
    let payload = format!("{header}{prefix}{fragment}{suffix}");
    assert_eq!(html::fragment(payload.as_bytes()), Some(fragment));
    let invalid = payload.replace(
        &format!("StartFragment:{start:010}"),
        &format!("StartFragment:{:010}", start + 1),
    );
    assert!(html::fragment(invalid.as_bytes()).is_none());
    assert!(html::fragment(b"StartFragment:99999999\r\nEndFragment:999999999\r\n<p>a").is_none());
}
#[test]
fn interleaved_text_images_entities_and_blocks_keep_order() {
    let html = format!(
        "<div>甲 &amp; <b>乙</b></div><img src='{}'><p>丙<br>丁</p><img src='{}'>",
        embedded(),
        embedded()
    );
    let mut preview = from_clip(&clip(&html)).unwrap();
    preview.resolve_images();
    assert_eq!(preview.blocks.len(), 4);
    assert!(matches!(&preview.blocks[0], Block::Text { text } if text=="甲 & 乙"));
    assert!(
        matches!(&preview.blocks[1], Block::Image { src: Some(src), pending: None, .. } if src.starts_with("data:image/"))
    );
    assert!(matches!(&preview.blocks[2], Block::Text { text } if text=="丙\n丁"));
    assert!(matches!(
        &preview.blocks[3],
        Block::Image { src: Some(_), .. }
    ));
}
#[test]
fn local_file_preview_reencodes_and_never_serializes_source_path() {
    let f = Fixture::new();
    let source = f.source();
    let mut preview = from_clip(&clip(&format!("前<img src='{source}'>后"))).unwrap();
    let pending = serde_json::to_string(&preview).unwrap();
    assert!(!pending.contains("file:") && !pending.contains("测试"));
    preview.resolve_images();
    assert!(preview.linked_images);
    assert!(matches!(
        &preview.blocks[1],
        Block::Image {
            src: Some(_),
            message: None,
            pending: None
        }
    ));
    let json = serde_json::to_string(&preview).unwrap();
    assert!(!json.contains("file:") && !json.contains("测试"));
}
#[test]
fn scripts_svg_objects_and_remote_urls_never_execute_or_load() {
    let mut preview = from_clip(&clip("<script>hidden()</script><style>hidden</style><iframe src='https://invalid'>hidden</iframe><svg><image href='file:///C:/synthetic.png'></svg><p hidden>hidden</p>safe<img onerror='run()' src='https://invalid/p.png'><img src='javascript:run()'><img src='file://server/share/image.png'>")).unwrap();
    preview.resolve_images();
    let json = serde_json::to_string(&preview).unwrap();
    assert!(!json.contains("hidden") && !json.contains("run()") && !json.contains("server/share"));
    assert_eq!(preview.blocks.len(), 4);
    assert!(preview.blocks.iter().skip(1).all(|b| matches!(
        b,
        Block::Image {
            src: None,
            message: Some(_),
            pending: None
        }
    )));
}
#[test]
fn missing_or_invalid_local_image_preserves_text_and_reports_placeholder() {
    let f = Fixture::new();
    let source = tauri::Url::from_file_path(std::path::absolute(f.0.join("missing.png")).unwrap())
        .unwrap()
        .to_string();
    let mut preview = from_clip(&clip(&format!("前<img src='{source}'>后"))).unwrap();
    preview.resolve_images();
    assert!(
        matches!(&preview.blocks[1], Block::Image { src: None, message: Some(message), .. } if message.contains("失效"))
    );
    fs::write(f.0.join("missing.png"), b"not a picture").unwrap();
    let mut preview = from_clip(&clip(&format!("<img src='{source}'>"))).unwrap();
    preview.resolve_images();
    assert!(matches!(
        &preview.blocks[0],
        Block::Image {
            src: None,
            message: Some(_),
            ..
        }
    ));
}

#[test]
fn limits_bound_html_text_images_and_nesting() {
    let oversized = format!("{}<img>", "x".repeat(MAX_HTML_BYTES));
    assert!(from_clip(&clip(&oversized)).unwrap().truncated);
    let text = format!(
        "{}<img src='{}'>",
        "文".repeat(MAX_TEXT_CHARS + 1),
        embedded()
    );
    let preview = from_clip(&clip(&text)).unwrap();
    assert!(preview.truncated);
    assert!(
        matches!(&preview.blocks[0], Block::Text { text } if text.chars().count()==MAX_TEXT_CHARS)
    );
    let images = "<img src='https://invalid/img'>".repeat(MAX_IMAGES + 2);
    let preview = from_clip(&clip(&images)).unwrap();
    assert!(preview.truncated);
    assert_eq!(preview.blocks.len(), MAX_IMAGES);
    let nested = format!("<img>{}x{}", "<div>".repeat(1000), "</div>".repeat(1000));
    assert!(from_clip(&clip(&nested)).unwrap().truncated);
}
#[test]
fn malformed_fragment_falls_back_and_plain_html_keeps_existing_text_preview() {
    let preview = from_clip(&clip(
        "StartFragment:1000000000\r\nEndFragment:1000000001\r\n<p>broken",
    ))
    .unwrap();
    assert!(preview.truncated);
    assert!(from_clip(&clip("<b>普通富文本无图片</b>")).is_none());
    assert!(html::fragment(&[0xff, 0xfe, 0x31]).is_none());
}
#[test]
fn unsupported_embedded_formats_and_file_extensions_are_rejected() {
    let mut preview = from_clip(&clip("<img src='data:image/svg+xml;base64,PHN2Zz4='><img src='data:image/png;base64,@@@'><img src='file:///C:/synthetic.txt'><img src='relative.jpg'>")).unwrap();
    preview.resolve_images();
    assert!(preview.blocks.iter().all(|b| matches!(
        b,
        Block::Image {
            src: None,
            message: Some(_),
            pending: None
        }
    )));
}
#[test]
fn html_image_preview_does_not_change_original_paste_payloads() {
    let f = Fixture::new();
    let record = clip(&format!("甲<img src='{}'>乙", f.source()));
    let before = record.hash();
    let formats = record.formats.clone();
    from_clip(&record).unwrap().resolve_images();
    assert_eq!(record.formats, formats);
    assert_eq!(record.hash(), before);
}
#[test]
fn unsafe_device_or_remote_paths_never_read() {
    for source in [
        "file://remote/share/a.png",
        "file:///C:/a.png?query",
        "file:///C:/a.png#fragment",
        "https://invalid/a.png",
        "blob:https://invalid/1",
    ] {
        assert!(images::load(source, &mut images::Budget::default()).is_err());
    }
}
#[test]
#[ignore = "需要显式提供独立的真实记录格式副本，仅输出结构统计"]
fn isolated_real_html_fixture() {
    let root = PathBuf::from(std::env::var_os("B10_FIXTURE_DIR").expect("需指定隔离格式目录"));
    let manifests: Vec<serde_json::Value> =
        serde_json::from_slice(&fs::read(root.join("formats.json")).unwrap()).unwrap();
    let formats = manifests
        .iter()
        .map(|entry| FormatData {
            name: entry["name"].as_str().unwrap().into(),
            bytes: fs::read(root.join(entry["file"].as_str().unwrap())).unwrap(),
        })
        .collect();
    let record = Clip { formats };
    let original = record.hash();
    let mut preview = from_clip(&record).expect("样本包含图文");
    preview.resolve_images();
    let images = preview
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Image { src: Some(_), .. }))
        .count();
    let text = preview
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Text { .. }))
        .count();
    let failures = preview
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Image { src: None, .. }))
        .count();
    fs::write(
        root.join("resolved-preview.json"),
        serde_json::to_vec(&preview).unwrap(),
    )
    .unwrap();
    assert_eq!(record.hash(), original);
    assert!(images > 0);
    assert_eq!(failures, 0);
    println!("text_blocks={text} images={images} placeholders={failures} payload_unchanged=true");
}
