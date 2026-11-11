use super::*;
use std::{
    io::Cursor,
    time::{SystemTime, UNIX_EPOCH},
};
const MASTER: [u8; 32] = [73; 32];
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "m1-storage-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(path.join("data/blobs")).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn connect(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(
        None,
        "key",
        format!(
            "x'{}'",
            MASTER
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
    )
    .unwrap();
    conn
}
fn text(value: &str) -> Clip {
    Clip {
        formats: vec![model::FormatData {
            name: "CF_UNICODETEXT".into(),
            bytes: value
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
        }],
    }
}
fn blob(root: &Path, bytes: &[u8]) -> String {
    let id = model::digest(bytes);
    fs::write(
        root.join("data/blobs").join(&id),
        crypto::encrypt(&crypto::derive(&MASTER), &id, bytes).unwrap(),
    )
    .unwrap();
    id
}
fn legacy(root: &Path) -> (Vec<Clip>, String, String) {
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(3, 2))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let mut clips = vec![
        text("旧库文本 😀"),
        text(&"外部文本".repeat(18000)),
        Clip {
            formats: vec![model::FormatData {
                name: "PNG".into(),
                bytes: png.into_inner(),
            }],
        },
        Clip {
            formats: vec![model::FormatData {
                name: "CF_HDROP".into(),
                bytes: model::encode_hdrop(&[r"C:\测试\报告.txt".into(), r"D:\图片.png".into()]),
            }],
        },
    ];
    let mut rich = text("富文本的纯文本");
    rich.formats.push(model::FormatData {
        name: "Rich Text Format".into(),
        bytes: b"{\\rtf1 synthetic}".to_vec(),
    });
    clips.push(rich);
    let conn = connect(&root.join("history.db"));
    conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE items(id TEXT PRIMARY KEY,view TEXT NOT NULL,touched INTEGER NOT NULL); CREATE TABLE formats(item TEXT NOT NULL REFERENCES items(id),name TEXT NOT NULL,inline BLOB,blob TEXT,PRIMARY KEY(item,name));").unwrap();
    let preview_png = |w| {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(w, 1))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    };
    let thumb = blob(root, &preview_png(1));
    let preview = blob(root, &preview_png(2));
    for (n, clip) in clips.iter().enumerate() {
        let json = serde_json::json!({"id":clip.hash(),"kind":if n==4 {"text"} else {clip.kind()},"summary":"旧记录","formats":clip.formats.iter().map(|f|f.name.clone()).collect::<Vec<_>>(),"thumbnail":if n==2 {Some(&thumb)}else{None},"preview":if n==2 {Some(&preview)}else{None},"placeholder":null,"files":clip.get("CF_HDROP").and_then(model::decode_hdrop).unwrap_or_default()});
        conn.execute(
            "INSERT INTO items VALUES(?1,?2,?3)",
            params![clip.hash(), json.to_string(), 100 + n],
        )
        .unwrap();
        for f in &clip.formats {
            let external = f.name == "PNG" || f.bytes.len() > 64 * 1024;
            conn.execute(
                "INSERT INTO formats VALUES(?1,?2,?3,?4)",
                params![
                    clip.hash(),
                    f.name,
                    if external { None } else { Some(&f.bytes) },
                    if external {
                        Some(blob(root, &f.bytes))
                    } else {
                        None
                    }
                ],
            )
            .unwrap();
        }
    }
    (clips, thumb, preview)
}
fn version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}
fn backups(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("history.db.bak-v")
        })
        .collect()
}

/// 按旧版 v0 结构建库，返回可继续写入的旧连接。
fn v0_db(root: &Path) -> Connection {
    let conn = connect(&root.join("history.db"));
    conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE items(id TEXT PRIMARY KEY,view TEXT NOT NULL,touched INTEGER NOT NULL); CREATE TABLE formats(item TEXT NOT NULL REFERENCES items(id),name TEXT NOT NULL,inline BLOB,blob TEXT,PRIMARY KEY(item,name));").unwrap();
    conn
}
fn v0_view(clip: &Clip) -> String {
    serde_json::json!({
        "id": clip.hash(),
        "kind": clip.kind(),
        "summary": "旧记录",
        "formats": clip.formats.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
        "thumbnail": null,
        "preview": null,
        "placeholder": null,
        "files": clip.get("CF_HDROP").and_then(model::decode_hdrop).unwrap_or_default(),
    })
    .to_string()
}
/// 按旧版策略入库：图片格式与超过 64KB 的格式外置，其余内联。
fn put_v0(conn: &Connection, root: &Path, clip: &Clip, touched: i64) {
    conn.execute(
        "INSERT INTO items VALUES(?1,?2,?3)",
        params![clip.hash(), v0_view(clip), touched],
    )
    .unwrap();
    for f in &clip.formats {
        let external =
            model::image_priority().contains(&f.name.as_str()) || f.bytes.len() > 64 * 1024;
        conn.execute(
            "INSERT INTO formats VALUES(?1,?2,?3,?4)",
            params![
                clip.hash(),
                f.name,
                if external {
                    None
                } else {
                    Some(f.bytes.clone())
                },
                if external {
                    Some(blob(root, &f.bytes))
                } else {
                    None
                }
            ],
        )
        .unwrap();
    }
}
/// 迁移后的 (size, search_text)。
fn item_text(s: &Storage, id: &str) -> (i64, String) {
    s.conn
        .query_row(
            "SELECT size,search_text FROM items WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}
fn columns(conn: &Connection) -> Vec<String> {
    conn.prepare("PRAGMA table_info(items)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn encrypted_v0_migration_preserves_data_and_backup_and_is_idempotent() {
    let root = Fixture::new();
    let (clips, thumb, preview) = legacy(&root.0);
    let before = unix_millis();
    let s = Storage::open(&root.0, &MASTER).unwrap();
    let after = unix_millis();
    assert_eq!(version(&s.conn), migrations::VERSION);
    for clip in &clips {
        assert_eq!(s.load(&clip.hash()).unwrap().formats, {
            let mut f = clip.formats.clone();
            f.sort_by(|a, b| a.name.cmp(&b.name));
            f
        });
        let view = s
            .list()
            .unwrap()
            .into_iter()
            .find(|v| v.id == clip.hash())
            .unwrap();
        assert_eq!(view.kind, clip.kind());
        assert!((before..=after).contains(&view.created));
        assert_eq!(view.created, view.used_at);
        assert_eq!(
            view.size,
            clip.formats
                .iter()
                .map(|f| f.bytes.len() as i64)
                .sum::<i64>()
        );
        assert!(!view.pinned && !view.favorite && !view.sensitive);
        assert!(
            view.group_id.is_none()
                && view.source_exe.is_none()
                && view.source_title.is_none()
                && view.expires_at.is_none()
        );
        assert_eq!(
            s.conn
                .query_row(
                    "SELECT search_text FROM items WHERE id=?1",
                    [clip.hash()],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            search_text(clip)
        );
        let json: String = s
            .conn
            .query_row("SELECT view FROM items WHERE id=?1", [clip.hash()], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&json)
            .unwrap()
            .get("kind")
            .is_none());
    }
    assert_eq!(
        image::load_from_memory(&s.read_thumbnail(&thumb).unwrap())
            .unwrap()
            .width(),
        1
    );
    assert_eq!(
        image::load_from_memory(&s.read_thumbnail(&preview).unwrap())
            .unwrap()
            .width(),
        2
    );
    assert!(s
        .read_thumbnail(&model::digest(&clips[2].formats[0].bytes))
        .is_err());
    assert_eq!(
        s.conn
            .query_row("SELECT MIN(touched) FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        100
    );
    let paths = backups(&root.0);
    assert_eq!(paths, vec![root.0.join("history.db.bak-v0")]);
    let encrypted = fs::read(&paths[0]).unwrap();
    assert_ne!(&encrypted[..16], b"SQLite format 3\0");
    let backup = connect(&paths[0]);
    assert_eq!(version(&backup), 0);
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM items", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        clips.len()
    );
    let mut q = backup.prepare("SELECT view FROM items").unwrap();
    assert_eq!(
        q.query_map([], |r| r.get::<_, String>(0)).unwrap().count(),
        clips.len()
    );
    drop(q);
    drop(backup);
    drop(s);
    let s = Storage::open(&root.0, &MASTER).unwrap();
    assert_eq!(s.list().unwrap().len(), clips.len());
    assert_eq!(backups(&root.0), paths);
    assert_eq!(fs::read(&paths[0]).unwrap(), encrypted);
}
#[test]
fn failure_rolls_back_schema_version_and_rows_and_does_not_overwrite_backup() {
    let root = Fixture::new();
    let (clips, _, _) = legacy(&root.0);
    let original = root.0.join("history.db.bak-v0");
    fs::write(&original, b"existing backup").unwrap();
    let error = Storage::open_with_hook(&root.0, &MASTER, |tx| {
        assert_eq!(version(tx), 1);
        bail!("测试注入迁移失败")
    })
    .err()
    .unwrap()
    .to_string();
    assert!(
        error.contains("已回滚") && error.contains("原数据未改动") && error.contains("备份位于")
    );
    assert_eq!(fs::read(&original).unwrap(), b"existing backup");
    let conn = connect(&root.0.join("history.db"));
    assert_eq!(version(&conn), 0);
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(items)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(columns, ["id", "view", "touched"]);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM items", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        clips.len()
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('groups','settings')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    let actual = backups(&root.0)
        .into_iter()
        .find(|p| p != &original)
        .unwrap();
    let conn = connect(&actual);
    assert_eq!(version(&conn), 0);
    drop(conn);
    let s = Storage::open(&root.0, &MASTER).unwrap();
    for clip in clips {
        assert_eq!(s.load(&clip.hash()).unwrap().hash(), clip.hash());
    }
}
#[test]
fn missing_or_unreadable_blobs_do_not_abort_migration() {
    let root = Fixture::new();
    let (clips, _, _) = legacy(&root.0);
    fs::remove_file(
        root.0
            .join("data/blobs")
            .join(model::digest(&clips[2].formats[0].bytes)),
    )
    .unwrap();
    let external = root
        .0
        .join("data/blobs")
        .join(model::digest(&clips[1].formats[0].bytes));
    fs::write(external, vec![0; 100]).unwrap();
    let s = Storage::open(&root.0, &MASTER).unwrap();
    let views = s.list().unwrap();
    let image = views.iter().find(|v| v.id == clips[2].hash()).unwrap();
    assert_eq!(image.size, 0);
    assert!(s
        .conn
        .query_row(
            "SELECT search_text FROM items WHERE id=?1",
            [&image.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap()
        .is_empty());
    let text = views.iter().find(|v| v.id == clips[1].hash()).unwrap();
    assert_eq!(text.size, 100 - crypto::BLOB_OVERHEAD as i64);
    assert!(s
        .conn
        .query_row(
            "SELECT search_text FROM items WHERE id=?1",
            [&text.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap()
        .is_empty());
}
#[test]
fn fresh_and_migrated_schema_match_and_future_version_is_preserved() {
    let fresh = Fixture::new();
    let legacy_root = Fixture::new();
    legacy(&legacy_root.0);
    let a = Storage::open(&fresh.0, &MASTER).unwrap();
    let b = Storage::open(&legacy_root.0, &MASTER).unwrap();
    let schema = |c: &Connection| {
        c.prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| {
                r.unwrap()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(schema(&a.conn), schema(&b.conn));
    assert!(backups(&fresh.0).is_empty());
    a.conn.pragma_update(None, "user_version", 99).unwrap();
    drop(a);
    assert!(Storage::open(&fresh.0, &MASTER)
        .err()
        .unwrap()
        .to_string()
        .contains("不受支持"));
    let c = connect(&fresh.0.join("history.db"));
    assert_eq!(version(&c), 99);
}
#[test]
fn checkpoint_busy_aborts_before_backup_or_schema_changes() {
    let root = Fixture::new();
    legacy(&root.0);
    let reader = connect(&root.0.join("history.db"));
    reader.execute_batch("BEGIN; SELECT * FROM items;").unwrap();
    let writer = connect(&root.0.join("history.db"));
    writer
        .execute("UPDATE items SET touched=touched+1", [])
        .unwrap();
    drop(writer);
    let error = Storage::open(&root.0, &MASTER).err().unwrap().to_string();
    assert!(error.contains("占用"), "{error}");
    assert!(backups(&root.0).is_empty());
    assert_eq!(version(&reader), 0);
    reader.execute_batch("ROLLBACK").unwrap();
}
#[test]
fn page_sort_filters_boundaries_and_column_authority() {
    let root = Fixture::new();
    let mut s = Storage::open(&root.0, &MASTER).unwrap();
    for n in 0..505 {
        s.save(&text(&format!("page-{n}"))).unwrap();
    }
    let first = text("page-0").hash();
    let second = text("page-1").hash();
    s.conn
        .execute("INSERT INTO groups VALUES(7,'测试组',0,1)", [])
        .unwrap();
    s.conn
        .execute(
            "UPDATE items SET pinned=1,favorite=1,group_id=7 WHERE id=?1",
            [&first],
        )
        .unwrap();
    s.conn.execute("UPDATE items SET kind='rich',favorite=1,source_exe='test.exe',source_title='测试',sensitive=1,expires_at=123 WHERE id=?1",[&second]).unwrap();
    let filter = ListFilter::default();
    let page = s.list_page(&filter, 0, 2).unwrap();
    assert_eq!(page[0].id, first);
    assert_eq!(page[1].id, text("page-504").hash());
    assert_eq!(
        s.list_page(&filter, 2, 1).unwrap()[0].id,
        text("page-503").hash()
    );
    assert_eq!(s.list_page(&filter, 0, usize::MAX).unwrap().len(), 500);
    assert!(s.list_page(&filter, 0, 0).unwrap().is_empty());
    assert!(s.list_page(&filter, 505, 50).unwrap().is_empty());
    assert!(s.list_page(&filter, usize::MAX, 50).unwrap().is_empty());
    assert_eq!(s.list_page(&filter, 500, 50).unwrap().len(), 5);
    assert_eq!(s.list().unwrap().len(), 200);
    assert_eq!(
        s.list_page(
            &ListFilter {
                favorite_only: true,
                ..Default::default()
            },
            0,
            50
        )
        .unwrap()
        .len(),
        2
    );
    assert_eq!(
        s.list_page(
            &ListFilter {
                group_id: Some(7),
                ..Default::default()
            },
            0,
            50
        )
        .unwrap()[0]
            .id,
        first
    );
    let rich = s
        .list_page(
            &ListFilter {
                kind: Some("rich".into()),
                favorite_only: true,
                ..Default::default()
            },
            0,
            50,
        )
        .unwrap();
    assert_eq!(rich.len(), 1);
    assert_eq!(rich[0].id, second);
    assert!(rich[0].sensitive);
    assert_eq!(rich[0].expires_at, Some(123));
    assert_eq!(rich[0].source_exe.as_deref(), Some("test.exe"));
    assert!(s
        .list_page(
            &ListFilter {
                kind: Some("' OR 1=1 --".into()),
                ..Default::default()
            },
            0,
            50
        )
        .unwrap()
        .is_empty());
    assert!(s
        .list_page(
            &ListFilter {
                kind: Some("rich".into()),
                group_id: Some(7),
                ..Default::default()
            },
            0,
            50
        )
        .unwrap()
        .is_empty());
    let old = s.list_page(&filter, 0, 1).unwrap().remove(0);
    s.conn
        .execute("UPDATE items SET used_at=0 WHERE id=?1", [&first])
        .unwrap();
    s.touch(&first).unwrap();
    let updated = s.list_page(&filter, 0, 1).unwrap().remove(0);
    assert_eq!(old.created, updated.created);
    assert!(updated.used_at > 0);
    assert!(updated.pinned);
    s.conn
        .execute("UPDATE items SET used_at=0 WHERE id=?1", [&first])
        .unwrap();
    s.save(&text("page-0")).unwrap();
    let updated = s.list_page(&filter, 0, 1).unwrap().remove(0);
    assert_eq!(old.created, updated.created);
    assert!(updated.used_at > 0);
    assert!(updated.favorite);
}
#[test]
fn serde_defaults_and_utf8_search_boundary() {
    let old = r#"{"id":"old","kind":"text","summary":"old","formats":[],"thumbnail":null,"placeholder":null,"files":[]}"#;
    let view: ItemView = serde_json::from_str(old).unwrap();
    assert_eq!(view.created, 0);
    assert_eq!(view.size, 0);
    assert!(!view.favorite);
    assert!(view.group_id.is_none());
    let value = search_text(&text(&"中😀".repeat(5000)));
    assert!(value.len() <= 16 * 1024);
    assert!(value.len() > 16 * 1024 - 4);
}

#[test]
fn external_long_text_truncates_search_text_at_char_boundary() {
    let root = Fixture::new();
    let conn = v0_db(&root.0);
    // 16KiB 边界正好落在 3 字节汉字与 4 字节 emoji 内部，两者都必须回退到字符边界。
    let cases = ["中".repeat(40000), "a😀".repeat(20000)];
    let mut clips = Vec::new();
    for (n, value) in cases.iter().enumerate() {
        let clip = text(value);
        assert!(clip.formats[0].bytes.len() > 64 * 1024);
        put_v0(&conn, &root.0, &clip, 10 + n as i64);
        clips.push(clip);
    }
    let short = text("短文本");
    put_v0(&conn, &root.0, &short, 99);
    drop(conn);
    let s = Storage::open(&root.0, &MASTER).unwrap();
    for (clip, expected) in clips.iter().zip([16383usize, 16381]) {
        let full = clip.text().unwrap();
        assert!(full.len() > 16 * 1024);
        assert!(16 * 1024 - expected < 4, "应贴紧 16KiB 上界回退");
        assert!(full.is_char_boundary(expected));
        assert!(!full.is_char_boundary(expected + 1));
        let (size, stored) = item_text(&s, &clip.hash());
        // 外置文本的描述长度取明文长度（文件长度减去加密开销）。
        assert_eq!(size, clip.formats[0].bytes.len() as i64);
        assert_eq!(stored.len(), expected);
        assert_eq!(stored, full[..expected]);
        assert!(!stored.contains('\u{fffd}'));
        assert_eq!(s.load(&clip.hash()).unwrap().hash(), clip.hash());
    }
    let (size, stored) = item_text(&s, &short.hash());
    assert_eq!(stored, "短文本");
    assert_eq!(size, short.formats[0].bytes.len() as i64);
}

#[test]
fn bulk_migration_keeps_every_row_complete() {
    let root = Fixture::new();
    let conn = v0_db(&root.0);
    let body = "批量".repeat(1500);
    let clips: Vec<Clip> = (0..1500).map(|n| text(&format!("{n}-{body}"))).collect();
    for (n, clip) in clips.iter().enumerate() {
        put_v0(&conn, &root.0, clip, 1000 + n as i64);
    }
    drop(conn);
    let s = Storage::open(&root.0, &MASTER).unwrap();
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM items", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        clips.len()
    );
    for clip in &clips {
        let (size, stored) = item_text(&s, &clip.hash());
        assert_eq!(size, clip.formats[0].bytes.len() as i64);
        // 迁移边扫描 items 边更新同一行，任何被跳过的行都会留下空 search_text。
        assert_eq!(stored, clip.text().unwrap());
        let json: String = s
            .conn
            .query_row("SELECT view FROM items WHERE id=?1", [clip.hash()], |r| {
                r.get(0)
            })
            .unwrap();
        let view: ItemView = serde_json::from_str(&json).unwrap();
        assert_eq!(view.id, clip.hash());
        assert_eq!(view.kind, "");
        assert_eq!(view.formats, vec!["CF_UNICODETEXT".to_string()]);
    }
}

#[test]
fn migration_succeeds_without_overwriting_existing_backup() {
    let root = Fixture::new();
    let (clips, _, _) = legacy(&root.0);
    let existing = root.0.join("history.db.bak-v0");
    fs::write(&existing, b"existing backup").unwrap();
    let s = Storage::open(&root.0, &MASTER).unwrap();
    assert_eq!(version(&s.conn), migrations::VERSION);
    assert_eq!(s.list().unwrap().len(), clips.len());
    assert_eq!(fs::read(&existing).unwrap(), b"existing backup");
    let all = backups(&root.0);
    assert_eq!(all.len(), 2, "{all:?}");
    let fresh = all.iter().find(|p| **p != existing).unwrap();
    assert!(fresh
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("history.db.bak-v0-"));
    let conn = connect(fresh);
    assert_eq!(version(&conn), 0);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM items", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        clips.len()
    );
    drop(conn);
    // 再次打开不产生新备份，也不改写任何已有备份。
    drop(s);
    Storage::open(&root.0, &MASTER).unwrap();
    assert_eq!(backups(&root.0).len(), 2);
    assert_eq!(fs::read(&existing).unwrap(), b"existing backup");
}

#[test]
fn corrupt_view_json_rolls_back_and_keeps_original_database() {
    let root = Fixture::new();
    let conn = v0_db(&root.0);
    let good = text("好记录");
    put_v0(&conn, &root.0, &good, 1);
    let corrupt = "{\"id\": 不是 JSON";
    conn.execute(
        "INSERT INTO items VALUES(?1,?2,?3)",
        params!["损坏记录", corrupt, 2],
    )
    .unwrap();
    drop(conn);
    let before = fs::read(root.0.join("history.db")).unwrap();
    let error = Storage::open(&root.0, &MASTER).err().unwrap().to_string();
    assert!(error.contains("已回滚"), "{error}");
    assert!(error.contains("原数据未改动"), "{error}");
    assert!(error.contains("展示数据无法解析"), "{error}");
    let conn = connect(&root.0.join("history.db"));
    assert_eq!(version(&conn), 0);
    assert_eq!(columns(&conn), ["id", "view", "touched"]);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM items", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM formats", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT view FROM items WHERE id=?1",
            ["损坏记录"],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        corrupt
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('groups','settings')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    let baked = backups(&root.0);
    assert_eq!(baked.len(), 1);
    assert_eq!(
        fs::read(&baked[0]).unwrap(),
        before,
        "备份应逐字节等于迁移前的原库"
    );
    // 修好损坏行之后可以完成迁移，且不覆盖失败那一轮的备份。
    let conn = connect(&root.0.join("history.db"));
    conn.execute("DELETE FROM items WHERE id=?1", ["损坏记录"])
        .unwrap();
    drop(conn);
    let s = Storage::open(&root.0, &MASTER).unwrap();
    assert_eq!(version(&s.conn), migrations::VERSION);
    assert_eq!(s.list().unwrap().len(), 1);
    assert_eq!(s.load(&good.hash()).unwrap().formats, good.formats);
    assert_eq!(backups(&root.0).len(), 2);
}

#[test]
fn missing_external_text_blob_degrades_size_and_search_text() {
    let root = Fixture::new();
    let (clips, thumb, _) = legacy(&root.0);
    // clips[1] 是超过 64KB 的外置文本，直接删除其外部对象。
    let external = model::digest(&clips[1].formats[0].bytes);
    fs::remove_file(root.0.join("data/blobs").join(&external)).unwrap();
    fs::remove_file(root.0.join("data/blobs").join(&thumb)).unwrap();
    let s = Storage::open(&root.0, &MASTER).unwrap();
    let views = s.list().unwrap();
    let view = views.iter().find(|v| v.id == clips[1].hash()).unwrap();
    assert_eq!(view.kind, "text");
    assert_eq!(view.size, 0);
    let (size, stored) = item_text(&s, &view.id);
    assert_eq!(size, 0);
    assert!(stored.is_empty());
    assert!(s.load(&clips[1].hash()).is_err());
    // 其余记录不受影响。
    assert_eq!(s.load(&clips[0].hash()).unwrap().hash(), clips[0].hash());
    // 缩略图对象缺失时视图仍保留引用，读取报错而不是 panic。
    let image = views.iter().find(|v| v.id == clips[2].hash()).unwrap();
    assert_eq!(image.thumbnail.as_deref(), Some(thumb.as_str()));
    assert!(s.read_thumbnail(&thumb).is_err());
    assert!(s.read_thumbnail(&external).is_err());
}

#[test]
fn format_rows_without_payload_do_not_abort_migration() {
    let root = Fixture::new();
    legacy(&root.0);
    let ghost = text("无载荷记录");
    let conn = connect(&root.0.join("history.db"));
    put_v0(&conn, &root.0, &ghost, 900);
    conn.execute(
        "INSERT INTO formats VALUES(?1,'CF_HTML',NULL,NULL)",
        [ghost.hash()],
    )
    .unwrap();
    drop(conn);
    let s = Storage::open(&root.0, &MASTER).unwrap();
    let (size, stored) = item_text(&s, &ghost.hash());
    assert_eq!(stored, "无载荷记录");
    assert_eq!(size, ghost.formats[0].bytes.len() as i64);
    assert!(s.list().unwrap().iter().any(|v| v.id == ghost.hash()));
    assert!(
        s.load(&ghost.hash()).is_err(),
        "缺少载荷的格式应报错，而不是返回不完整记录"
    );
}

#[test]
fn unsupported_version_with_data_is_rejected_without_side_effects() {
    let root = Fixture::new();
    let (clips, _, _) = legacy(&root.0);
    let conn = connect(&root.0.join("history.db"));
    conn.pragma_update(None, "user_version", migrations::VERSION + 1)
        .unwrap();
    drop(conn);
    let error = Storage::open(&root.0, &MASTER).err().unwrap().to_string();
    assert!(error.contains("不受支持"), "{error}");
    assert!(error.contains("原数据未改动"), "{error}");
    assert!(backups(&root.0).is_empty(), "拒绝打开时不应写备份");
    let conn = connect(&root.0.join("history.db"));
    assert_eq!(version(&conn), migrations::VERSION + 1);
    assert_eq!(columns(&conn), ["id", "view", "touched"]);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM items", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        clips.len()
    );
    // 负版本号同样视为不受支持。
    conn.pragma_update(None, "user_version", -1).unwrap();
    assert!(Storage::open(&root.0, &MASTER)
        .err()
        .unwrap()
        .to_string()
        .contains("不受支持"));
    assert!(backups(&root.0).is_empty());
}

#[test]
fn files_search_text_uses_the_same_size_cap_as_text() {
    // 文件名与完整路径拼接后，同样在字符边界截断到 16KiB。
    let paths: Vec<String> = (0..3000)
        .map(|n| format!("C:\\很长的目录\\xx{n:04}-示例文件名.txt"))
        .collect();
    let clip = Clip {
        formats: vec![model::FormatData {
            name: "CF_HDROP".into(),
            bytes: model::encode_hdrop(&paths),
        }],
    };
    let value = search_text(&clip);
    assert!(
        value.len() <= 16 * 1024,
        "files 的 search_text 长度 {} 超过 16KiB 上限",
        value.len()
    );
    let full = paths
        .iter()
        .flat_map(|path| [path.rsplit('\\').next().unwrap(), path.as_str()])
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !full.is_char_boundary(16 * 1024),
        "测试应覆盖多字节字符截断"
    );
    assert!(full.is_char_boundary(value.len()));
    assert!(value.len() > 16 * 1024 - 4);
    assert_eq!(value, full[..value.len()]);
}

fn page_ids(s: &Storage, filter: &ListFilter, offset: usize, limit: usize) -> Vec<String> {
    s.list_page(filter, offset, limit)
        .unwrap()
        .into_iter()
        .map(|v| v.id)
        .collect()
}

#[test]
fn page_pinned_priority_offsets_limits_and_filter_combinations() {
    let root = Fixture::new();
    let mut s = Storage::open(&root.0, &MASTER).unwrap();
    let saved: Vec<Clip> = (0..12).map(|n| text(&format!("条目-{n}"))).collect();
    for clip in &saved {
        s.save(clip).unwrap();
    }
    let id = |n: usize| saved[n].hash();
    let all = ListFilter::default();
    // 默认顺序：最近保存的在前（save 递增 touched）。
    assert_eq!(
        page_ids(&s, &all, 0, PAGE_LIMIT),
        (0..12).rev().map(id).collect::<Vec<_>>()
    );
    // 置顶优先，多个置顶之间仍按 touched 降序。
    s.conn
        .execute(
            "UPDATE items SET pinned=1 WHERE id IN (?1,?2)",
            params![id(3), id(7)],
        )
        .unwrap();
    let order: Vec<String> = [7, 3, 11, 10, 9, 8, 6, 5, 4, 2, 1, 0]
        .iter()
        .map(|n| id(*n))
        .collect();
    assert_eq!(page_ids(&s, &all, 0, PAGE_LIMIT), order);
    assert_eq!(page_ids(&s, &all, 1, 3), order[1..4].to_vec());
    assert_eq!(page_ids(&s, &all, 11, PAGE_LIMIT), vec![id(0)]);
    assert!(page_ids(&s, &all, 12, PAGE_LIMIT).is_empty());
    assert!(page_ids(&s, &all, 13, PAGE_LIMIT).is_empty());
    assert!(page_ids(&s, &all, usize::MAX, PAGE_LIMIT).is_empty());
    // limit 边界：0 返回空，超过上限时按 PAGE_LIMIT 截断。
    assert!(page_ids(&s, &all, 0, 0).is_empty());
    assert!(page_ids(&s, &all, usize::MAX, 0).is_empty());
    assert_eq!(page_ids(&s, &all, 0, 1), vec![id(7)]);
    assert_eq!(page_ids(&s, &all, 0, usize::MAX).len(), 12);
    assert_eq!(page_ids(&s, &all, 0, PAGE_LIMIT + 1).len(), 12);
    // 过滤条件：各不相同且可组合。
    let favorite = ListFilter {
        favorite_only: true,
        ..Default::default()
    };
    assert!(page_ids(&s, &favorite, 0, PAGE_LIMIT).is_empty());
    s.conn
        .execute(
            "UPDATE items SET favorite=1 WHERE id IN (?1,?2)",
            params![id(0), id(5)],
        )
        .unwrap();
    s.conn
        .execute("INSERT INTO groups VALUES(3,'测试组',0,1)", [])
        .unwrap();
    s.conn
        .execute("UPDATE items SET group_id=3 WHERE id=?1", [id(5)])
        .unwrap();
    assert_eq!(
        page_ids(&s, &favorite, 0, PAGE_LIMIT),
        vec![id(5), id(0)],
        "收藏过滤按 touched 降序"
    );
    let combo = ListFilter {
        kind: Some("text".into()),
        favorite_only: true,
        group_id: Some(3),
        ..Default::default()
    };
    assert_eq!(page_ids(&s, &combo, 0, PAGE_LIMIT), vec![id(5)]);
    assert!(page_ids(&s, &combo, 1, PAGE_LIMIT).is_empty());
    assert!(page_ids(&s, &combo, 0, 0).is_empty());
    for filter in [
        ListFilter {
            kind: Some("rich".into()),
            ..combo.clone()
        },
        ListFilter {
            kind: Some("image".into()),
            ..combo.clone()
        },
        ListFilter {
            group_id: Some(99),
            ..combo.clone()
        },
        ListFilter {
            kind: None,
            favorite_only: false,
            group_id: Some(99),
            ..Default::default()
        },
        ListFilter {
            kind: Some(String::new()),
            ..Default::default()
        },
    ] {
        assert!(
            page_ids(&s, &filter, 0, PAGE_LIMIT).is_empty(),
            "{filter:?} 不应命中任何记录"
        );
    }
    // 去掉任意一个条件都能扩大结果集，说明三个条件确实独立生效。
    assert_eq!(
        page_ids(
            &s,
            &ListFilter {
                kind: Some("text".into()),
                favorite_only: true,
                group_id: None,
                ..Default::default()
            },
            0,
            PAGE_LIMIT
        )
        .len(),
        2
    );
    assert_eq!(
        page_ids(
            &s,
            &ListFilter {
                kind: None,
                favorite_only: true,
                group_id: Some(3),
                ..Default::default()
            },
            0,
            PAGE_LIMIT
        ),
        vec![id(5)]
    );
    // 置顶在过滤结果内部仍然优先。
    s.conn
        .execute("UPDATE items SET pinned=1 WHERE id=?1", [id(0)])
        .unwrap();
    assert_eq!(page_ids(&s, &favorite, 0, PAGE_LIMIT), vec![id(0), id(5)]);
    assert_eq!(page_ids(&s, &favorite, 1, PAGE_LIMIT), vec![id(5)]);
    assert_eq!(s.list().unwrap().len(), 12);
}

fn file_clip(path: &str, effect: Option<u32>, rich: bool) -> Clip {
    let mut formats = vec![model::FormatData {
        name: "CF_HDROP".into(),
        bytes: model::encode_hdrop(&[path.into()]),
    }];
    if let Some(effect) = effect {
        formats.push(model::FormatData {
            name: "Preferred DropEffect".into(),
            bytes: effect.to_le_bytes().to_vec(),
        });
    }
    if rich {
        for (name, bytes) in [
            ("CF_DIB", vec![0; 9_500_000]),
            ("CF_DIBV5", vec![1; 9_500_000]),
            ("HTML Format", b"<img src='synthetic.jpg'>".to_vec()),
        ] {
            formats.push(model::FormatData {
                name: name.into(),
                bytes,
            });
        }
    }
    Clip { formats }
}
fn blob_snapshot(root: &Path) -> std::collections::BTreeMap<String, String> {
    fs::read_dir(root.join("data/blobs"))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                model::digest(&fs::read(path).unwrap()),
            )
        })
        .collect()
}
fn check_file_save_keeps_first_formats(rich_first: bool) {
    let root = Fixture::new();
    let path = root.0.join("desktop.jpg").to_string_lossy().into_owned();
    let first = file_clip(&path, Some(if rich_first { 1 } else { 5 }), rich_first);
    let second = file_clip(&path, Some(if rich_first { 5 } else { 1 }), !rich_first);
    let mut s = Storage::open(&root.0, &MASTER).unwrap();
    s.save(&first).unwrap();
    let original = s.load(&first.hash()).unwrap().formats;
    let before = s.list().unwrap().remove(0);
    let blobs = blob_snapshot(&root.0);
    s.save(&second).unwrap();
    assert_eq!(s.list().unwrap().len(), 1);
    // 插入另一条记录后，再复制文件应将旧文件记录移回顶部。
    s.save(&text("其他记录")).unwrap();
    s.conn
        .execute("UPDATE items SET used_at=0 WHERE id=?1", [first.hash()])
        .unwrap();
    s.save(&second).unwrap();
    let items = s.list().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, first.hash());
    assert_eq!(items[0].created, before.created);
    assert_eq!(items[0].size, before.size);
    assert_eq!(items[0].thumbnail, before.thumbnail);
    assert_eq!(items[0].preview, before.preview);
    assert!(items[0].used_at > 0);
    assert_eq!(s.load(&first.hash()).unwrap().formats, original);
    assert_eq!(blob_snapshot(&root.0), blobs);
    let (key, touched): (String, i64) = s
        .conn
        .query_row(
            "SELECT dedup_key,touched FROM items WHERE id=?1",
            [first.hash()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(key, first.dedup_key());
    assert_eq!(touched, 4);
    assert!(s.load(&second.hash()).is_err());
}
#[test]
fn save_explorer_then_qq_deduplicates_before_writing_any_blobs() {
    check_file_save_keeps_first_formats(false);
}
#[test]
fn save_qq_then_explorer_keeps_the_original_bitmap_formats_and_blobs() {
    check_file_save_keeps_first_formats(true);
}
#[test]
fn save_heals_file_dedup_key_fallback_and_then_matches_extra_dib() {
    let root = Fixture::new();
    let clip = file_clip(r"D:\Desktop\image.jpg", None, false);
    let id = clip.hash();
    let key = clip.dedup_key();
    assert_ne!(id, key);
    let mut s = Storage::open(&root.0, &MASTER).unwrap();
    s.save(&clip).unwrap();
    s.save(&text("其他记录")).unwrap();
    // 模拟迁移无法从外部 CF_HDROP 计算文件键而退回 id。
    s.conn
        .execute("UPDATE items SET dedup_key=id,used_at=0 WHERE id=?1", [&id])
        .unwrap();
    assert_ne!(s.list().unwrap()[0].id, id);
    s.save(&clip).unwrap();
    let items = s.list().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, id);
    assert!(items[0].used_at > 0);
    let healed: (String, i64) = s
        .conn
        .query_row(
            "SELECT dedup_key,touched FROM items WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(healed, (key.clone(), 3));

    let mut rich = clip.clone();
    rich.formats.push(model::FormatData {
        name: "CF_DIB".into(),
        bytes: vec![0; 40],
    });
    assert_ne!(rich.hash(), id);
    assert_eq!(rich.dedup_key(), key);
    s.save(&text("其他记录")).unwrap();
    s.conn
        .execute("UPDATE items SET used_at=0 WHERE id=?1", [&id])
        .unwrap();
    s.save(&rich).unwrap();
    let items = s.list().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, id);
    assert!(items[0].used_at > 0);
    let touched: i64 = s
        .conn
        .query_row("SELECT touched FROM items WHERE id=?1", [&id], |r| r.get(0))
        .unwrap();
    assert_eq!(touched, 5);
    assert_eq!(s.load(&id).unwrap().formats, clip.formats);
}
#[test]
fn save_selects_latest_across_id_fallback_and_file_key_matches() {
    for id_match_is_latest in [false, true] {
        let root = Fixture::new();
        let clip = file_clip(r"D:\Desktop\image.jpg", None, false);
        let mut rich = clip.clone();
        rich.formats.push(model::FormatData {
            name: "CF_DIB".into(),
            bytes: vec![0; 40],
        });
        let id = clip.hash();
        let rich_id = rich.hash();
        let mut s = Storage::open(&root.0, &MASTER).unwrap();
        s.save(&clip).unwrap();
        s.conn
            .execute("UPDATE items SET dedup_key=id WHERE id=?1", [&id])
            .unwrap();
        s.save(&rich).unwrap();
        if id_match_is_latest {
            s.conn
                .execute("UPDATE items SET touched=3 WHERE id=?1", [&id])
                .unwrap();
        }
        s.conn.execute("UPDATE items SET used_at=0", []).unwrap();
        let (expected, untouched) = if id_match_is_latest {
            (&id, &rich_id)
        } else {
            (&rich_id, &id)
        };
        let before: (String, i64, i64) = s
            .conn
            .query_row(
                "SELECT dedup_key,touched,used_at FROM items WHERE id=?1",
                [untouched],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        s.save(&clip).unwrap();
        let items = s.list().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(&items[0].id, expected);
        assert!(items[0].used_at > 0);
        let key: String = s
            .conn
            .query_row("SELECT dedup_key FROM items WHERE id=?1", [expected], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(key, clip.dedup_key());
        let after: (String, i64, i64) = s
            .conn
            .query_row(
                "SELECT dedup_key,touched,used_at FROM items WHERE id=?1",
                [untouched],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(after, before);
    }
}
#[test]
fn save_copy_and_cut_remain_separate_records() {
    let root = Fixture::new();
    let path = root.0.join("desktop.jpg").to_string_lossy().into_owned();
    let copy = file_clip(&path, Some(5), false);
    let cut = file_clip(&path, Some(2), false);
    let mut s = Storage::open(&root.0, &MASTER).unwrap();
    s.save(&copy).unwrap();
    s.save(&cut).unwrap();
    s.save(&copy).unwrap();
    assert_eq!(s.list().unwrap().len(), 2);
    assert_eq!(s.list().unwrap()[0].id, copy.hash());
    assert_eq!(
        s.load(&cut.hash()).unwrap().get("Preferred DropEffect"),
        Some(2u32.to_le_bytes().as_slice())
    );
}

/// 通过逐版事务提交 v1，主动中断 v2，构造真实的 v1 中间库。
fn stage_v1(root: &Path) {
    let error = Storage::open_with_hook(root, &MASTER, |tx| {
        if version(tx) == 2 {
            bail!("测试停在 v1");
        }
        Ok(())
    })
    .err()
    .unwrap();
    assert!(error.to_string().contains("已回滚"));
    let conn = connect(&root.join("history.db"));
    assert_eq!(version(&conn), 1);
    assert!(!columns(&conn).contains(&"dedup_key".into()));
}
fn check_dedup_migration(start_version: i64) {
    let root = Fixture::new();
    let conn = v0_db(&root.0);
    let copy = file_clip(r"D:\Desktop\image.jpg", Some(5), false);
    let qq = file_clip("d:/desktop/IMAGE.JPG/", Some(1), true);
    let cut = file_clip(r"D:\Desktop\image.jpg", Some(2), false);
    let absent_effect = file_clip(r"D:\Desktop\image.jpg", None, false);
    let mut invalid = file_clip(r"D:\invalid.jpg", None, false);
    invalid.formats[0].bytes = vec![0; 20];
    let missing = file_clip(r"D:\missing.jpg", None, false);
    let bad_effect = file_clip(r"D:\effect.jpg", Some(2), false);
    let external = file_clip(r"D:\external.jpg", None, false);
    let bad_type = file_clip(r"D:\type.jpg", None, false);
    let clips = vec![
        copy.clone(),
        qq.clone(),
        cut.clone(),
        absent_effect,
        text("旧文本"),
        Clip {
            formats: vec![model::FormatData {
                name: "PNG".into(),
                bytes: b"synthetic".to_vec(),
            }],
        },
        invalid,
        missing.clone(),
        bad_effect.clone(),
        external.clone(),
        bad_type.clone(),
    ];
    for (n, clip) in clips.iter().enumerate() {
        put_v0(&conn, &root.0, clip, 100 + n as i64);
    }
    // 重复记录中 QQ 的 touched 最大，不能按 id 或扫描顺序随便选一条。
    conn.execute("UPDATE items SET touched=500 WHERE id=?1", [qq.hash()])
        .unwrap();
    conn.execute("DELETE FROM formats WHERE item=?1", [missing.hash()])
        .unwrap();
    conn.execute(
        "UPDATE formats SET inline=NULL WHERE item=?1 AND name='Preferred DropEffect'",
        [bad_effect.hash()],
    )
    .unwrap();
    conn.execute(
        "UPDATE formats SET inline=NULL,blob=?2 WHERE item=?1",
        params![external.hash(), blob(&root.0, &external.formats[0].bytes)],
    )
    .unwrap();
    conn.execute(
        "UPDATE formats SET inline='invalid storage type' WHERE item=?1",
        [bad_type.hash()],
    )
    .unwrap();
    drop(conn);
    if start_version == 1 {
        stage_v1(&root.0);
    }
    let blobs = blob_snapshot(&root.0);
    let mut s = Storage::open(&root.0, &MASTER).unwrap();
    assert_eq!(version(&s.conn), migrations::VERSION);
    assert_eq!(s.list().unwrap().len(), clips.len());
    for (n, clip) in clips.iter().enumerate() {
        let (key, touched): (String, i64) = s
            .conn
            .query_row(
                "SELECT dedup_key,touched FROM items WHERE id=?1",
                [clip.hash()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(key, if n < 4 { clip.dedup_key() } else { clip.hash() });
        assert_eq!(touched, if n == 1 { 500 } else { 100 + n as i64 });
    }
    assert_ne!(copy.dedup_key(), cut.dedup_key());
    assert_eq!(
        s.conn
            .query_row(
                "SELECT count(*) FROM items WHERE dedup_key=?1",
                [copy.dedup_key()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        3
    );
    let unique: i64 = s
        .conn
        .query_row(
            "SELECT \"unique\" FROM pragma_index_list('items') WHERE name='items_dedup_key'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unique, 0);
    assert_eq!(blob_snapshot(&root.0), blobs);
    let backup_path = root.0.join(format!("history.db.bak-v{start_version}"));
    let backup = connect(&backup_path);
    assert_eq!(version(&backup), start_version);
    assert!(!columns(&backup).contains(&"dedup_key".into()));
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM items", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        clips.len()
    );
    drop(backup);
    let backup_paths = backups(&root.0);
    let original = s.load(&qq.hash()).unwrap().formats;
    s.conn.execute("UPDATE items SET used_at=0", []).unwrap();
    s.save(&copy).unwrap();
    assert_eq!(s.list().unwrap().len(), clips.len());
    assert_eq!(s.list().unwrap()[0].id, qq.hash());
    assert!(s.list().unwrap()[0].used_at > 0);
    assert_eq!(s.load(&qq.hash()).unwrap().formats, original);
    let older: (i64, i64) = s
        .conn
        .query_row(
            "SELECT touched,used_at FROM items WHERE id=?1",
            [copy.hash()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(older, (100, 0));
    assert_eq!(blob_snapshot(&root.0), blobs);
    drop(s);
    let reopened =
        Storage::open_with_hook(&root.0, &MASTER, |_| panic!("当前版本不应再次执行迁移")).unwrap();
    assert_eq!(version(&reopened.conn), migrations::VERSION);
    assert_eq!(reopened.list().unwrap().len(), clips.len());
    assert_eq!(reopened.list().unwrap()[0].id, qq.hash());
    assert_eq!(backups(&root.0), backup_paths);
}
#[test]
fn v0_to_v2_backfills_dedup_keys_preserves_duplicates_and_reopens_without_migration() {
    check_dedup_migration(0);
}
#[test]
fn v1_to_v2_backfills_dedup_keys_preserves_duplicates_and_reopens_without_migration() {
    check_dedup_migration(1);
}
#[test]
fn v2_failure_rolls_back_dedup_column_index_and_version() {
    let root = Fixture::new();
    let (clips, _, _) = legacy(&root.0);
    stage_v1(&root.0);
    let error = Storage::open_with_hook(&root.0, &MASTER, |tx| {
        assert_eq!(version(tx), 2);
        assert!(columns(tx).contains(&"dedup_key".into()));
        bail!("测试 v2 回填后失败")
    })
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("已回滚") && error.contains("bak-v1"));
    let conn = connect(&root.0.join("history.db"));
    assert_eq!(version(&conn), 1);
    assert!(!columns(&conn).contains(&"dedup_key".into()));
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='items_dedup_key'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM items", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        clips.len()
    );
    drop(conn);
    assert_eq!(
        version(&Storage::open(&root.0, &MASTER).unwrap().conn),
        migrations::VERSION
    );
}

#[test]
fn migrated_legacy_library_accepts_latest_source_without_new_migration() {
    let fixture = Fixture::new();
    let (clips, _, _) = legacy(&fixture.0);
    let store = Mutex::new(Storage::open(&fixture.0, &MASTER).unwrap());
    assert!(store
        .lock()
        .unwrap()
        .list()
        .unwrap()
        .iter()
        .all(|row| row.source_exe.is_none() && row.source_title.is_none()));
    let source = crate::capture_source::CaptureSource {
        exe: Some("notepad.exe".into()),
        title: Some("合成迁移测试".into()),
    };
    assert!(
        !Storage::save_shared_with_source(&store, &clips[0], Some(&source))
            .unwrap()
            .inserted
    );
    drop(store);
    let reopened = Storage::open(&fixture.0, &MASTER).unwrap();
    let row = reopened
        .list()
        .unwrap()
        .into_iter()
        .find(|row| row.id == clips[0].hash())
        .unwrap();
    assert_eq!(row.source_exe, source.exe);
    assert_eq!(row.source_title, source.title);
    assert_eq!(
        reopened
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
            .unwrap(),
        4
    );
}
