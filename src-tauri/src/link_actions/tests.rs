use super::*;
use crate::model::{Clip, FormatData};
use std::path::PathBuf;

struct Fixture {
    storage: Option<Storage>,
    root: PathBuf,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!("link-actions-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let storage = Storage::open(&root, &[93; 32]).unwrap();
        Self {
            storage: Some(storage),
            root,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.storage.take());
        std::fs::remove_dir_all(&self.root).unwrap();
    }
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
fn complete_saved_url_is_used_and_link_flag_reaches_all_views() {
    let mut fixture = Fixture::new("complete");
    let storage = fixture.storage.as_mut().unwrap();
    let value = format!(" HTTPS://例子.测试/你好?query={}#section ", "a".repeat(400));
    let clip = text(&value);
    storage.save(&clip).unwrap();
    let id = clip.hash();
    let expected = tauri::Url::parse(value.trim()).unwrap();
    assert_eq!(target(storage, &id).unwrap(), expected);
    assert!(storage.preview_item(&id).unwrap().is_link);
    assert!(storage.list().unwrap()[0].is_link);
    assert!(storage.panel_items(&[&id]).unwrap()[0].is_link);
    assert!(target(storage, "missing").is_err());
}

#[test]
fn unsafe_schemes_non_links_and_rich_records_never_open() {
    let mut fixture = Fixture::new("schemes");
    let storage = fixture.storage.as_mut().unwrap();
    for value in [
        "file:///C:/test.exe",
        "javascript:alert(1)",
        "mailto:a@example.com",
        "ms-settings:privacy",
        "https://",
        "https://example.com/ argument",
        "plain text",
    ] {
        let clip = text(value);
        storage.save(&clip).unwrap();
        assert!(target(storage, &clip.hash()).is_err(), "{value}");
        assert!(!storage.preview_item(&clip.hash()).unwrap().is_link);
    }
    let mut rich = Clip::default();
    rich.formats.push(FormatData {
        name: "HTML Format".into(),
        bytes: b"<p>url</p>".to_vec(),
    });
    storage.save(&rich).unwrap();
    assert!(target(storage, &rich.hash()).is_err());
    let huge = text(&format!("https://example.com/{}", "x".repeat(8192)));
    storage.save(&huge).unwrap();
    assert!(target(storage, &huge.hash()).is_err());
}

#[test]
fn privacy_expiry_and_stale_link_flags_cannot_bypass_validation() {
    let mut fixture = Fixture::new("privacy");
    let connection = rusqlite::Connection::open(fixture.root.join("history.db")).unwrap();
    connection
        .pragma_update(None, "key", format!("x'{}'", "5d".repeat(32)))
        .unwrap();
    let storage = fixture.storage.as_mut().unwrap();
    let clip = text("https://example.com/private");
    storage.save(&clip).unwrap();
    let id = clip.hash();
    storage
        .set_sensitive(std::slice::from_ref(&id), true, 10)
        .unwrap();
    assert!(target(storage, &id).unwrap_err().contains("敏感"));
    assert!(!storage.preview_item(&id).unwrap().is_link);
    assert!(!storage.list().unwrap()[0].is_link);
    assert!(!storage.panel_items(&[&id]).unwrap()[0].is_link);
    storage
        .set_sensitive(std::slice::from_ref(&id), false, 10)
        .unwrap();
    assert!(target(storage, &id).is_ok());
    connection
        .execute("UPDATE items SET expires_at=1 WHERE id=?1", [&id])
        .unwrap();
    assert!(target(storage, &id).unwrap_err().contains("到期"));
    let plain = text("not a URL");
    storage.save(&plain).unwrap();
    connection
        .execute("UPDATE items SET is_link=1 WHERE id=?1", [plain.hash()])
        .unwrap();
    assert!(target(storage, &plain.hash()).is_err());
}

#[test]
fn browser_failures_are_reported_without_link_contents() {
    assert!(launch_result(33).is_ok());
    assert!(launch_result(31).unwrap_err().contains("默认浏览器"));
    assert!(launch_result(5).unwrap_err().contains("拒绝"));
    assert!(launch_result(0).is_err());
}
