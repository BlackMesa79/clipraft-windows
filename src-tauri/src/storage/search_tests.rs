use super::*;
use std::collections::BTreeSet;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MASTER: [u8; 32] = [84; 32];
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "t4-search-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> Storage {
        Storage::open(&self.0, &MASTER).unwrap()
    }
    fn connect(&self) -> Connection {
        let conn = Connection::open(self.0.join("history.db")).unwrap();
        conn.pragma_update(None, "key", format!("x'{}'", "54".repeat(32)))
            .unwrap();
        conn
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
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
fn query(s: &Storage, q: &str) -> Vec<String> {
    s.list_page(
        &ListFilter {
            query: Some(q.into()),
            ..Default::default()
        },
        0,
        PAGE_LIMIT,
    )
    .unwrap()
    .into_iter()
    .map(|i| i.id)
    .collect()
}
fn check(s: &Storage) {
    s.conn
        .execute(
            "INSERT INTO items_fts(items_fts) VALUES('integrity-check')",
            [],
        )
        .unwrap();
    let mismatches: i64 = s.conn.query_row("SELECT count(*) FROM items i LEFT JOIN items_fts_map m ON m.item_id=i.id LEFT JOIN items_fts f ON f.rowid=m.search_id WHERE f.rowid IS NULL OR i.search_text!=f.search_text OR i.search_pinyin!=f.search_pinyin", [], |r| r.get(0)).unwrap();
    assert_eq!(mismatches, 0);
    let counts: (i64,i64,i64) = s.conn.query_row("SELECT (SELECT count(*) FROM items),(SELECT count(*) FROM items_fts_map),(SELECT count(*) FROM items_fts)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(counts.0, counts.1);
    assert_eq!(counts.0, counts.2);
    assert!(s
        .conn
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}
// 仅供旧版迁移测试构造真实 v3/v2 结构，不让 v4 对象混进旧库夹具。
pub(super) fn remove_v4(conn: &Connection) {
    conn.execute_batch("DROP TRIGGER items_search_insert; DROP TRIGGER items_search_delete; DROP TRIGGER items_search_update;
        DROP TABLE items_fts; DROP TABLE items_fts_map; DROP INDEX items_link;
        ALTER TABLE items DROP COLUMN search_pinyin; ALTER TABLE items DROP COLUMN is_link; PRAGMA user_version=3;").unwrap();
}
#[test]
fn bundled_sqlcipher_actually_supports_trigram() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE VIRTUAL TABLE probe USING fts5(body,tokenize='trigram'); INSERT INTO probe VALUES('Hello你好世界');").unwrap();
    for q in ["ell", "你好世", "HEL"] {
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM probe WHERE probe MATCH ?1",
                [q],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}
#[test]
fn mixed_text_substrings_and_case() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text("Hello你好世界 MixedCase");
    s.save(&clip).unwrap();
    for q in ["ell", "HELLO", "你好世", "好世界", "xedca"] {
        assert_eq!(query(&s, q), [clip.hash()], "{q}");
    }
    assert!(query(&s, "absent").is_empty());
    check(&s);
}
#[test]
fn short_queries_escape_like_metacharacters() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text(r"你好 100% a_b c\d");
    s.save(&clip).unwrap();
    s.save(&text("unrelated abc")).unwrap();
    for q in ["你", "你好", "%", "_", r"\", "a_", "0%", r"\d"] {
        assert_eq!(query(&s, q), [clip.hash()], "{q}");
    }
}
#[test]
fn words_are_anded_across_original_and_pinyin_columns() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text("你好世界 Hello Rust");
    s.save(&clip).unwrap();
    s.save(&text("Hello only")).unwrap();
    for q in [" Hello\t你好 \n", "HELLO shi Rust", "nh 世界", "niha rust"] {
        assert_eq!(query(&s, q), [clip.hash()], "{q}");
    }
    assert!(query(&s, "Hello absent").is_empty());
}
#[test]
fn fts_syntax_is_always_literal() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text(r#"quote"here a*b NEAR(foo) OR -minus abc% x_y a\b """"#);
    s.save(&clip).unwrap();
    s.save(&text("unrelated")).unwrap();
    for q in [
        r#"quote"here"#,
        "a*b",
        "NEAR",
        "NEAR(foo)",
        "OR",
        "-minus",
        "*",
        "-",
        "abc%",
        "x_y",
        r"a\b",
        r#"""""#,
    ] {
        assert_eq!(query(&s, q), [clip.hash()], "{q}");
    }
    for q in ["NEAR(nothing)", r#"text:"absent""#, "a**", "NOT"] {
        assert!(query(&s, q).is_empty(), "{q}");
    }
}
#[test]
fn pinyin_full_initial_partial_and_mixed() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text("Hello你好世界");
    s.save(&clip).unwrap();
    for q in [
        "nihao",
        "nh",
        "nhsj",
        "niha",
        "HAOSHI",
        "hellonihao",
        "hellonh",
        "Hello 世界",
    ] {
        assert_eq!(query(&s, q), [clip.hash()], "{q}");
    }
}
#[test]
fn pinyin_size_is_bounded_and_ascii_needs_no_copy() {
    assert!(search::pinyin_text("abc%_123").is_empty());
    let p = search::pinyin_text(&format!("{}界", "好".repeat(2000)));
    assert_eq!(p, format!("{}\n{}", "hao".repeat(2000), "h".repeat(2000)));
}
#[test]
fn link_detection_requires_entire_http_url_and_text_kind() {
    for t in [
        "https://example.com",
        " \nHTTP://example.com/a?b=c#d\t",
        "https://例子.测试/你好",
    ] {
        assert!(search::is_link("text", t), "{t}");
    }
    for t in [
        "before https://example.com",
        "https://example.com after",
        "ftp://example.com",
        "https://exa mple.com",
        "https://",
        "http:example.com",
        "not-url",
        "https://example.com/\u{0}x",
    ] {
        assert!(!search::is_link("text", t), "{t:?}");
    }
    for kind in ["rich", "image", "files"] {
        assert!(!search::is_link(kind, "https://example.com"));
    }
}
#[test]
fn link_filter_combines_with_query() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text(" https://example.com/你好 ");
    s.save(&clip).unwrap();
    s.save(&text("prefix https://example.com")).unwrap();
    let rows = s
        .list_page(
            &ListFilter {
                kind: Some("link".into()),
                query: Some("EXAMPLE nihao".into()),
                ..Default::default()
            },
            0,
            100,
        )
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, clip.hash());
    check(&s);
}
#[test]
fn empty_query_does_not_filter_and_nonempty_does_not_match_image() {
    let f = Fixture::new();
    let mut s = f.open();
    s.save(&text("test")).unwrap();
    s.conn
        .execute(
            "INSERT INTO items(id,view,touched,kind) VALUES('image',?1,2,'image')",
            [display_json(&ItemView::default()).unwrap()],
        )
        .unwrap();
    assert_eq!(query(&s, " \n\t ").len(), 2);
    assert_eq!(query(&s, "test").len(), 1);
    assert!(query(&s, "图片").is_empty());
    check(&s);
}
#[test]
fn text_filter_includes_rich_and_other_kinds_remain_exact() {
    let f = Fixture::new();
    let mut s = f.open();
    s.save(&text("plain match")).unwrap();
    let mut rich = text("rich match");
    rich.formats.push(model::FormatData {
        name: "HTML Format".into(),
        bytes: b"<b>rich</b>".to_vec(),
    });
    s.save(&rich).unwrap();
    // Clip::kind 的富文本定义以实际模型为准，明确模拟已存储的 rich 类型。
    s.conn
        .execute("UPDATE items SET kind='rich' WHERE id=?1", [rich.hash()])
        .unwrap();
    assert_eq!(
        s.list_page(
            &ListFilter {
                kind: Some("text".into()),
                query: Some("match".into()),
                ..Default::default()
            },
            0,
            100
        )
        .unwrap()
        .len(),
        2
    );
    for kind in ["image", "files", "link"] {
        assert!(s
            .list_page(
                &ListFilter {
                    kind: Some(kind.into()),
                    query: Some("match".into()),
                    ..Default::default()
                },
                0,
                100
            )
            .unwrap()
            .is_empty());
    }
}
#[test]
fn search_filters_paging_and_pin_order_have_no_gaps() {
    let f = Fixture::new();
    let mut s = f.open();
    s.conn
        .execute("INSERT INTO groups VALUES(1,'组',0,0)", [])
        .unwrap();
    for i in 0..35 {
        let clip = text(&format!("你好 common {i:03}"));
        s.save(&clip).unwrap();
        s.conn
            .execute(
                "UPDATE items SET favorite=1,group_id=1,pinned=?2 WHERE id=?1",
                params![clip.hash(), i == 0],
            )
            .unwrap();
    }
    s.save(&text("common outside")).unwrap();
    let filter = ListFilter {
        query: Some("common nh".into()),
        kind: Some("text".into()),
        favorite_only: true,
        group_id: Some(1),
    };
    let all = s.list_page(&filter, 0, 100).unwrap();
    assert_eq!(all.len(), 35);
    assert_eq!(all[0].id, text("你好 common 000").hash());
    let mut paged = Vec::new();
    for offset in (0..40).step_by(7) {
        paged.extend(
            s.list_page(&filter, offset, 7)
                .unwrap()
                .into_iter()
                .map(|i| i.id),
        );
    }
    assert_eq!(paged, all.into_iter().map(|i| i.id).collect::<Vec<_>>());
    assert!(s.list_page(&filter, 0, 0).unwrap().is_empty());
}
#[test]
fn index_tracks_save_duplicate_update_delete_and_cleanup() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text("你好 original");
    s.save(&clip).unwrap();
    check(&s);
    s.save(&clip).unwrap();
    assert_eq!(query(&s, "nihao"), [clip.hash()]);
    check(&s);
    s.conn
        .execute(
            "UPDATE items SET search_text='changed',search_pinyin='' WHERE id=?1",
            [clip.hash()],
        )
        .unwrap();
    assert!(query(&s, "original").is_empty());
    assert_eq!(query(&s, "changed"), [clip.hash()]);
    check(&s);
    s.delete(&[clip.hash()]).unwrap();
    assert!(query(&s, "changed").is_empty());
    check(&s);
    for i in 0..6 {
        s.save(&text(&format!("清理 cleanup {i}"))).unwrap();
    }
    let policy = RetentionPolicy {
        max_items: 2,
        max_bytes: 1_000_000,
        max_days: 0,
    };
    assert_eq!(s.preview_cleanup(&policy, unix_millis()).unwrap(), 4);
    check(&s);
    assert_eq!(s.cleanup(&policy, unix_millis()).unwrap().deleted_items, 4);
    assert_eq!(query(&s, "cleanup").len(), 2);
    check(&s);
}
#[test]
fn index_rollback_and_vacuum_keep_mapping_stable() {
    let f = Fixture::new();
    let mut s = f.open();
    for i in 0..8 {
        s.save(&text(&format!("stable 测试 {i}"))).unwrap();
    }
    s.delete(&[text("stable 测试 2").hash(), text("stable 测试 4").hash()])
        .unwrap();
    let before = query(&s, "stable");
    let tx = s.conn.transaction().unwrap();
    tx.execute("DELETE FROM items", []).unwrap_err(); // formats 外键保护整个语句，包括触发器。
    tx.rollback().unwrap();
    check(&s);
    assert_eq!(query(&s, "stable"), before);
    // 主动让隐式 rowid 与 FTS 编号分离，不依赖当前 SQLite 是否在 VACUUM 中重编号。
    s.conn
        .execute("UPDATE items SET rowid=rowid+10000", [])
        .unwrap();
    let mapping = |s: &Storage| {
        s.conn
            .prepare("SELECT item_id,search_id FROM items_fts_map ORDER BY item_id")
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let before_mapping = mapping(&s);
    s.conn.execute_batch("VACUUM").unwrap();
    assert_eq!(mapping(&s), before_mapping);
    check(&s);
    assert_eq!(query(&s, "stable"), before);
    drop(s);
    let s = f.open();
    check(&s);
    assert_eq!(mapping(&s), before_mapping);
    assert_eq!(query(&s, "ceshi"), before);
}
#[test]
fn v3_backfill_backup_and_reopen() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text(" https://example.com/你好 ");
    s.save(&clip).unwrap();
    remove_v4(&s.conn);
    drop(s);
    let s = f.open();
    check(&s);
    assert_eq!(query(&s, "nihao"), [clip.hash()]);
    assert_eq!(
        s.conn
            .query_row("SELECT is_link FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let backup = Connection::open(f.0.join("history.db.bak-v3")).unwrap();
    backup
        .pragma_update(None, "key", format!("x'{}'", "54".repeat(32)))
        .unwrap();
    assert_eq!(
        backup
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    drop(backup);
    drop(s);
    let s = f.open();
    check(&s);
    assert_eq!(query(&s, "nh"), [clip.hash()]);
    assert_eq!(
        fs::read_dir(&f.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with("history.db.bak"))
            .count(),
        1
    );
}
#[test]
fn v4_failure_rolls_back_all_schema_data_and_index_objects() {
    let f = Fixture::new();
    let mut s = f.open();
    s.save(&text("你好 rollback")).unwrap();
    remove_v4(&s.conn);
    drop(s);
    let err = Storage::open_with_hook(&f.0, &MASTER, |_| bail!("合成 v4 提交失败"))
        .err()
        .unwrap();
    assert!(err.to_string().contains("已回滚"));
    let c = f.connect();
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(c.query_row("SELECT count(*) FROM sqlite_master WHERE name LIKE 'items_fts%' OR name LIKE 'items_search_%' OR name='items_link'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(c.prepare("SELECT search_pinyin FROM items").is_err());
    drop(c);
    let s = f.open();
    check(&s);
    assert_eq!(query(&s, "nihao").len(), 1);
    assert_eq!(
        fs::read_dir(&f.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with("history.db.bak-v3"))
            .count(),
        2
    );
}
fn seed(s: &mut Storage, count: usize) {
    let tx = s.conn.transaction().unwrap();
    for i in 0..count {
        let content = format!(
            "项目记录 {i:05} Hello你好世界 Rust clipboard 文件路径 D:/项目/报告-{i}.txt 100% a_b"
        );
        tx.execute(
            "INSERT INTO items(id,view,touched,search_text,search_pinyin) VALUES(?1,?2,?3,?4,?5)",
            params![
                format!("seed-{i:05}"),
                display_json(&ItemView {
                    id: format!("seed-{i:05}"),
                    summary: content.clone(),
                    formats: vec!["CF_UNICODETEXT".into()],
                    ..Default::default()
                })
                .unwrap(),
                i as i64,
                content,
                search::pinyin_text(&content)
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}
#[test]
fn small_search_dataset_smoke() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&mut s, 100);
    check(&s);
    assert_eq!(query(&s, "你好 rust").len(), 100);
    assert_eq!(query(&s, "00042").len(), 1);
}
#[test]
#[ignore = "5000 条搜索性能，需单独运行以避免构建负载干扰"]
fn search_5000_benchmark() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&mut s, 5000);
    check(&s);
    println!(
        "SQLite {} / 5000 encrypted rows / limit 100",
        s.conn
            .query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))
            .unwrap()
    );
    for q in [
        "你好世界",
        "CLIPBOARD",
        "niha",
        "nhsj",
        "你",
        "你好",
        "%",
        "你好 rust",
        "04999",
        "absent",
    ] {
        let filter = ListFilter {
            query: Some(q.into()),
            ..Default::default()
        };
        let mut times = Vec::new();
        let mut count = 0;
        for _ in 0..31 {
            let start = Instant::now();
            count = s.list_page(&filter, 0, 100).unwrap().len();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let first = times[0];
        times.sort_by(f64::total_cmp);
        println!(
            "query={q:?} first_ms={first:.3} median_ms={:.3} max_ms={:.3} rows={count}",
            times[15], times[30]
        );
    }
}

#[test]
fn file_paths_names_and_pinyin_are_searchable_after_dedup() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = Clip {
        formats: vec![model::FormatData {
            name: "CF_HDROP".into(),
            bytes: model::encode_hdrop(&[r"D:\项目\你好.txt".into()]),
        }],
    };
    s.save(&clip).unwrap();
    check(&s);
    let mut extra = clip.clone();
    extra.formats.push(model::FormatData {
        name: "HTML Format".into(),
        bytes: b"extra".to_vec(),
    });
    s.save(&extra).unwrap();
    check(&s);
    for q in ["你好", "nihao", ".txt", r"D:\项目"] {
        assert_eq!(query(&s, q), [clip.hash()], "{q}");
    }
}
#[test]
fn batch_delete_vacuum_preserves_search_index_for_survivors() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&mut s, 205);
    let report = s
        .delete(&(0..201).map(|i| format!("seed-{i:05}")).collect::<Vec<_>>())
        .unwrap();
    assert_eq!(report.deleted_items, 201);
    assert!(report.vacuumed);
    assert!(report.maintenance_errors.is_empty());
    check(&s);
    let survivors: Vec<_> = (201..205).rev().map(|i| format!("seed-{i:05}")).collect();
    assert_eq!(query(&s, "nihao"), survivors);
    assert_eq!(query(&s, "nhsj"), survivors);
    assert!(query(&s, "00042").is_empty());
    drop(s);
    let mut s = f.open();
    check(&s);
    assert_eq!(query(&s, "你好 rust"), survivors);
    let clip = text("你好 post-vacuum survivor");
    s.save(&clip).unwrap();
    s.delete(&["seed-00202".into()]).unwrap();
    assert_eq!(query(&s, "post-vacuum"), [clip.hash()]);
    assert!(query(&s, "00202").is_empty());
    check(&s);
}
#[test]
fn migration_backfills_all_kinds_without_indexing_image_content() {
    let f = Fixture::new();
    let mut s = f.open();
    for (value, kind) in [
        ("你好 rich", "rich"),
        ("D:/你好.txt", "files"),
        ("", "image"),
        ("hello plain", "text"),
    ] {
        let clip = text(value);
        s.save(&clip).unwrap();
        s.conn
            .execute(
                "UPDATE items SET kind=?2 WHERE id=?1",
                params![clip.hash(), kind],
            )
            .unwrap();
    }
    remove_v4(&s.conn);
    drop(s);
    let s = f.open();
    check(&s);
    assert_eq!(query(&s, "nihao").len(), 2);
    assert_eq!(query(&s, "hello").len(), 1);
    assert_eq!(query(&s, "").len(), 4);
}

// AC-1：快路径与锁外准备后的复查都只更新使用信息，不重建或复制索引。
#[test]
fn duplicate_fast_path_and_prepared_recheck_preserve_search_fields() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text("https://example.com/你好/duplicate");
    let prepared = PreparedSave::new(&clip, SaveIdentity::new(&clip), &MASTER).unwrap();
    s.save(&clip).unwrap();
    s.conn
        .execute("UPDATE items SET used_at=0 WHERE id=?1", [clip.hash()])
        .unwrap();
    let snapshot = |s: &Storage| {
        s.conn
            .query_row(
                "SELECT m.search_id,i.search_text,i.search_pinyin,i.is_link,i.touched FROM items i JOIN items_fts_map m ON m.item_id=i.id WHERE i.id=?1",
                [clip.hash()],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, bool>(3)?, r.get::<_, i64>(4)?)),
            )
            .unwrap()
    };
    let before = snapshot(&s);
    assert!(before.3);
    s.conn.execute_batch("CREATE TRIGGER reject_search_rewrite BEFORE UPDATE OF search_text,search_pinyin ON items BEGIN SELECT RAISE(ABORT,'去重不得重写搜索内容'); END;").unwrap();
    for _ in 0..4 {
        assert!(!s.save_with_outcome(&clip).unwrap());
    }
    let outcome = s.commit_prepared(prepared).unwrap();
    assert!(!outcome.inserted);
    assert_eq!(outcome.id, clip.hash());
    let after = snapshot(&s);
    assert_eq!(
        (&after.0, &after.1, &after.2, after.3),
        (&before.0, &before.1, &before.2, before.3)
    );
    assert_eq!(after.4, before.4 + 5);
    assert!(s
        .conn
        .query_row(
            "SELECT used_at>0 FROM items WHERE id=?1",
            [clip.hash()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(query(&s, "duplicate nh"), [clip.hash()]);
    check(&s);
}

// AC-1：覆盖 v4 每批 128 行回填的边界，并与 commit_prepared 的正常写入对照。
#[test]
fn v3_backfill_matches_new_writes_across_batch_boundary() {
    let migrated_fixture = Fixture::new();
    let fresh_fixture = Fixture::new();
    let mut migrated = migrated_fixture.open();
    let mut fresh = fresh_fixture.open();
    for i in 0..130 {
        let content = match i % 4 {
            0 => format!("https://example.com:8080/你好/sample-{i:03}"),
            1 => format!("Hello你好 Rust sample-{i:03}"),
            2 => format!("a😀b e\u{301}x sample-{i:03}"),
            _ => format!("Straße İSTANBUL sample-{i:03}"),
        };
        let clip = text(&content);
        migrated.save(&clip).unwrap();
        fresh.save(&clip).unwrap();
    }
    remove_v4(&migrated.conn);
    drop(migrated);
    let migrated = migrated_fixture.open();
    let fields = |s: &Storage| {
        s.conn
            .prepare("SELECT id,search_text,search_pinyin,is_link FROM items ORDER BY id")
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(fields(&migrated), fields(&fresh));
    for kind in [None, Some("text"), Some("link")] {
        for q in [
            "sample",
            "nihao",
            "nh",
            "HELLO 你好",
            "a😀b",
            "e\u{301}x",
            "sample-129",
            "absent",
        ] {
            let filter = ListFilter {
                kind: kind.map(str::to_owned),
                query: Some(q.into()),
                ..Default::default()
            };
            let ids = |s: &Storage| {
                let mut ids: Vec<_> = s
                    .list_page(&filter, 0, PAGE_LIMIT)
                    .unwrap()
                    .into_iter()
                    .map(|r| r.id)
                    .collect();
                ids.sort();
                ids
            };
            assert_eq!(ids(&migrated), ids(&fresh), "kind={kind:?}, query={q:?}");
        }
    }
    assert_eq!(query(&migrated, "sample").len(), 130);
    assert_eq!(query(&migrated, "nihao").len(), 66);
    check(&migrated);
    check(&fresh);
}

// AC-2：两字符 LIKE 与恰好三字符 FTS 条件必须同时满足。
#[test]
fn mixed_two_and_three_char_words_require_both() {
    let f = Fixture::new();
    let mut s = f.open();
    let both = text("cat ab 你好");
    for clip in [
        &both,
        &text("cat only"),
        &text("ab only"),
        &text("你好 only"),
    ] {
        s.save(clip).unwrap();
    }
    for q in ["cat ab", "ab cat", "CAT 你好", "cat ab 你好"] {
        assert_eq!(query(&s, q), [both.hash()], "{q:?}");
    }
    assert!(query(&s, "cat zz").is_empty());
}

// AC-2：Rust chars 和 trigram 均按 Unicode 标量计数，而不是 UTF-8 字节或字素簇。
#[test]
fn unicode_trigram_counts_scalars_including_emoji_and_combining_marks() {
    let f = Fixture::new();
    let mut s = f.open();
    for word in ["a😀b", "e\u{301}x", "👩\u{200d}💻"] {
        assert_eq!(word.chars().count(), 3);
        let clip = text(&format!("left {word} right"));
        s.save(&clip).unwrap();
        assert_eq!(query(&s, word), [clip.hash()], "{word:?}");
    }
    assert!(query(&s, "éx").is_empty(), "不隐式进行 NFC/NFD 归一化");
    assert_eq!(query(&s, "😀b"), [text("left a😀b right").hash()]);
    assert!(query(&s, "a😀c").is_empty());
    check(&s);
}

// AC-2：split_whitespace 同时接受全角空格和制表符；纯 Unicode 空白不筛选。
#[test]
fn unicode_whitespace_separates_words_and_blank_query_preserves_order() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text("Hello你好 marker");
    s.save(&clip).unwrap();
    s.save(&text("Hello only")).unwrap();
    for q in [
        "HELLO\u{3000}你好",
        "你好\tHELLO",
        "\u{3000}Hello\t你好\r\n",
    ] {
        assert_eq!(query(&s, q), [clip.hash()], "{q:?}");
    }
    let all = query(&s, "");
    for q in ["\u{3000}", "\t\u{3000}\r\n", "\u{a0}\u{2003}"] {
        assert_eq!(query(&s, q), all, "{q:?}");
    }
}

// AC-2：长单词与多个 FTS 单词分别经过真实查询，避免仅验证 SQL 构造。
#[test]
fn two_thousand_character_single_word_and_fts_word_list_are_searchable() {
    let f = Fixture::new();
    let mut s = f.open();
    let word = format!("{}xyz", "a".repeat(1997));
    let clip = text(&format!("prefix {word} suffix cat"));
    s.save(&clip).unwrap();
    s.save(&text(&format!("{}xyq cat", "a".repeat(1997))))
        .unwrap();
    assert_eq!(word.chars().count(), 2000);
    assert_eq!(query(&s, &word), [clip.hash()]);
    let words = "xyz ".repeat(500);
    assert_eq!(words.chars().count(), 2000);
    assert_eq!(query(&s, &words), [clip.hash()]);
    assert!(query(&s, &"z".repeat(2000)).is_empty());
}

// AC-2：大量重复短词去重后仍可查询，不触发 SQLite 表达式深度上限。
#[test]
fn two_thousand_character_short_word_list_should_remain_searchable() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text("a");
    s.save(&clip).unwrap();
    let words = "a ".repeat(1000);
    assert_eq!(words.chars().count(), 2000);
    assert_eq!(query(&s, &words), [clip.hash()]);
}

// AC-2：记录 SQLite 默认 Unicode 折叠的实际边界，不要求语言学等价。
#[test]
fn unicode_case_folding_documents_german_turkish_and_short_like_behavior() {
    let f = Fixture::new();
    let mut s = f.open();
    let german = text("Straße");
    let turkish = text("İSTANBUL");
    let umlauts = text("ÄÖÜ");
    for clip in [&german, &turkish, &umlauts] {
        s.save(clip).unwrap();
    }
    for q in ["straße", "STRAßE", "ß"] {
        assert_eq!(query(&s, q), [german.hash()], "{q:?}");
    }
    for q in ["STRASSE", "ẞ", "istanbul", "is", "äö"] {
        assert!(query(&s, q).is_empty(), "{q:?}");
    }
    for q in ["İST", "STANBUL", "İs"] {
        assert_eq!(query(&s, q), [turkish.hash()], "{q:?}");
    }
    assert_eq!(query(&s, "äöü"), [umlauts.hash()]);
}

// AC-3：换行也是 trigram 的字符，不能将全拼末尾与首字母开头直接拼接。
#[test]
fn pinyin_separator_prevents_concatenated_boundary_matches() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text("你好");
    s.save(&clip).unwrap();
    assert_eq!(search::pinyin_text("你好"), "nihao\nnh");
    for q in ["aon", "onh", "aonh", "nihaonh"] {
        assert!(query(&s, q).is_empty(), "{q:?} 不应跨换行拼接命中");
    }
    for q in ["hao nh", "hao\nnh"] {
        assert_eq!(query(&s, q), [clip.hash()], "空白分词按 AND 匹配：{q:?}");
    }
}

// AC-3：首字母只缩写汉字，英文、数字、符号、空白均原样保留。
#[test]
fn mixed_pinyin_initials_preserve_non_han_characters() {
    let value = "Hi你-好42 Rust界";
    assert_eq!(
        search::pinyin_text(value),
        "Hini-hao42 Rustjie\nHin-h42 Rustj"
    );
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text(value);
    s.save(&clip).unwrap();
    for q in ["Hin-h42", "Rustj", "HINI-HAO42", "hin-h42 rustj"] {
        assert_eq!(query(&s, q), [clip.hash()], "{q:?}");
    }
    assert!(query(&s, "hnh42").is_empty(), "英文并不缩写为首字母 h");
}

// AC-3：限制针对可转拼音的汉字数量；第 2001 个汉字起截断拼音，不截断原文。
#[test]
fn pinyin_limit_counts_han_and_keeps_original_suffix_searchable() {
    let value = format!("{}X界After", "好".repeat(2000));
    let expected = format!("{}X\n{}X", "hao".repeat(2000), "h".repeat(2000));
    assert_eq!(search::pinyin_text(&value), expected);
    assert_eq!(
        search::pinyin_text(&format!("{}界", "好A".repeat(1999))),
        format!("{}jie\n{}j", "haoA".repeat(1999), "hA".repeat(1999))
    );
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text(&value);
    s.save(&clip).unwrap();
    for q in ["界After", "haox", "hx"] {
        assert_eq!(query(&s, q), [clip.hash()], "{q:?}");
    }
    assert!(query(&s, "jieAfter").is_empty());
    assert_eq!(
        s.conn
            .query_row("SELECT search_pinyin FROM items", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        expected
    );
    check(&s);
}

// AC-4：合法尾标点属于 URL，不自动剥离；格式合法不代表域名可达。
#[test]
fn link_detection_covers_ports_ipv6_empty_hosts_and_trailing_punctuation() {
    let cases = [
        ("http://example.com:8080/path", true),
        ("https://example.com:65535/", true),
        ("http://[::1]:8080/path", true),
        ("https://[2001:db8::1]/", true),
        ("https://例子.测试/你好", true),
        ("https://example.com/path。", true),
        ("https://example.com/path,", true),
        ("https://example.com。", true),
        ("http://", false),
        ("http://:8080", false),
        ("http://example.com:65536/", false),
        ("http://example.com:port/", false),
        ("http://[::1", false),
        ("http://::1/", false),
        ("https://example.com/path 。", false),
        ("http://[::1]。", false),
    ];
    let f = Fixture::new();
    let mut s = f.open();
    let mut expected = Vec::new();
    for (value, is_link) in cases {
        assert_eq!(search::is_link("text", value), is_link, "{value:?}");
        let clip = text(value);
        s.save(&clip).unwrap();
        if is_link {
            expected.push(clip.hash());
        }
    }
    let mut actual: Vec<_> = s
        .list_page(
            &ListFilter {
                kind: Some("link".into()),
                ..Default::default()
            },
            0,
            PAGE_LIMIT,
        )
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    check(&s);
}

// AC-4：完整内容包含空白及附加文字，不应因 16KiB 搜索文本截断而误判为纯链接。
#[test]
fn link_filter_must_validate_full_clipboard_text_beyond_search_limit() {
    let prefix = "https://example.com/";
    let value = format!(
        "{prefix}{} trailing prose",
        "a".repeat(16 * 1024 - prefix.len())
    );
    assert!(!search::is_link("text", &value));
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text(&value);
    s.save(&clip).unwrap();
    let links = s
        .list_page(
            &ListFilter {
                kind: Some("link".into()),
                query: Some("example".into()),
                ..Default::default()
            },
            0,
            PAGE_LIMIT,
        )
        .unwrap();
    assert!(
        links.is_empty(),
        "完整剪贴板内容不是单个 URL，不应进入链接筛选"
    );
}

// AC-5：为每个条件单独构造反例，防止遗漏 favorite 或 group 条件仍误通过。
#[test]
fn query_favorite_group_and_kind_each_exclude_independent_mismatches() {
    let f = Fixture::new();
    let s = f.open();
    s.conn
        .execute_batch("INSERT INTO groups VALUES(1,'目标组',0,0),(2,'其他组',0,1)")
        .unwrap();
    for (id, content, kind, favorite, group) in [
        ("plain", "你好 target", "text", true, 1),
        ("rich", "你好 target", "rich", true, 1),
        ("wrong-favorite", "你好 target", "text", false, 1),
        ("wrong-group", "你好 target", "text", true, 2),
        ("wrong-query", "你好 absent", "text", true, 1),
        ("wrong-kind", "你好 target", "files", true, 1),
    ] {
        s.conn.execute("INSERT INTO items(id,view,touched,search_text,search_pinyin,kind,favorite,group_id) VALUES(?1,?2,1,?3,?4,?5,?6,?7)", params![id,display_json(&ItemView::default()).unwrap(),content,search::pinyin_text(content),kind,favorite,group]).unwrap();
    }
    let filter = ListFilter {
        query: Some("target nh".into()),
        kind: Some("text".into()),
        favorite_only: true,
        group_id: Some(1),
    };
    let ids: Vec<_> = s
        .list_page(&filter, 0, PAGE_LIMIT)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["plain", "rich"]);
    check(&s);
}

// AC-5：置顶项来自插入顺序中间，第二页跨越置顶/非置顶边界，并验证同 touched 的 id 排序。
#[test]
fn filtered_paging_straddles_pinned_boundary_with_tied_timestamps() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&mut s, 12);
    s.conn.execute_batch("INSERT INTO groups VALUES(1,'分页组',0,0);
        UPDATE items SET favorite=1,group_id=1,touched=7;
        UPDATE items SET pinned=1 WHERE id IN ('seed-00002','seed-00005','seed-00008','seed-00011');").unwrap();
    let filter = ListFilter {
        query: Some("你好 nh".into()),
        kind: Some("text".into()),
        favorite_only: true,
        group_id: Some(1),
    };
    let expected: Vec<_> = [2, 5, 8, 11, 0, 1, 3, 4, 6, 7, 9, 10]
        .into_iter()
        .map(|i| format!("seed-{i:05}"))
        .collect();
    for page_size in [1, 3, 4, 5, 7] {
        let mut actual = Vec::new();
        for offset in (0..12).step_by(page_size) {
            actual.extend(
                s.list_page(&filter, offset, page_size)
                    .unwrap()
                    .into_iter()
                    .map(|r| r.id),
            );
        }
        assert_eq!(actual, expected, "page_size={page_size}");
        assert_eq!(actual.iter().collect::<BTreeSet<_>>().len(), 12);
        assert!(s.list_page(&filter, 12, page_size).unwrap().is_empty());
    }
    check(&s);
}

#[test]
fn query_limit_uses_first_32_case_insensitive_unique_words_with_and() {
    let f = Fixture::new();
    let mut s = f.open();
    for long in [false, true] {
        let mut words = vec!["Ax".to_string()];
        words.extend((0..31).map(|i| {
            if long && i % 2 == 0 {
                format!("term{i:02}")
            } else {
                format!("{i:02}")
            }
        }));
        let matched = text(&words.join(" "));
        s.save(&matched).unwrap();
        let partial = text(&words[..31].join(" "));
        s.save(&partial).unwrap();
        // 大小写重复词不占额度；第 32 个唯一词仍参与 AND，第 33 个不参与。
        let query_text = format!("{} {} absent", "Ax aX ".repeat(350), words[1..].join(" "));
        let results = query(&s, &query_text);
        assert!(results.contains(&matched.hash()));
        assert!(!results.contains(&partial.hash()));
        assert!(query(&s, &format!("absent {}", words.join(" "))).is_empty());
    }
}

#[test]
fn link_length_limit_counts_full_utf8_bytes_before_trimming() {
    let prefix = "https://example.com/";
    let at_limit = format!(
        "{prefix}{}",
        "a".repeat(search::LINK_TEXT_LIMIT - prefix.len())
    );
    assert!(search::is_link("text", &at_limit));
    assert!(!search::is_link("text", &format!("{at_limit} ")));
    assert!(!search::is_link("text", &format!("{at_limit}中")));
}

#[test]
fn v4_link_backfill_uses_full_inline_and_blob_with_safe_fallback() {
    let f = Fixture::new();
    let mut s = f.open();
    let mut expected = Vec::new();
    for (label, value, external, missing, expected_link) in [
        (
            "inline",
            format!("https://example.com/{} tail", "a".repeat(17 * 1024)),
            false,
            false,
            false,
        ),
        (
            "blob",
            format!("https://example.com/{} tail", "b".repeat(40 * 1024)),
            true,
            false,
            false,
        ),
        (
            "valid-blob",
            "https://example.com/valid".into(),
            true,
            false,
            true,
        ),
        (
            "missing-short",
            "https://example.com/missing".into(),
            true,
            true,
            true,
        ),
        (
            "missing-long",
            format!("https://example.com/{} tail", "c".repeat(40 * 1024)),
            true,
            true,
            false,
        ),
        (
            "missing-format",
            "https://example.com/no-format".into(),
            false,
            true,
            true,
        ),
    ] {
        let clip = text(&value);
        s.save(&clip).unwrap();
        if external {
            let blob = s.write_blob(clip.get("CF_UNICODETEXT").unwrap()).unwrap();
            s.conn.execute("UPDATE formats SET inline=NULL,blob=?2 WHERE item=?1 AND name='CF_UNICODETEXT'", params![clip.hash(), blob]).unwrap();
            if missing {
                fs::remove_file(s.blobs.join(blob)).unwrap();
            }
        } else if missing {
            s.conn
                .execute("DELETE FROM formats WHERE item=?1", [clip.hash()])
                .unwrap();
        }
        // 短前缀也不能覆盖可读取的完整内容：迁移必须优先使用载荷。
        if !missing && !expected_link {
            s.conn
                .execute(
                    "UPDATE items SET search_text='https://example.com/prefix' WHERE id=?1",
                    [clip.hash()],
                )
                .unwrap();
        }
        expected.push((label, clip.hash(), expected_link));
    }
    remove_v4(&s.conn);
    drop(s);
    let s = f.open();
    for (label, id, expected_link) in expected {
        let actual: bool = s
            .conn
            .query_row("SELECT is_link FROM items WHERE id=?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(actual, expected_link, "{label}");
    }
    check(&s);
}
