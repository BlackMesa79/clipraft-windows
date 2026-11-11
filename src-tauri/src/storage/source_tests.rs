//! 合成来源与独立数据库；不接触用户历史。
use super::*;
use crate::capture_source::CaptureSource;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "t6-source-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> Mutex<Storage> {
        Mutex::new(Storage::open(&self.0, &[61; 32]).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn clip() -> Clip {
    Clip {
        formats: vec![model::FormatData {
            name: "CF_UNICODETEXT".into(),
            bytes: "合成来源测试"
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
        }],
    }
}
fn source(exe: &str, title: &str) -> CaptureSource {
    CaptureSource {
        exe: Some(exe.into()),
        title: Some(title.into()),
    }
}
fn save(store: &Mutex<Storage>, clip: &Clip, source: &CaptureSource) -> SaveOutcome {
    Storage::save_shared_with_source(store, clip, Some(source)).unwrap()
}
#[test]
fn source_columns_roundtrip_and_reopen_v4() {
    let root = Fixture::new();
    let store = root.store();
    let c = clip();
    let src = source("notepad.exe", "合成窗口 😀");
    assert!(save(&store, &c, &src).inserted);
    drop(store);
    let store = root.store();
    let s = store.lock().unwrap();
    let row = s.list().unwrap().remove(0);
    assert_eq!(row.source_exe, src.exe);
    assert_eq!(row.source_title, src.title);
    assert_eq!(
        s.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
            .unwrap(),
        4
    );
    assert!(!fs::read_dir(&root.0).unwrap().any(|p| p
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("bak")));
}
#[test]
fn source_missing_saved_as_sql_null() {
    let root = Fixture::new();
    let store = root.store();
    save(&store, &clip(), &CaptureSource::default());
    let s = store.lock().unwrap();
    assert!(s
        .conn
        .query_row(
            "SELECT source_exe IS NULL AND source_title IS NULL FROM items",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(s.list().unwrap()[0].source_exe, None);
}
#[test]
fn duplicate_updates_latest_source_but_preserves_content_and_protection() {
    let root = Fixture::new();
    let store = root.store();
    let c = clip();
    save(&store, &c, &source("first.exe", "一"));
    let before = {
        let s = store.lock().unwrap();
        s.conn
            .execute("UPDATE items SET pinned=1,favorite=1", [])
            .unwrap();
        s.list().unwrap().remove(0)
    };
    assert!(!save(&store, &c, &source("second.exe", "二")).inserted);
    let s = store.lock().unwrap();
    let row = s.list().unwrap().remove(0);
    assert_eq!(row.source_exe.as_deref(), Some("second.exe"));
    assert_eq!(row.source_title.as_deref(), Some("二"));
    assert_eq!(row.created, before.created);
    assert!(row.pinned && row.favorite);
    assert_eq!(row.id, before.id);
    assert_eq!(row.size, before.size);
    assert_eq!(s.load(&row.id).unwrap().hash(), c.hash());
}
#[test]
fn duplicate_unknown_source_clears_previous_source() {
    let root = Fixture::new();
    let store = root.store();
    let c = clip();
    save(&store, &c, &source("old.exe", "旧"));
    assert!(!save(&store, &c, &CaptureSource::default()).inserted);
    let row = store.lock().unwrap().list().unwrap().remove(0);
    assert_eq!(row.source_exe, None);
    assert_eq!(row.source_title, None);
}
#[test]
fn duplicate_after_preparation_updates_source() {
    let root = Fixture::new();
    let store = root.store();
    let c = clip();
    let mut prepared = {
        let s = store.lock().unwrap();
        PreparedSave::new(&c, SaveIdentity::new(&c), &s.key).unwrap()
    };
    prepared.source = Some(source("last.exe", "最后"));
    save(&store, &c, &source("first.exe", "最先"));
    let mut s = store.lock().unwrap();
    assert!(!s.commit_prepared(prepared).unwrap().inserted);
    assert_eq!(s.list().unwrap()[0].source_exe.as_deref(), Some("last.exe"));
}
#[test]
fn file_dedup_updates_source_and_keeps_first_formats() {
    let root = Fixture::new();
    let store = root.store();
    let first = Clip {
        formats: vec![model::FormatData {
            name: "CF_HDROP".into(),
            bytes: model::encode_hdrop(&[r"C:\synthetic\file.txt".into()]),
        }],
    };
    let mut second = first.clone();
    second.formats.push(model::FormatData {
        name: "HTML Format".into(),
        bytes: b"synthetic".to_vec(),
    });
    let a = save(&store, &first, &source("explorer.exe", "文件"));
    let b = save(&store, &second, &source("other.exe", "合成"));
    assert!(!b.inserted);
    assert_eq!(a.id, b.id);
    let s = store.lock().unwrap();
    assert_eq!(s.load(&b.id).unwrap().hash(), first.hash());
    assert_eq!(
        s.list().unwrap()[0].source_exe.as_deref(),
        Some("other.exe")
    );
}
#[test]
fn touch_does_not_overwrite_source() {
    let root = Fixture::new();
    let store = root.store();
    let c = clip();
    save(&store, &c, &source("app.exe", "窗口"));
    let s = store.lock().unwrap();
    s.touch(&c.hash()).unwrap();
    assert_eq!(s.list().unwrap()[0].source_title.as_deref(), Some("窗口"));
}
#[test]
fn source_update_sql_failure_is_atomic() {
    let root = Fixture::new();
    let store = root.store();
    let c = clip();
    save(&store, &c, &source("old.exe", "旧"));
    let before = {
        let s = store.lock().unwrap();
        s.conn.execute_batch("CREATE TRIGGER reject_source BEFORE UPDATE OF source_exe ON items BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        s.conn
            .query_row("SELECT touched FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    assert!(Storage::save_shared_with_source(&store, &c, Some(&source("new.exe", "新"))).is_err());
    let s = store.lock().unwrap();
    assert_eq!(s.list().unwrap()[0].source_exe.as_deref(), Some("old.exe"));
    assert_eq!(
        s.conn
            .query_row("SELECT touched FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn source_exe_is_not_searchable_and_deletion_removes_source_row() {
    let root = Fixture::new();
    let store = root.store();
    let clip = clip();
    let outcome = Storage::save_shared_with_source(
        &store,
        &clip,
        Some(&source("private-source-token.exe", "来源标题")),
    )
    .unwrap();
    {
        let s = store.lock().unwrap();
        let results = s
            .list_page(
                &ListFilter {
                    query: Some("private-source-token".into()),
                    ..Default::default()
                },
                0,
                10,
            )
            .unwrap();
        assert!(results.is_empty(), "source_exe 不应进入搜索语义");
    }
    let report = store
        .lock()
        .unwrap()
        .delete(std::slice::from_ref(&outcome.id))
        .unwrap();
    assert_eq!(report.deleted_items, 1);
    assert!(store.lock().unwrap().list().unwrap().is_empty());
}
