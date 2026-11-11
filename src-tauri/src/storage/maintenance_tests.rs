use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../.test-data")
                .join(format!(
                    "t2-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                )),
        )
    }
    fn open(&self) -> Storage {
        Storage::open(&self.0, &[72; 32]).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
const NOW: i64 = 100 * 86_400_000;
fn add(s: &Storage, id: &str, touched: i64, size: i64, used: i64) {
    let json = display_json(&ItemView {
        id: id.into(),
        ..Default::default()
    })
    .unwrap();
    s.conn
        .execute(
            "INSERT INTO items(id,view,touched,size,created,used_at) VALUES(?1,?2,?3,?4,0,?5)",
            params![id, json, touched, size, used],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO formats(item,name,inline) VALUES(?1,'test',X'01')",
            [id],
        )
        .unwrap();
}
fn seed(s: &Storage, count: usize) {
    for i in 0..count {
        add(s, &format!("item-{i}"), i as i64, 10, NOW);
    }
}
fn policy(count: u32, bytes: u64, days: u32) -> RetentionPolicy {
    RetentionPolicy {
        max_items: count,
        max_bytes: bytes,
        max_days: days,
    }
}
fn ids(s: &Storage) -> Vec<String> {
    s.list_page(&ListFilter::default(), 0, 500)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect()
}

fn bitmap_clip() -> Clip {
    use std::io::Cursor;
    let image = image::RgbaImage::from_pixel(40, 30, image::Rgba([21, 42, 84, 128]));
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    Clip {
        formats: vec![model::FormatData {
            name: "PNG".into(),
            bytes: png.into_inner(),
        }],
    }
}

fn assert_loaded(s: &Storage, clip: &Clip) {
    let loaded = s.load(&clip.hash()).unwrap();
    assert_eq!(loaded.formats.len(), clip.formats.len());
    for format in &clip.formats {
        assert_eq!(loaded.get(&format.name), Some(format.bytes.as_slice()));
    }
}

fn page_counts(s: &Storage) -> (u64, u64) {
    (
        s.conn
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap(),
        s.conn
            .pragma_query_value(None, "freelist_count", |r| r.get(0))
            .unwrap(),
    )
}

#[test]
fn retention_defaults_and_validation() {
    let p = RetentionPolicy::default();
    assert_eq!(p, policy(1000, 1024 * 1024 * 1024, 30));
    assert!(validate_retention_policy(&p).is_ok());
    assert!(validate_retention_policy(&policy(0, 1, 0)).is_err());
    assert!(validate_retention_policy(&policy(1, 0, 0)).is_err());
    assert!(validate_retention_policy(&policy(1, u64::MAX, 0)).is_err());
    assert!(validate_retention_policy(&policy(u32::MAX, i64::MAX as u64, u32::MAX)).is_ok());
}
#[test]
fn delete_missing_and_repeated_ids_count_actual_rows() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 1);
    assert_eq!(s.delete(&[]).unwrap().deleted_items, 0);
    assert_eq!(s.delete(&["absent".into()]).unwrap().deleted_items, 0);
    let report = s
        .delete(&["item-0".into(), "item-0".into(), "absent".into()])
        .unwrap();
    assert_eq!(report.deleted_items, 1);
    assert!(ids(&s).is_empty());
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM formats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn shared_thumbnail_is_reclaimed_only_after_last_record() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 2);
    let blob = s.write_blob(b"shared thumbnail").unwrap();
    s.conn
        .execute("UPDATE items SET thumbnail=?1", [&blob])
        .unwrap();
    assert_eq!(s.delete(&["item-0".into()]).unwrap().reclaimed_blobs, 0);
    assert!(s.blobs.join(&blob).is_file());
    assert_eq!(s.read_thumbnail(&blob).unwrap(), b"shared thumbnail");
    assert_eq!(s.delete(&["item-1".into()]).unwrap().reclaimed_blobs, 1);
    assert!(!s.blobs.join(blob).exists());
}
#[test]
fn cross_column_references_and_duplicate_candidates_are_preserved() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 3);
    let blob = s.write_blob(b"shared across all three columns").unwrap();
    s.conn
        .execute(
            "UPDATE items SET thumbnail=?1,preview=?1 WHERE id='item-0'",
            [&blob],
        )
        .unwrap();
    s.conn
        .execute("UPDATE items SET preview=?1 WHERE id='item-1'", [&blob])
        .unwrap();
    s.conn
        .execute(
            "UPDATE formats SET blob=?1,inline=NULL WHERE item='item-2'",
            [&blob],
        )
        .unwrap();
    assert_eq!(s.delete(&["item-0".into()]).unwrap().reclaimed_blobs, 0);
    assert_eq!(s.delete(&["item-1".into()]).unwrap().reclaimed_blobs, 0);
    assert!(s.blobs.join(&blob).exists());
    assert_eq!(s.delete(&["item-2".into()]).unwrap().reclaimed_blobs, 1);
}
#[test]
fn delete_rollback_leaves_formats_items_and_blob_intact() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 2);
    let blob = s.write_blob(b"rollback").unwrap();
    s.conn
        .execute("UPDATE items SET thumbnail=?1", [&blob])
        .unwrap();
    s.conn.execute_batch("CREATE TRIGGER reject_delete BEFORE DELETE ON items WHEN OLD.id='item-1' BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(s.delete(&["item-0".into(), "item-1".into()]).is_err());
    assert_eq!(ids(&s).len(), 2);
    assert!(s.load("item-0").is_ok());
    assert!(s.blobs.join(blob).exists());
}
#[test]
fn failed_blob_deletion_is_reported_and_retried_on_scan() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 1);
    let blob = s.write_blob(b"locked synthetic blob").unwrap();
    s.conn
        .execute("UPDATE items SET thumbnail=?1", [&blob])
        .unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(s.blobs.join(&blob))
        .unwrap();
    let r = s.delete(&["item-0".into()]).unwrap();
    assert_eq!(r.deleted_items, 1);
    assert_eq!(r.failed_blobs, vec![blob.clone()]);
    assert!(s.blobs.join(&blob).exists());
    drop(locked);
    assert_eq!(s.scan_orphan_blobs().unwrap().reclaimed_blobs, 1);
}
#[test]
fn orphan_scan_only_removes_hash_orphans_and_tmp_regular_files() {
    let f = Fixture::new();
    let s = f.open();
    seed(&s, 3);
    let orphan = s.write_blob(b"orphan").unwrap();
    let refs = [
        s.write_blob(b"format").unwrap(),
        s.write_blob(b"thumb").unwrap(),
        s.write_blob(b"preview").unwrap(),
    ];
    s.conn
        .execute("UPDATE formats SET blob=?1 WHERE item='item-0'", [&refs[0]])
        .unwrap();
    s.conn
        .execute(
            "UPDATE items SET thumbnail=?1 WHERE id='item-1'",
            [&refs[1]],
        )
        .unwrap();
    s.conn
        .execute("UPDATE items SET preview=?1 WHERE id='item-2'", [&refs[2]])
        .unwrap();
    let keep = ["README", "abc"];
    for name in &keep {
        fs::write(s.blobs.join(name), b"keep").unwrap();
    }
    let uppercase = "A".repeat(64);
    fs::write(s.blobs.join(&uppercase), b"keep").unwrap();
    fs::write(s.blobs.join("stale.tmp"), b"partial").unwrap();
    fs::create_dir(s.blobs.join("folder.tmp")).unwrap();
    fs::create_dir(s.blobs.join("f".repeat(64))).unwrap();
    let nested_hash = s.blobs.join("folder.tmp").join("e".repeat(64));
    let nested_temp = s.blobs.join("f".repeat(64)).join("nested.tmp");
    fs::write(&nested_hash, b"nested orphan").unwrap();
    fs::write(&nested_temp, b"nested temp").unwrap();
    let report = s.scan_orphan_blobs().unwrap();
    assert_eq!((report.reclaimed_blobs, report.removed_temps), (1, 1));
    assert!(!s.blobs.join(orphan).exists());
    for id in refs {
        assert!(s.blobs.join(id).exists());
    }
    assert!(s.blobs.join("README").is_file());
    assert!(s.blobs.join("abc").is_file());
    assert!(s.blobs.join(uppercase).is_file());
    assert!(s.blobs.join("folder.tmp").is_dir());
    assert_eq!(fs::read(nested_hash).unwrap(), b"nested orphan");
    assert_eq!(fs::read(nested_temp).unwrap(), b"nested temp");
    assert_eq!(s.scan_orphan_blobs().unwrap().reclaimed_blobs, 0);
}
#[test]
fn cleanup_equal_limits_keeps_all_and_one_over_removes_oldest() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 3);
    assert_eq!(s.cleanup(&policy(3, 30, 0), NOW).unwrap().deleted_items, 0);
    assert_eq!(s.cleanup(&policy(2, 30, 0), NOW).unwrap().deleted_items, 1);
    assert_eq!(ids(&s), vec!["item-2", "item-1"]);
}
#[test]
fn cleanup_days_uses_used_at_and_zero_disables_age_limit() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 4);
    s.conn
        .execute(
            "UPDATE items SET used_at=?1 WHERE id='item-0'",
            [NOW - 30 * 86_400_000 - 1],
        )
        .unwrap();
    s.conn
        .execute(
            "UPDATE items SET used_at=?1 WHERE id='item-1'",
            [NOW - 30 * 86_400_000],
        )
        .unwrap();
    assert_eq!(
        s.cleanup(&policy(10, 1000, 0), NOW).unwrap().deleted_items,
        0
    );
    assert_eq!(
        s.cleanup(&policy(10, 1000, 30), NOW).unwrap().deleted_items,
        1
    );
    assert_eq!(ids(&s), vec!["item-3", "item-2", "item-1"]);
}
#[test]
fn cleanup_protected_records_neither_deleted_nor_counted() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 5);
    s.conn
        .execute_batch(
            "INSERT INTO groups VALUES(1,'synthetic',0,0);
        UPDATE items SET pinned=1,size=999999,used_at=0 WHERE id='item-0';
        UPDATE items SET favorite=1,size=999999,used_at=0 WHERE id='item-1';
        UPDATE items SET group_id=1,size=999999,used_at=0 WHERE id='item-2';",
        )
        .unwrap();
    assert_eq!(s.cleanup(&policy(2, 20, 1), NOW).unwrap().deleted_items, 0);
    assert_eq!(s.cleanup(&policy(1, 10, 1), NOW).unwrap().deleted_items, 1);
    let left = ids(&s);
    for id in ["item-0", "item-1", "item-2", "item-4"] {
        assert!(left.contains(&id.into()));
    }
}
#[test]
fn cleanup_newest_survives_age_size_and_expiration() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 1);
    s.conn
        .execute_batch("UPDATE items SET used_at=0,expires_at=1,size=999999")
        .unwrap();
    assert_eq!(s.cleanup(&policy(1, 1, 1), NOW).unwrap().deleted_items, 0);
    assert_eq!(ids(&s), vec!["item-0"]);
}
#[test]
fn cleanup_space_removes_old_prefix_not_small_new_items() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 4);
    s.conn
        .execute_batch("UPDATE items SET size=100 WHERE id='item-1'")
        .unwrap();
    assert_eq!(
        s.cleanup(&policy(100, 25, 0), NOW).unwrap().deleted_items,
        2
    );
    assert_eq!(ids(&s), vec!["item-3", "item-2"]);
}
#[test]
fn cleanup_expiration_ignores_protection_and_includes_exact_now() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 5);
    s.conn
        .execute_batch(
            "INSERT INTO groups VALUES(1,'synthetic',0,0);
        UPDATE items SET pinned=1 WHERE id='item-0';
        UPDATE items SET favorite=1 WHERE id='item-1';
        UPDATE items SET group_id=1 WHERE id='item-2';",
        )
        .unwrap();
    s.conn
        .execute(
            "UPDATE items SET expires_at=?1 WHERE id IN ('item-0','item-1','item-2')",
            [NOW],
        )
        .unwrap();
    s.conn
        .execute(
            "UPDATE items SET expires_at=?1 WHERE id='item-3'",
            [NOW + 1],
        )
        .unwrap();
    assert_eq!(
        s.cleanup(&policy(100, 1000, 0), NOW).unwrap().deleted_items,
        3
    );
    assert_eq!(ids(&s), vec!["item-4", "item-3"]);
}
#[test]
fn mandatory_deletions_do_not_cause_extra_capacity_deletions() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 4);
    s.conn
        .execute("UPDATE items SET expires_at=?1 WHERE id='item-2'", [NOW])
        .unwrap();
    assert_eq!(s.cleanup(&policy(3, 30, 0), NOW).unwrap().deleted_items, 1);
    assert_eq!(ids(&s), vec!["item-3", "item-1", "item-0"]);
}
#[test]
fn protected_newest_does_not_consume_capacity() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 3);
    s.conn
        .execute_batch("UPDATE items SET favorite=1 WHERE id='item-2'")
        .unwrap();
    assert_eq!(s.cleanup(&policy(2, 20, 0), NOW).unwrap().deleted_items, 0);
}
#[test]
fn cleanup_empty_invalid_policy_and_clock_extremes() {
    let f = Fixture::new();
    let mut s = f.open();
    assert_eq!(
        s.cleanup(&RetentionPolicy::default(), NOW)
            .unwrap()
            .deleted_items,
        0
    );
    seed(&s, 2);
    for invalid in [
        policy(0, 10, 0),
        policy(2, 0, 0),
        policy(2, i64::MAX as u64 + 1, 0),
        policy(2, u64::MAX, 0),
    ] {
        assert!(s.cleanup(&invalid, NOW).is_err());
        assert_eq!(ids(&s), vec!["item-1", "item-0"]);
        assert!(s.load("item-0").is_ok());
        assert!(s.load("item-1").is_ok());
    }
    assert_eq!(
        s.cleanup(&policy(2, 20, u32::MAX), i64::MIN)
            .unwrap()
            .deleted_items,
        0
    );
}
#[test]
fn save_outcome_dedup_and_touch_distinguish_new_records() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = Clip {
        formats: vec![model::FormatData {
            name: "Rich Text Format".into(),
            bytes: b"synthetic".to_vec(),
        }],
    };
    assert!(s.save_with_outcome(&clip).unwrap());
    assert!(!s.save_with_outcome(&clip).unwrap());
    s.touch(&clip.hash()).unwrap();
    assert!(!s.save_with_outcome(&clip).unwrap());
}
#[test]
fn vacuum_after_large_delete_keeps_sqlcipher_wal_data_readable() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 402);
    let first: Vec<_> = (0..200).map(|i| format!("item-{i}")).collect();
    assert!(!s.delete(&first).unwrap().vacuumed);
    let second: Vec<_> = (200..401).map(|i| format!("item-{i}")).collect();
    assert!(s.delete(&second).unwrap().vacuumed);
    assert_eq!(
        s.conn
            .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        s.conn
            .pragma_query_value(None, "secure_delete", |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(s);
    assert_ne!(
        &fs::read(f.0.join("history.db")).unwrap()[..16],
        b"SQLite format 3\0"
    );
    let s = f.open();
    assert_eq!(ids(&s), vec!["item-401"]);
    assert_eq!(s.load("item-401").unwrap().formats[0].bytes, vec![1]);
}
#[test]
fn startup_vacuum_reclaims_free_pages_and_keeps_data_after_reopen() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 3);
    assert!(!s.vacuum_on_startup().unwrap());
    s.conn
        .execute_batch("UPDATE formats SET inline=zeroblob(1048576) WHERE item='item-0'")
        .unwrap();
    assert!(!s.delete(&["item-0".into()]).unwrap().vacuumed);
    assert!(s.vacuum_on_startup().unwrap());
    assert!(!s.vacuum_on_startup().unwrap());
    drop(s);
    let s = f.open();
    assert_eq!(s.load("item-2").unwrap().formats[0].bytes, vec![1]);
}
#[test]
fn v3_indexes_migrate_from_v2_with_encrypted_backup_and_rollback() {
    let f = Fixture::new();
    let s = f.open();
    seed(&s, 2);
    super::search_tests::remove_v4(&s.conn);
    s.conn.execute_batch("DROP INDEX formats_blob; DROP INDEX items_touched; DROP INDEX items_expiry; DROP INDEX items_retention; PRAGMA user_version=2;").unwrap();
    drop(s);
    assert!(Storage::open_with_hook(&f.0, &[72; 32], |_| anyhow::bail!(
        "synthetic migration failure"
    ))
    .is_err());
    let conn = Connection::open(f.0.join("history.db")).unwrap();
    conn.pragma_update(None, "key", format!("x'{}'", "48".repeat(32)))
        .unwrap();
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='formats_blob'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    let s = f.open();
    assert_eq!(
        s.conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        migrations::VERSION
    );
    assert_eq!(s.conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name IN ('formats_blob','items_touched','items_expiry','items_retention')",[],|r|r.get::<_,i64>(0)).unwrap(),4);
    assert_eq!(ids(&s).len(), 2);
    assert_ne!(
        &fs::read(f.0.join("history.db.bak-v2")).unwrap()[..16],
        b"SQLite format 3\0"
    );
}

#[test]
fn file_and_bitmap_records_really_share_generated_previews() {
    use std::io::Cursor;
    let f = Fixture::new();
    let mut s = f.open();
    let image = image::RgbaImage::from_pixel(40, 30, image::Rgba([21, 42, 84, 128]));
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let source = f.0.join("synthetic.png");
    fs::write(&source, png.get_ref()).unwrap();
    let bitmap = Clip {
        formats: vec![model::FormatData {
            name: "PNG".into(),
            bytes: png.into_inner(),
        }],
    };
    let file = Clip {
        formats: vec![model::FormatData {
            name: "CF_HDROP".into(),
            bytes: model::encode_hdrop(&[source.to_string_lossy().into_owned()]),
        }],
    };
    s.save(&bitmap).unwrap();
    s.save(&file).unwrap();
    let views = s.list().unwrap();
    assert!(views[0].thumbnail.is_some());
    assert_eq!(views[0].thumbnail, views[1].thumbnail);
    let thumbnail = views[0].thumbnail.clone().unwrap();
    s.delete(&[file.hash()]).unwrap();
    assert!(s.read_thumbnail(&thumbnail).is_ok());
    assert!(s.load(&bitmap.hash()).is_ok());
    s.delete(&[bitmap.hash()]).unwrap();
    assert_eq!(fs::read_dir(&s.blobs).unwrap().count(), 0);
    assert!(source.is_file());
}
#[test]
fn cleanup_tied_touched_keeps_exactly_one_deterministic_newest() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 3);
    s.conn
        .execute_batch("UPDATE items SET touched=1,expires_at=1")
        .unwrap();
    assert_eq!(s.cleanup(&policy(1, 1, 1), NOW).unwrap().deleted_items, 2);
    assert_eq!(ids(&s), vec!["item-0"]);
}

#[test]
fn cleanup_days_and_item_limit_apply_together() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 6);
    // 较新的记录先因天数到期删除，再对其余记录应用条数上限。
    s.conn
        .execute(
            "UPDATE items SET used_at=?1 WHERE id='item-4'",
            [NOW - 30 * 86_400_000 - 1],
        )
        .unwrap();
    let report = s.cleanup(&policy(3, 1000, 30), NOW).unwrap();
    assert_eq!(report.deleted_items, 3);
    assert_eq!(ids(&s), vec!["item-5", "item-3", "item-2"]);
    for id in ["item-0", "item-1", "item-4"] {
        assert!(s.load(id).is_err());
    }
}

#[test]
fn cleanup_all_protected_records_survive_each_protection_type() {
    for protection in ["pinned=1", "favorite=1", "group_id=1"] {
        let f = Fixture::new();
        let mut s = f.open();
        seed(&s, 3);
        s.conn
            .execute_batch("INSERT INTO groups VALUES(1,'synthetic',0,0)")
            .unwrap();
        s.conn
            .execute_batch(&format!(
                "UPDATE items SET {protection},size=999999,used_at=0"
            ))
            .unwrap();
        let report = s.cleanup(&policy(1, 1, 1), NOW).unwrap();
        assert_eq!(report.deleted_items, 0, "{protection}");
        assert_eq!(report.reclaimed_blobs, 0);
        assert_eq!(ids(&s).len(), 3);
    }
}

#[test]
fn cleanup_expired_protected_newest_survives_each_protection_type() {
    for protection in ["pinned=1", "favorite=1", "group_id=1"] {
        let f = Fixture::new();
        let mut s = f.open();
        seed(&s, 3);
        s.conn
            .execute_batch("INSERT INTO groups VALUES(1,'synthetic',0,0)")
            .unwrap();
        s.conn
            .execute(
                &format!("UPDATE items SET {protection},expires_at=?1"),
                [NOW],
            )
            .unwrap();
        let report = s.cleanup(&policy(1, 1, 1), NOW).unwrap();
        assert_eq!(report.deleted_items, 2, "{protection}");
        assert_eq!(ids(&s), vec!["item-2"]);
        assert!(s.load("item-2").is_ok());
    }
}

#[test]
fn cleanup_oversized_newest_removes_older_records() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 3);
    s.conn
        .execute_batch("UPDATE items SET size=11 WHERE id='item-2'")
        .unwrap();
    assert_eq!(s.cleanup(&policy(10, 10, 0), NOW).unwrap().deleted_items, 2);
    assert_eq!(ids(&s), vec!["item-2"]);
    assert!(s.load("item-2").is_ok());
    assert_eq!(s.cleanup(&policy(10, 10, 0), NOW).unwrap().deleted_items, 0);
}

#[test]
fn cleanup_tied_touched_deletes_in_stable_order() {
    for order in [[0, 1, 2, 3], [3, 2, 1, 0]] {
        let f = Fixture::new();
        let mut s = f.open();
        for i in order {
            add(&s, &format!("item-{i}"), 7, 10, NOW);
        }
        // 记录真实 DELETE 顺序，避免只验证最终集合而漏掉并列排序回归。
        s.conn
            .execute_batch(
                "CREATE TEMP TABLE deletion_order(seq INTEGER PRIMARY KEY,id TEXT);
                 CREATE TEMP TRIGGER log_delete AFTER DELETE ON items BEGIN
                     INSERT INTO deletion_order(id) VALUES(OLD.id);
                 END;",
            )
            .unwrap();
        assert_eq!(s.cleanup(&policy(2, 20, 0), NOW).unwrap().deleted_items, 2);
        assert_eq!(ids(&s), vec!["item-0", "item-1"]);
        let deleted: Vec<String> = s
            .conn
            .prepare("SELECT id FROM deletion_order ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(deleted, vec!["item-3", "item-2"]);
    }
}

#[test]
fn delete_then_save_identical_content_recreates_blobs_and_loads() {
    let f = Fixture::new();
    let mut s = f.open();
    let mut clip = bitmap_clip();
    clip.formats.push(model::FormatData {
        name: "Rich Text Format".into(),
        bytes: vec![0x5a; 64 * 1024 + 1],
    });
    s.save(&clip).unwrap();
    let blobs: Vec<String> = s
        .conn
        .prepare(
            "SELECT blob FROM formats WHERE blob IS NOT NULL
             UNION SELECT thumbnail FROM items WHERE thumbnail IS NOT NULL
             UNION SELECT preview FROM items WHERE preview IS NOT NULL",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(blobs.len() >= 2);
    let thumb = s.list().unwrap()[0].thumbnail.clone().unwrap();
    let expected_thumb = s.read_thumbnail(&thumb).unwrap();
    assert_loaded(&s, &clip);
    let report = s.delete(&[clip.hash()]).unwrap();
    assert_eq!(report.deleted_items, 1);
    assert_eq!(report.reclaimed_blobs, blobs.len());
    assert!(report.failed_blobs.is_empty());
    assert!(s.load(&clip.hash()).is_err());
    for blob in &blobs {
        assert!(!s.blobs.join(blob).exists());
    }

    assert!(s.save_with_outcome(&clip).unwrap());
    for blob in &blobs {
        assert!(s.blobs.join(blob).is_file());
    }
    assert_loaded(&s, &clip);
    assert_eq!(s.read_thumbnail(&thumb).unwrap(), expected_thumb);
    drop(s);
    let s = f.open();
    assert_loaded(&s, &clip);
    assert_eq!(s.read_thumbnail(&thumb).unwrap(), expected_thumb);
}

#[test]
fn orphan_scan_preserves_all_loads_thumbnails_and_previews() {
    let f = Fixture::new();
    let mut s = f.open();
    let clips = [
        bitmap_clip(),
        Clip {
            formats: vec![model::FormatData {
                name: "Rich Text Format".into(),
                bytes: vec![0x42; 64 * 1024 + 1],
            }],
        },
    ];
    for clip in &clips {
        s.save(clip).unwrap();
    }
    let preview = s.write_blob(b"registered preview").unwrap();
    s.conn
        .execute(
            "UPDATE items SET preview=?1 WHERE id=?2",
            params![preview, clips[1].hash()],
        )
        .unwrap();
    let previews: Vec<_> = s
        .list()
        .unwrap()
        .into_iter()
        .flat_map(|v| v.thumbnail.into_iter().chain(v.preview))
        .map(|id| {
            let bytes = s.read_thumbnail(&id).unwrap();
            (id, bytes)
        })
        .collect();
    assert!(previews.len() >= 2);
    let orphan = s.write_blob(b"unreferenced object").unwrap();
    let temp = s.blobs.join("interrupted.tmp");
    fs::write(&temp, b"partial").unwrap();
    let report = s.scan_orphan_blobs().unwrap();
    assert_eq!((report.reclaimed_blobs, report.removed_temps), (1, 1));
    assert!(report.failed_files.is_empty());
    assert!(!s.blobs.join(orphan).exists());
    assert!(!temp.exists());
    for clip in &clips {
        assert_loaded(&s, clip);
    }
    for (id, expected) in &previews {
        assert_eq!(&s.read_thumbnail(id).unwrap(), expected);
    }
    drop(s);
    let s = f.open();
    for clip in &clips {
        assert_loaded(&s, clip);
    }
    for (id, expected) in &previews {
        assert_eq!(&s.read_thumbnail(id).unwrap(), expected);
    }
}

#[test]
fn startup_vacuum_runs_only_strictly_above_twenty_five_percent() {
    let f = Fixture::new();
    let s = f.open();
    seed(&s, 1);
    s.conn
        .execute_batch("CREATE TABLE vacuum_padding(id INTEGER PRIMARY KEY, payload BLOB)")
        .unwrap();
    let page_size: u64 = s
        .conn
        .pragma_query_value(None, "page_size", |r| r.get::<_, String>(0))
        .unwrap()
        .parse()
        .unwrap();
    // 半页以上的行各占一个叶页；用真实 SQLCipher 页数构造整除边界。
    for id in 1..=80 {
        s.conn
            .execute(
                "INSERT INTO vacuum_padding VALUES(?1,zeroblob(?2))",
                params![id, page_size / 2],
            )
            .unwrap();
    }
    for id in 81..=96 {
        if page_counts(&s).0.is_multiple_of(4) {
            break;
        }
        s.conn
            .execute(
                "INSERT INTO vacuum_padding VALUES(?1,zeroblob(?2))",
                params![id, page_size / 2],
            )
            .unwrap();
    }
    let (pages, free) = page_counts(&s);
    assert_eq!(free, 0);
    assert_eq!(pages % 4, 0, "必须精确构造 25% 边界");
    let quarter = pages / 4;
    let mut next_id = 1;
    while page_counts(&s).1 < quarter - 1 {
        assert!(next_id <= 80, "构造空闲页的步骤必须有界");
        s.conn
            .execute("DELETE FROM vacuum_padding WHERE id=?1", [next_id])
            .unwrap();
        next_id += 1;
    }
    assert_eq!(page_counts(&s), (pages, quarter - 1));
    assert!(!s.vacuum_on_startup().unwrap());
    assert_eq!(page_counts(&s), (pages, quarter - 1));

    s.conn
        .execute("DELETE FROM vacuum_padding WHERE id=?1", [next_id])
        .unwrap();
    assert_eq!(page_counts(&s), (pages, quarter));
    assert!(!s.vacuum_on_startup().unwrap());
    assert_eq!(page_counts(&s), (pages, quarter));

    s.conn
        .execute("DELETE FROM vacuum_padding WHERE id=?1", [next_id + 1])
        .unwrap();
    assert_eq!(page_counts(&s), (pages, quarter + 1));
    assert!(s.vacuum_on_startup().unwrap());
    assert_eq!(page_counts(&s).1, 0);
    drop(s);
    let s = f.open();
    assert_eq!(s.load("item-0").unwrap().formats[0].bytes, vec![1]);
}

// 合成小位图与独立加密数据库覆盖采集→维护→固化，不访问系统剪贴板。
fn flush_capture(owned: bool) -> crate::clipboard::Captured {
    let depth = if owned { 24 } else { 32 };
    crate::clipboard::Captured {
        source: Default::default(),
        clip: Clip {
            formats: vec![model::FormatData {
                name: "CF_DIB".into(),
                bytes: crate::owner_flush::tests::dib(19, 17, depth, false, None),
            }],
        },
        sequence: if owned { 1 } else { 2 },
        context: crate::owner_flush::CaptureContext {
            formats: Some(if owned {
                [8, 13].into_iter().collect()
            } else {
                [8, 2, 17].into_iter().collect()
            }),
            owner: if owned {
                crate::owner_flush::tests::owned()
            } else {
                crate::clipboard_owner::ClipboardOwner::absent()
            },
            fingerprint: None,
            bitmap_offered: true,
            record_id: None,
        },
    }
}

fn cleanup_capture(
    storage: &Mutex<Storage>,
    context: &Mutex<Option<crate::owner_flush::CaptureContext>>,
    policy: &RetentionPolicy,
) -> DeleteReport {
    // 与 worker 一致，先上下文后存储；复用同一个纯失效判定函数。
    let mut previous = context.lock().unwrap();
    let report = storage
        .lock()
        .unwrap()
        .cleanup(policy, unix_millis())
        .unwrap();
    crate::owner_flush::invalidate_deleted(&mut previous, &report.deleted_ids);
    report
}

#[test]
fn owner_flush_survives_cleanup_without_deletions() {
    let f = Fixture::new();
    let storage = Mutex::new(f.open());
    let context = Mutex::new(None);
    assert_eq!(
        crate::runtime::save_capture(&storage, &context, flush_capture(true)).unwrap(),
        Some(true)
    );
    let before = storage.lock().unwrap().list().unwrap();
    assert_eq!(
        context.lock().unwrap().as_ref().unwrap().record_id.as_ref(),
        Some(&before[0].id)
    );
    let report = cleanup_capture(&storage, &context, &RetentionPolicy::default());
    assert_eq!(report.deleted_items, 0);
    assert!(report.deleted_ids.is_empty());
    assert_eq!(
        crate::runtime::save_capture(&storage, &context, flush_capture(false)).unwrap(),
        None
    );
    assert_eq!(
        serde_json::to_value(storage.lock().unwrap().list().unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

#[test]
fn owner_flush_context_cleared_when_cleanup_deletes_its_record() {
    let f = Fixture::new();
    let storage = Mutex::new(f.open());
    let context = Mutex::new(None);
    let first = flush_capture(true);
    let first_id = first.clip.hash();
    crate::runtime::save_capture(&storage, &context, first).unwrap();
    storage
        .lock()
        .unwrap()
        .save(&Clip {
            formats: vec![model::FormatData {
                name: "Rich Text Format".into(),
                bytes: b"newer record".to_vec(),
            }],
        })
        .unwrap();
    let report = cleanup_capture(&storage, &context, &policy(1, 1_000_000, 0));
    assert_eq!(report.deleted_ids, vec![first_id]);
    assert!(context.lock().unwrap().is_none());
    assert_eq!(
        crate::runtime::save_capture(&storage, &context, flush_capture(false)).unwrap(),
        Some(true)
    );
}

#[test]
fn owner_flush_survives_cleanup_deleting_other_records() {
    let f = Fixture::new();
    let storage = Mutex::new(f.open());
    let context = Mutex::new(None);
    add(&storage.lock().unwrap(), "older", 0, 1, unix_millis());
    crate::runtime::save_capture(&storage, &context, flush_capture(true)).unwrap();
    let report = cleanup_capture(&storage, &context, &policy(1, 1_000_000, 0));
    assert_eq!(report.deleted_ids, vec!["older"]);
    assert!(context.lock().unwrap().is_some());
    assert_eq!(
        crate::runtime::save_capture(&storage, &context, flush_capture(false)).unwrap(),
        None
    );
}

#[test]
fn manual_delete_invalidates_only_actual_context_record_and_reports_unique_ids() {
    let f = Fixture::new();
    let storage = Mutex::new(f.open());
    let context = Mutex::new(None);
    crate::runtime::save_capture(&storage, &context, flush_capture(true)).unwrap();
    add(&storage.lock().unwrap(), "other", 0, 1, unix_millis());
    let mut previous = context.lock().unwrap();
    let id = previous.as_ref().unwrap().record_id.clone().unwrap();
    let report = storage
        .lock()
        .unwrap()
        .delete(&["other".into(), "other".into(), "missing".into()])
        .unwrap();
    assert_eq!(report.deleted_items, 1);
    assert_eq!(report.deleted_ids, vec!["other"]);
    assert!(serde_json::to_value(&report)
        .unwrap()
        .get("deleted_ids")
        .is_none());
    crate::owner_flush::invalidate_deleted(&mut previous, &report.deleted_ids);
    assert!(previous.is_some());
    let report = storage
        .lock()
        .unwrap()
        .delete(&[id.clone(), id.clone()])
        .unwrap();
    assert_eq!(report.deleted_items, 1);
    assert_eq!(report.deleted_ids, vec![id]);
    crate::owner_flush::invalidate_deleted(&mut previous, &report.deleted_ids);
    assert!(previous.is_none());
    assert!(!crate::owner_flush::should_skip(
        previous.as_ref(),
        &flush_capture(false).context
    ));
}

#[test]
fn capture_dedup_context_tracks_existing_id_instead_of_new_hash() {
    let f = Fixture::new();
    let storage = Mutex::new(f.open());
    let context = Mutex::new(None);
    let mut first = flush_capture(true);
    first.clip = Clip {
        formats: vec![model::FormatData {
            name: "CF_HDROP".into(),
            bytes: model::encode_hdrop(&[r"C:\synthetic\item.txt".into()]),
        }],
    };
    let id = first.clip.hash();
    let mut second = flush_capture(true);
    second.clip = first.clip.clone();
    second.clip.formats.push(model::FormatData {
        name: "HTML Format".into(),
        bytes: b"extra format".to_vec(),
    });
    assert_ne!(id, second.clip.hash());
    assert_eq!(
        crate::runtime::save_capture(&storage, &context, first).unwrap(),
        Some(true)
    );
    assert_eq!(
        crate::runtime::save_capture(&storage, &context, second).unwrap(),
        Some(false)
    );
    let mut previous = context.lock().unwrap();
    assert_eq!(previous.as_ref().unwrap().record_id.as_ref(), Some(&id));
    let report = storage.lock().unwrap().delete(&[id]).unwrap();
    crate::owner_flush::invalidate_deleted(&mut previous, &report.deleted_ids);
    assert!(previous.is_none());
}

#[test]
fn prepared_save_recheck_returns_actual_duplicate_id() {
    let f = Fixture::new();
    let mut storage = f.open();
    let first = Clip {
        formats: vec![model::FormatData {
            name: "CF_HDROP".into(),
            bytes: model::encode_hdrop(&[r"C:\synthetic\item.txt".into()]),
        }],
    };
    let mut second = first.clone();
    second.formats.push(model::FormatData {
        name: "HTML Format".into(),
        bytes: b"extra format".to_vec(),
    });
    let prepared = PreparedSave::new(&second, SaveIdentity::new(&second), &storage.key).unwrap();
    storage.save(&first).unwrap();
    assert_eq!(
        storage.commit_prepared(prepared).unwrap(),
        SaveOutcome {
            id: first.hash(),
            inserted: false
        }
    );
}

#[test]
fn preview_matches_cleanup_for_each_policy_and_protections() {
    for policy in [
        policy(10, 10000, 0),
        policy(100, 30, 0),
        policy(100, 10000, 1),
        policy(10, 30, 1),
    ] {
        let fixture = Fixture::new();
        let mut s = fixture.open();
        seed(&s, 30);
        s.conn.execute_batch("UPDATE items SET used_at=0 WHERE touched < 10; UPDATE items SET pinned=1 WHERE touched=2; UPDATE items SET favorite=1 WHERE touched=3; UPDATE items SET expires_at=0 WHERE touched=4;").unwrap();
        let before = ids(&s);
        let preview = s.preview_cleanup(&policy, NOW).unwrap();
        assert_eq!(ids(&s), before);
        let actual = s.cleanup(&policy, NOW).unwrap();
        assert_eq!(preview, actual.deleted_items);
        assert_eq!(s.preview_cleanup(&policy, NOW).unwrap(), 0);
    }
}

// T3-4：query_only 和 total_changes 同时约束预估；预期删除对象独立于 cleanup_ids。
fn assert_readonly_preview_then_cleanup(
    s: &mut Storage,
    policy: &RetentionPolicy,
    now: i64,
    expected: &[&str],
) {
    let before_ids = ids(s);
    let changes = s.conn.total_changes();
    let blobs: Vec<_> = fs::read_dir(&s.blobs)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    s.conn.pragma_update(None, "query_only", true).unwrap();
    for _ in 0..2 {
        assert_eq!(s.preview_cleanup(policy, now).unwrap(), expected.len());
        assert_eq!(ids(s), before_ids);
        assert_eq!(s.conn.total_changes(), changes);
        for (path, bytes) in &blobs {
            assert_eq!(fs::read(path).unwrap(), *bytes);
        }
        assert_eq!(fs::read_dir(&s.blobs).unwrap().count(), blobs.len());
    }
    s.conn.pragma_update(None, "query_only", false).unwrap();
    let report = s.cleanup(policy, now).unwrap();
    assert_eq!(report.deleted_items, expected.len());
    assert_eq!(report.deleted_ids, expected);
    assert_eq!(ids(s).len(), before_ids.len() - expected.len());
    assert_eq!(s.preview_cleanup(policy, now).unwrap(), 0);
}

#[test]
fn preview_days_and_count_combined_have_independent_expected_victims() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 6);
    s.conn
        .execute(
            "UPDATE items SET used_at=?1 WHERE id='item-4'",
            [NOW - 30 * 86_400_000 - 1],
        )
        .unwrap();
    assert_readonly_preview_then_cleanup(
        &mut s,
        &policy(3, 1000, 30),
        NOW,
        &["item-0", "item-1", "item-4"],
    );
}

#[test]
fn preview_preserves_each_protection_and_does_not_count_it_toward_limits() {
    for protection in ["pinned=1", "favorite=1", "group_id=1"] {
        let f = Fixture::new();
        let mut s = f.open();
        seed(&s, 6);
        s.conn
            .execute_batch("INSERT INTO groups VALUES(1,'synthetic',0,0)")
            .unwrap();
        s.conn
            .execute_batch(&format!(
                "UPDATE items SET {protection},size=999999,used_at=0 WHERE touched<3"
            ))
            .unwrap();
        assert_readonly_preview_then_cleanup(&mut s, &policy(2, 20, 1), NOW, &["item-3"]);
    }
}

#[test]
fn preview_expiration_overrides_each_protection_but_preserves_newest() {
    for protection in ["pinned=1", "favorite=1", "group_id=1"] {
        let f = Fixture::new();
        let mut s = f.open();
        seed(&s, 3);
        s.conn
            .execute_batch("INSERT INTO groups VALUES(1,'synthetic',0,0)")
            .unwrap();
        s.conn
            .execute(
                &format!("UPDATE items SET {protection},expires_at=?1,size=999999,used_at=0"),
                [NOW],
            )
            .unwrap();
        assert_readonly_preview_then_cleanup(&mut s, &policy(1, 1, 1), NOW, &["item-0", "item-1"]);
        assert_eq!(ids(&s), ["item-2"]);
    }
}

#[test]
fn preview_age_cutoff_and_zero_days_are_exact() {
    for (days, expected) in [(0, vec![]), (30, vec!["item-0"])] {
        let f = Fixture::new();
        let mut s = f.open();
        seed(&s, 4);
        let cutoff = NOW - 30 * 86_400_000;
        for (id, used) in [
            ("item-0", cutoff - 1),
            ("item-1", cutoff),
            ("item-2", cutoff + 1),
            ("item-3", 0),
        ] {
            s.conn
                .execute("UPDATE items SET used_at=?1 WHERE id=?2", params![used, id])
                .unwrap();
        }
        assert_readonly_preview_then_cleanup(&mut s, &policy(100, 1000, days), NOW, &expected);
    }
}

#[test]
fn preview_tied_newest_and_overlapping_limits_count_victims_once() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 4);
    s.conn
        .execute_batch("UPDATE items SET touched=1,used_at=0,expires_at=0,size=999999")
        .unwrap();
    assert_readonly_preview_then_cleanup(
        &mut s,
        &policy(1, 1, 1),
        NOW,
        &["item-3", "item-2", "item-1"],
    );
    assert_eq!(ids(&s), ["item-0"]);
}

#[test]
fn preview_queries_without_deleting_or_reclaiming_shared_blobs() {
    let f = Fixture::new();
    let mut s = f.open();
    seed(&s, 3);
    let shared = s.write_blob(b"shared preview blob").unwrap();
    let victim_only = s.write_blob(b"victim-only preview blob").unwrap();
    s.conn
        .execute("UPDATE items SET thumbnail=?1", [&shared])
        .unwrap();
    s.conn
        .execute(
            "UPDATE items SET preview=?1 WHERE id='item-0'",
            [&victim_only],
        )
        .unwrap();
    assert_readonly_preview_then_cleanup(&mut s, &policy(1, 1000, 0), NOW, &["item-0", "item-1"]);
    assert!(s.blobs.join(shared).is_file());
    assert!(!s.blobs.join(victim_only).exists());
}

#[test]
fn preview_empty_and_invalid_policies_never_change_database() {
    let f = Fixture::new();
    let mut s = f.open();
    assert_readonly_preview_then_cleanup(&mut s, &policy(1, 1, 1), NOW, &[]);
    seed(&s, 3);
    let before = ids(&s);
    let changes = s.conn.total_changes();
    for invalid in [policy(0, 1, 0), policy(1, 0, 0), policy(1, u64::MAX, 0)] {
        assert!(s.preview_cleanup(&invalid, NOW).is_err());
        assert_eq!(ids(&s), before);
        assert_eq!(s.conn.total_changes(), changes);
    }
}
