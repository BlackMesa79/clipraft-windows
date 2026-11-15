//! 合成内容、项目内独立加密数据库；不接触系统剪贴板。
use super::*;
use crate::settings::Settings;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "t7-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn open(&self) -> Storage {
        Storage::open(&self.0, &[77; 32]).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn token() -> String {
    format!("sk-{}", "SyntheticOnly0123456789abcdef")
}

#[test]
fn sensitive_image_returns_mosaic_and_explicit_reveal_returns_registered_preview() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let image = image::RgbImage::from_fn(120, 80, |x, y| {
        image::Rgb([(x * 2) as u8, (y * 3) as u8, ((x + y) % 256) as u8])
    });
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let clip = Clip {
        formats: vec![model::FormatData {
            name: "PNG".into(),
            bytes: encoded.into_inner(),
        }],
    };
    storage.save(&clip).unwrap();
    let item = storage.preview_item(&clip.hash()).unwrap();
    let hash = item.preview.unwrap();
    let clear = storage.read_thumbnail(&hash).unwrap();
    storage.set_sensitive(&[clip.hash()], true, 10).unwrap();
    assert!(storage.read_thumbnail(&hash).is_err());
    let masked = storage.display_detail(&clip.hash()).unwrap();
    let masked_bytes = masked.image.unwrap();
    assert_ne!(masked_bytes, clear);
    let decoded = image::load_from_memory(&masked_bytes).unwrap().to_rgba8();
    assert_eq!(decoded.dimensions(), (120, 80));
    let colors: HashSet<_> = decoded.pixels().map(|pixel| pixel.0).collect();
    assert!(colors.len() <= 100);
    assert_eq!(
        storage.reveal_detail(&clip.hash()).unwrap().image.unwrap(),
        clear
    );
    assert!(
        storage.read_thumbnail(&hash).is_err(),
        "显式查看不放宽普通图片协议"
    );
    assert!(storage
        .display_detail(&clip.hash())
        .unwrap()
        .image
        .is_some());
    storage.delete(&[clip.hash()]).unwrap();
    assert!(storage.reveal_detail(&clip.hash()).is_err());
}

#[test]
fn sensitive_image_files_use_cached_mosaic_and_reveal_without_reopening_source() {
    for extension in ["png", "jpg"] {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let path = fixture.0.join(format!("synthetic.{extension}"));
        let image = image::RgbImage::from_fn(120, 80, |x, y| {
            image::Rgb([(x * 2) as u8, (y * 3) as u8, ((x + y) % 256) as u8])
        });
        image.save(&path).unwrap();
        let path = path.to_string_lossy().into_owned();
        let clip = Clip {
            formats: vec![model::FormatData {
                name: "CF_HDROP".into(),
                bytes: model::encode_hdrop(std::slice::from_ref(&path)),
            }],
        };
        storage.save(&clip).unwrap();
        let original = storage.preview_item(&clip.hash()).unwrap();
        assert_eq!(original.kind, "files");
        let hash = original.preview.unwrap();
        let clear = storage.read_thumbnail(&hash).unwrap();
        storage.set_sensitive(&[clip.hash()], true, 10).unwrap();
        fs::remove_file(&path).unwrap();
        let safe = storage.preview_item(&clip.hash()).unwrap();
        assert!(safe.files.is_empty() && safe.preview.is_none() && safe.thumbnail.is_none());
        let masked = storage.display_detail(&clip.hash()).unwrap();
        assert!(masked.files.is_empty() && masked.text.is_empty());
        let bytes = masked.image.unwrap();
        assert_ne!(bytes, clear);
        let pixels = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (120, 80));
        assert!(
            pixels
                .pixels()
                .map(|pixel| pixel.0)
                .collect::<HashSet<_>>()
                .len()
                <= 100
        );
        let revealed = storage.reveal_detail(&clip.hash()).unwrap();
        assert_eq!(revealed.image.unwrap(), clear);
        assert_eq!(revealed.files[0].path, path);
        assert_eq!(revealed.files[0].exists, Some(false));
        assert!(storage.read_thumbnail(&hash).is_err());
        assert!(storage
            .display_detail(&clip.hash())
            .unwrap()
            .image
            .is_some());
    }
}

#[test]
fn sensitive_non_image_files_keep_masked_details_without_image() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let path = fixture.0.join("synthetic.txt");
    fs::write(&path, "synthetic text file").unwrap();
    let path = path.to_string_lossy().into_owned();
    let clip = Clip {
        formats: vec![model::FormatData {
            name: "CF_HDROP".into(),
            bytes: model::encode_hdrop(std::slice::from_ref(&path)),
        }],
    };
    storage.save(&clip).unwrap();
    storage.set_sensitive(&[clip.hash()], true, 10).unwrap();
    let masked = storage.display_detail(&clip.hash()).unwrap();
    assert!(masked.image.is_none() && masked.files.is_empty());
    let revealed = storage.reveal_detail(&clip.hash()).unwrap();
    assert!(revealed.image.is_none());
    assert_eq!(revealed.files[0].path, path);
}

#[test]
fn keeping_one_sensitive_record_cancels_expiry_and_survives_recopy_and_reopen() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let clip = text(&token());
    let id = clip.hash();
    storage.save(&clip).unwrap();
    let timed = text("独立定时删除测试");
    storage.save(&timed).unwrap();
    storage.set_sensitive(&[timed.hash()], true, 1).unwrap();
    assert_eq!(
        storage
            .keep_sensitive(&[id.clone(), id.clone(), "missing".into()])
            .unwrap(),
        1
    );
    assert_eq!(state(&storage, &id), (1, None));
    assert!(state(&storage, &timed.hash()).1.is_some());
    storage.save(&clip).unwrap();
    assert_eq!(state(&storage, &id), (1, None));
    storage.save(&text("最新的普通内容")).unwrap();
    storage.delete_expired(i64::MAX).unwrap();
    assert!(storage.load(&id).is_ok());
    assert!(storage.load(&timed.hash()).is_err());
    drop(storage);
    let mut storage = fixture.open();
    assert_eq!(state(&storage, &id), (1, None));
    storage.delete(std::slice::from_ref(&id)).unwrap();
    assert!(storage.load(&id).is_err(), "永久保留仍允许手动删除");
}

#[test]
fn single_and_global_disabled_sensitive_expiry_still_follow_normal_cleanup() {
    let policies = [
        RetentionPolicy {
            max_items: 100,
            max_bytes: u64::from(u32::MAX),
            max_days: 1,
        },
        RetentionPolicy {
            max_items: 1,
            max_bytes: u64::from(u32::MAX),
            max_days: 0,
        },
        RetentionPolicy {
            max_items: 100,
            max_bytes: 1,
            max_days: 0,
        },
    ];
    for global in [false, true] {
        for policy in &policies {
            for protection in ["none", "pinned", "grouped"] {
                let fixture = Fixture::new();
                let mut storage = fixture.open();
                if global {
                    storage
                        .save_settings(&Settings {
                            sensitive_ttl_minutes: 0,
                            ..Settings::default()
                        })
                        .unwrap();
                }
                let id = save(&mut storage);
                storage.keep_sensitive(std::slice::from_ref(&id)).unwrap();
                match protection {
                    "none" => {}
                    "pinned" => {
                        storage.set_pinned(std::slice::from_ref(&id), true).unwrap();
                    }
                    "grouped" => {
                        let group = storage.create_group("测试分组").unwrap().id;
                        storage
                            .move_to_group(std::slice::from_ref(&id), Some(group))
                            .unwrap();
                    }
                    _ => unreachable!(),
                }
                storage.save(&text("保留最新的普通记录")).unwrap();
                storage
                    .conn
                    .execute("UPDATE items SET used_at=0", [])
                    .unwrap();
                assert_eq!(state(&storage, &id), (1, None));
                let expected = usize::from(protection == "none");
                assert_eq!(storage.preview_cleanup(policy, i64::MAX).unwrap(), expected);
                let report = storage.cleanup(policy, i64::MAX).unwrap();
                assert_eq!(
                    report.deleted_items, expected,
                    "global={global}, protection={protection}, policy={policy:?}"
                );
                assert_eq!(storage.load(&id).is_ok(), protection != "none");
            }
        }
    }
}

#[test]
fn global_zero_cancels_existing_deadlines_and_covers_manual_and_detected_records() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let old = save(&mut storage);
    assert!(state(&storage, &old).1.is_some());
    let settings = Settings {
        sensitive_ttl_minutes: 0,
        ..Settings::default()
    };
    storage.save_settings(&settings).unwrap();
    assert_eq!(storage.load_settings().unwrap().sensitive_ttl_minutes, 0);
    assert_eq!(state(&storage, &old), (1, None));
    let automatic = text(&format!("另一条 {}", token()));
    storage.save(&automatic).unwrap();
    assert_eq!(state(&storage, &automatic.hash()), (1, None));
    let manual = text("全局永久保留的普通文本");
    storage.save(&manual).unwrap();
    storage.set_sensitive(&[manual.hash()], true, 1).unwrap();
    assert_eq!(state(&storage, &manual.hash()), (1, None));
    assert!(storage
        .delete_expired(i64::MAX)
        .unwrap()
        .deleted_ids
        .is_empty());
    assert!(storage.next_expiry().unwrap().is_none());
    storage.save_settings(&Settings::default()).unwrap();
    storage.save(&automatic).unwrap();
    assert_eq!(
        state(&storage, &automatic.hash()),
        (1, None),
        "已保留的内容不重新开始倒计时"
    );
    let timed = text("之后新标记的内容");
    storage.save(&timed).unwrap();
    storage.set_sensitive(&[timed.hash()], true, 1).unwrap();
    assert!(state(&storage, &timed.hash()).1.is_some());
}

/// AC-T9-2/4/5/7：分组、收藏、三态、分页搜索、清理和重启组合。
#[test]
fn t9_negative_state_search_survives_group_delete_cleanup_vacuum_and_reopen() {
    let f = Fixture::new();
    let mut s = f.open();
    let clip = text(&format!("组合你好 {}", token()));
    let id = clip.hash();
    s.save(&clip).unwrap();
    let group = s.create_group("T9组合").unwrap().id;
    s.move_to_group(std::slice::from_ref(&id), Some(group))
        .unwrap();
    s.set_favorite(std::slice::from_ref(&id), true).unwrap();
    assert_eq!(state(&s, &id), (1, None));
    let filter = ListFilter {
        query: Some("zuhe nihao".into()),
        favorite_only: true,
        group_id: Some(group),
        ..Default::default()
    };
    assert!(s.list_page(&filter, 0, 1).unwrap().is_empty());
    s.set_sensitive(std::slice::from_ref(&id), false, 1)
        .unwrap();
    assert_eq!(s.list_page(&filter, 0, 1).unwrap()[0].id, id);
    assert!(s.list_page(&filter, 1, 1).unwrap().is_empty());
    s.delete_group(group).unwrap();
    assert!(s.list_page(&filter, 0, 1).unwrap().is_empty());
    let victim = text("T9待清理 ordinary victim");
    let newest = text("T9最新 ordinary newest");
    s.save(&victim).unwrap();
    s.save(&newest).unwrap();
    // 固定时间，确保收藏项不是 newest，且普通旧项由天数策略删除。
    s.conn.execute("UPDATE items SET used_at=0", []).unwrap();
    let policy = RetentionPolicy {
        max_items: 1,
        max_bytes: 1,
        max_days: 1,
    };
    let report = s.cleanup(&policy, i64::MAX).unwrap();
    assert_eq!(report.deleted_ids, [victim.hash()]);
    assert!(s.load(&newest.hash()).is_ok());
    assert!(s
        .list_page(
            &ListFilter {
                query: Some("victim".into()),
                ..Default::default()
            },
            0,
            100
        )
        .unwrap()
        .is_empty());
    s.conn.execute_batch("VACUUM").unwrap();
    drop(s);
    let mut s = f.open();
    s.save(&clip).unwrap();
    assert_eq!(state(&s, &id), (-1, None));
    let filter = ListFilter {
        group_id: None,
        ..filter
    };
    let rows = s.list_page(&filter, 0, 1).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].group_id, None);
    let restored = indexed(&s, &id);
    assert_t7_indexes(&s, &id, (&restored.0, &restored.1));
}
fn text(s: &str) -> Clip {
    Clip {
        formats: vec![model::FormatData {
            name: "CF_UNICODETEXT".into(),
            bytes: s
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
        }],
    }
}
fn state(s: &Storage, id: &str) -> (i64, Option<i64>) {
    s.conn
        .query_row(
            "SELECT sensitive,expires_at FROM items WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}
fn save(s: &mut Storage) -> String {
    let c = text(&format!("普通前文 {} 普通后文", token()));
    s.save(&c).unwrap();
    c.hash()
}
fn indexed(s: &Storage, id: &str) -> (String, String) {
    s.conn
        .query_row(
            "SELECT search_text,search_pinyin FROM items WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}
#[test]
fn detection_writes_state_deadline_and_v4() {
    let f = Fixture::new();
    let mut s = f.open();
    let now = unix_millis();
    let id = save(&mut s);
    let (flag, deadline) = state(&s, &id);
    assert_eq!(flag, 1);
    assert!((now + 600_000..=unix_millis() + 600_000).contains(&deadline.unwrap()));
    assert_eq!(
        s.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
            .unwrap(),
        4
    );
}
#[test]
fn duplicate_refreshes_deadline_without_changing_id() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    s.conn
        .execute("UPDATE items SET expires_at=1 WHERE id=?1", [&id])
        .unwrap();
    assert_eq!(save(&mut s), id);
    assert!(state(&s, &id).1.unwrap() > unix_millis());
    assert_eq!(s.list().unwrap().len(), 1);
}
#[test]
fn manual_override_survives_recapture_and_reopen() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    s.set_sensitive(std::slice::from_ref(&id), false, 10)
        .unwrap();
    drop(s);
    let mut s = f.open();
    assert_eq!(save(&mut s), id);
    assert_eq!(state(&s, &id), (-1, None));
    assert!(!s.list().unwrap()[0].sensitive);
}
#[test]
fn detection_disabled_and_enabled_next_capture() {
    let f = Fixture::new();
    let mut s = f.open();
    let mut cfg = Settings {
        sensitive_detection: false,
        ..Default::default()
    };
    s.save_settings(&cfg).unwrap();
    let id = save(&mut s);
    assert_eq!(state(&s, &id), (0, None));
    cfg.sensitive_detection = true;
    s.save_settings(&cfg).unwrap();
    assert_eq!(state(&s, &id), (0, None));
    save(&mut s);
    assert_eq!(state(&s, &id).0, 1);
}
#[test]
fn disabling_does_not_mutate_or_refresh_existing() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    let before = state(&s, &id);
    s.save_settings(&Settings {
        sensitive_detection: false,
        ..Default::default()
    })
    .unwrap();
    save(&mut s);
    assert_eq!(state(&s, &id), before);
    assert_eq!(indexed(&s, &id), (String::new(), String::new()));
}
#[test]
fn manual_batch_mark_unmark_and_duplicate_ids() {
    let f = Fixture::new();
    let mut s = f.open();
    let a = text("普通文字甲");
    let b = text("普通文字乙");
    s.save(&a).unwrap();
    s.save(&b).unwrap();
    let ids = vec![a.hash(), b.hash(), a.hash(), "missing".into()];
    assert_eq!(s.set_sensitive(&ids, true, 1).unwrap(), 2);
    for id in &ids[..2] {
        assert_eq!(state(&s, id).0, 1);
    }
    assert_eq!(s.set_sensitive(&ids, false, 1).unwrap(), 2);
    for id in &ids[..2] {
        assert_eq!(state(&s, id), (-1, None));
    }
}
#[test]
fn manual_remark_overrides_negative_state() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    s.set_sensitive(std::slice::from_ref(&id), false, 1)
        .unwrap();
    s.set_sensitive(std::slice::from_ref(&id), true, 1).unwrap();
    assert_eq!(state(&s, &id).0, 1);
    assert!(state(&s, &id).1.is_some());
}
fn protection(which: u8) {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    let ids = vec![id.clone()];
    match which {
        0 => {
            s.set_pinned(&ids, true).unwrap();
            s.set_pinned(&ids, false).unwrap();
        }
        1 => {
            s.set_favorite(&ids, true).unwrap();
            s.set_favorite(&ids, false).unwrap();
        }
        _ => {
            let g = s.create_group("合成分组").unwrap();
            s.move_to_group(&ids, Some(g.id)).unwrap();
            s.move_to_group(&ids, None).unwrap();
        }
    }
    assert_eq!(state(&s, &id), (1, None));
    save(&mut s);
    assert_eq!(state(&s, &id), (1, None));
    assert!(!s.list().unwrap()[0].summary.contains(&token()));
}
#[test]
fn pinned_clears_expiry_permanently() {
    protection(0);
}
#[test]
fn favorite_clears_expiry_permanently() {
    protection(1);
}
#[test]
fn grouped_clears_expiry_permanently() {
    protection(2);
}
#[test]
fn ttl_change_is_not_retroactive() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    let before = state(&s, &id);
    s.save_settings(&Settings {
        sensitive_ttl_minutes: 1,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(state(&s, &id), before);
    save(&mut s);
    assert!(state(&s, &id).1.unwrap() < before.1.unwrap());
}
#[test]
fn touch_preserves_sensitive_and_deadline() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    let before = state(&s, &id);
    s.touch(&id).unwrap();
    assert_eq!(state(&s, &id), before);
}
#[test]
fn sensitive_not_indexed_unmark_restores_fts_and_pinyin() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    let filter = ListFilter {
        query: Some("SyntheticOnly".into()),
        ..Default::default()
    };
    assert_eq!(indexed(&s, &id), (String::new(), String::new()));
    assert!(s.list_page(&filter, 0, 100).unwrap().is_empty());
    s.set_sensitive(std::slice::from_ref(&id), false, 10)
        .unwrap();
    assert!(indexed(&s, &id).0.contains(&token()));
    assert!(!indexed(&s, &id).1.is_empty());
    assert_eq!(s.list_page(&filter, 0, 100).unwrap().len(), 1);
}
#[test]
fn list_quick_cache_detail_and_persisted_summary_are_masked() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    let quick = s.list().unwrap();
    let list = s.list_page(&ListFilter::default(), 0, 100).unwrap();
    for value in [
        serde_json::to_string(&quick).unwrap(),
        serde_json::to_string(&list).unwrap(),
        serde_json::to_string(&s.display_detail(&id).unwrap()).unwrap(),
    ] {
        assert!(!value.contains(&token()));
        assert!(value.contains("普通前文"));
    }
    let json: String = s
        .conn
        .query_row("SELECT view FROM items WHERE id=?1", [&id], |r| r.get(0))
        .unwrap();
    assert!(!json.contains(&token()));
    assert!(s.reveal_detail(&id).unwrap().text.contains(&token()));
    assert!(s.load(&id).unwrap().text().unwrap().contains(&token()));
}
#[test]
fn summary_masks_before_180_char_truncation() {
    let f = Fixture::new();
    let mut s = f.open();
    let c = text(&format!("{} sk-{}", "前".repeat(160), "A".repeat(200)));
    s.save(&c).unwrap();
    assert!(s.list().unwrap()[0].summary.ends_with("sk-A••••••••AA"));
}
#[test]
fn manual_unmatched_text_masks_detail() {
    let f = Fixture::new();
    let mut s = f.open();
    let c = text("合成私密备忘内容");
    s.save(&c).unwrap();
    s.set_sensitive(&[c.hash()], true, 10).unwrap();
    assert!(!s
        .display_detail(&c.hash())
        .unwrap()
        .text
        .contains("合成私密备忘内容"));
}
#[test]
fn cleanup_expiry_deletes_newest_but_normal_policy_does_not() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    s.conn
        .execute(
            "UPDATE items SET expires_at=NULL,size=99999999999,used_at=0 WHERE id=?1",
            [&id],
        )
        .unwrap();
    assert_eq!(
        s.cleanup(&RetentionPolicy::default(), unix_millis())
            .unwrap()
            .deleted_items,
        0
    );
    s.conn
        .execute("UPDATE items SET expires_at=1 WHERE id=?1", [&id])
        .unwrap();
    assert_eq!(
        s.cleanup(&RetentionPolicy::default(), unix_millis())
            .unwrap()
            .deleted_ids,
        vec![id]
    );
}
#[test]
fn expiry_path_deletes_newest_and_reports_context_ids() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    let deadline = state(&s, &id).1.unwrap();
    assert_eq!(s.next_expiry().unwrap(), Some(deadline));
    assert_eq!(s.delete_expired(deadline - 1).unwrap().deleted_items, 0);
    let report = s.delete_expired(deadline).unwrap();
    assert_eq!(report.deleted_ids, vec![id]);
    assert!(s.next_expiry().unwrap().is_none());
    assert!(s.list().unwrap().is_empty());
}
#[test]
fn batch_rollback_preserves_state_and_index() {
    let f = Fixture::new();
    let mut s = f.open();
    let a = text("合成甲");
    let b = text("合成乙");
    s.save(&a).unwrap();
    s.save(&b).unwrap();
    s.conn.execute_batch(&format!("CREATE TRIGGER reject_t7 BEFORE UPDATE OF sensitive ON items WHEN old.id='{}' BEGIN SELECT RAISE(ABORT,'synthetic'); END;",b.hash())).unwrap();
    assert!(s.set_sensitive(&[a.hash(), b.hash()], true, 10).is_err());
    assert_eq!(state(&s, &a.hash()), (0, None));
    assert_eq!(indexed(&s, &a.hash()).0, "合成甲");
}

#[test]
fn unreadable_batch_payload_is_rejected_before_any_mark_or_unmark_write() {
    for value in [true, false] {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        let first = text("合成第一条");
        let second = text(&"合成大文本".repeat(10000));
        store.save(&first).unwrap();
        store.save(&second).unwrap();
        let ids = [first.hash(), second.hash()];
        if !value {
            store.set_sensitive(&ids, true, 10).unwrap();
        }
        let before = (state(&store, &ids[0]), indexed(&store, &ids[0]));
        let view: String = store
            .conn
            .query_row("SELECT view FROM items WHERE id=?1", [&ids[0]], |row| {
                row.get(0)
            })
            .unwrap();
        let blob: String = store
            .conn
            .query_row("SELECT blob FROM formats WHERE item=?1", [&ids[1]], |row| {
                row.get(0)
            })
            .unwrap();
        fs::remove_file(store.blobs.join(blob)).unwrap();
        store.conn.execute_batch(&format!("CREATE TRIGGER detect_early_write BEFORE UPDATE OF sensitive ON items WHEN old.id='{}' BEGIN SELECT RAISE(ABORT,'TEST_ONLY_EARLY_WRITE'); END;", ids[0])).unwrap();
        let error = store.set_sensitive(&ids, value, 10).unwrap_err();
        assert!(!error.to_string().contains("TEST_ONLY_EARLY_WRITE"));
        assert_eq!((state(&store, &ids[0]), indexed(&store, &ids[0])), before);
        let after: String = store
            .conn
            .query_row("SELECT view FROM items WHERE id=?1", [&ids[0]], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(after, view);
    }
}
#[test]
fn high_entropy_setting_roundtrip_and_next_capture() {
    let f = Fixture::new();
    let mut s = f.open();
    let c = text("aB3!cD4@eF5#gH6$jK7%Lm8&");
    s.save(&c).unwrap();
    assert_eq!(state(&s, &c.hash()).0, 0);
    s.save_settings(&Settings {
        sensitive_high_entropy: true,
        ..Default::default()
    })
    .unwrap();
    drop(s);
    let mut s = f.open();
    assert!(s.load_settings().unwrap().sensitive_high_entropy);
    s.save(&c).unwrap();
    assert_eq!(state(&s, &c.hash()).0, 1);
}
#[test]
fn files_and_images_not_detected() {
    let f = Fixture::new();
    let mut s = f.open();
    for name in ["CF_HDROP", "PNG"] {
        let mut c = text(&token());
        c.formats.push(model::FormatData {
            name: name.into(),
            bytes: vec![],
        });
        s.save(&c).unwrap();
        assert_eq!(state(&s, &c.hash()).0, 0);
    }
}
#[test]
fn rich_plain_text_detected() {
    let f = Fixture::new();
    let mut s = f.open();
    let mut c = text(&token());
    c.formats.push(model::FormatData {
        name: "HTML Format".into(),
        bytes: b"<b>synthetic</b>".to_vec(),
    });
    s.save(&c).unwrap();
    assert_eq!(state(&s, &c.hash()).0, 1);
    assert!(!s.display_detail(&c.hash()).unwrap().text.contains(&token()));
}
#[test]
fn second_dedup_check_honors_user_override() {
    let f = Fixture::new();
    let mut s = f.open();
    let c = text(&token());
    let mut prepared = PreparedSave::new(&c, SaveIdentity::new(&c), &s.key).unwrap();
    prepared.detection = sensitivity::Detection::for_clip(&c, &Settings::default());
    s.save(&c).unwrap();
    s.set_sensitive(&[c.hash()], false, 1).unwrap();
    assert!(!s.commit_prepared(prepared).unwrap().inserted);
    assert_eq!(state(&s, &c.hash()), (-1, None));
}
#[test]
fn long_sensitive_suffix_not_sent_to_detail() {
    let f = Fixture::new();
    let mut s = f.open();
    let c = text(&format!(
        "{} {} TAIL_SYNTHETIC_SECRET",
        token(),
        " ".repeat(65536)
    ));
    s.save(&c).unwrap();
    assert!(!s
        .display_detail(&c.hash())
        .unwrap()
        .text
        .contains("TAIL_SYNTHETIC_SECRET"));
}
#[test]
fn invalid_ttl_rejected_without_changes() {
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    let before = state(&s, &id);
    assert!(s
        .set_sensitive(std::slice::from_ref(&id), true, 1441)
        .is_err());
    assert_eq!(state(&s, &id), before);
}
#[test]
fn shared_capture_updates_source_and_refreshes_expiry() {
    let f = Fixture::new();
    let s = Mutex::new(f.open());
    let c = text(&token());
    let source = crate::capture_source::CaptureSource {
        exe: Some("synthetic.exe".into()),
        title: Some("合成标题".into()),
    };
    let first = Storage::save_shared_with_source(&s, &c, Some(&source)).unwrap();
    assert!(first.inserted);
    s.lock()
        .unwrap()
        .conn
        .execute("UPDATE items SET expires_at=1 WHERE id=?1", [&first.id])
        .unwrap();
    let second = Storage::save_shared_with_source(
        &s,
        &c,
        Some(&crate::capture_source::CaptureSource::default()),
    )
    .unwrap();
    assert!(!second.inserted);
    assert_eq!(first.id, second.id);
    let store = s.lock().unwrap();
    assert!(state(&store, &first.id).1.unwrap() > unix_millis());
    assert!(store.list().unwrap()[0].source_exe.is_none());
    assert_eq!(indexed(&store, &first.id), (String::new(), String::new()));
}
#[test]
fn expiry_reclaims_external_text_and_thumb_cannot_read_it() {
    let f = Fixture::new();
    let mut s = f.open();
    let c = text(&format!("{} {}", token(), "普通文字".repeat(20000)));
    s.save(&c).unwrap();
    let blob: String = s
        .conn
        .query_row("SELECT blob FROM formats WHERE item=?1", [c.hash()], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(s.blobs.join(&blob).is_file());
    assert!(s.read_thumbnail(&blob).is_err());
    let deadline = state(&s, &c.hash()).1.unwrap();
    let report = s.delete_expired(deadline).unwrap();
    assert_eq!(report.deleted_items, 1);
    assert_eq!(report.reclaimed_blobs, 1);
    assert!(!s.blobs.join(&blob).exists());
}
#[test]
fn second_dedup_check_refreshes_expiry() {
    let f = Fixture::new();
    let mut s = f.open();
    let c = text(&token());
    let mut prepared = PreparedSave::new(&c, SaveIdentity::new(&c), &s.key).unwrap();
    prepared.detection = sensitivity::Detection::for_clip(&c, &Settings::default());
    s.save(&c).unwrap();
    s.conn
        .execute("UPDATE items SET expires_at=1 WHERE id=?1", [c.hash()])
        .unwrap();
    assert!(!s.commit_prepared(prepared).unwrap().inserted);
    assert!(state(&s, &c.hash()).1.unwrap() > unix_millis());
    assert!(!s.list().unwrap()[0].summary.contains(&token()));
}

// AC-2/3/6：检查 FTS 的实际内容副本，而不仅检查 items 派生列。
fn assert_t7_indexes(s: &Storage, id: &str, expected: (&str, &str)) {
    let columns = indexed(s, id);
    let fts: (String, String) = s.conn.query_row(
        "SELECT f.search_text,f.search_pinyin FROM items_fts f JOIN items_fts_map m ON f.rowid=m.search_id WHERE m.item_id=?1",
        [id], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(columns, (expected.0.into(), expected.1.into()));
    assert_eq!(fts, columns);
    s.conn
        .execute_batch("INSERT INTO items_fts(items_fts) VALUES('integrity-check')")
        .unwrap();
}

#[test]
fn automatic_manual_and_negative_state_keep_real_fts_in_sync() {
    let f = Fixture::new();
    let mut s = f.open();
    let fake = "sk-TESTONLY0000000000000000abcd";
    let c = text(&format!("前文 {fake} 后文"));
    let id = c.hash();
    s.save(&c).unwrap();
    for manual in [false, true] {
        if manual {
            s.set_sensitive(std::slice::from_ref(&id), true, 10)
                .unwrap();
        }
        assert_t7_indexes(&s, &id, ("", ""));
        for query in ["sk", "TESTONLY", "前文", "qianwen"] {
            let filter = ListFilter {
                query: Some(query.into()),
                ..Default::default()
            };
            assert!(s
                .list_page(&filter, 0, 100)
                .unwrap()
                .iter()
                .all(|v| v.id != id));
        }
        let view: String = s
            .conn
            .query_row("SELECT view FROM items WHERE id=?1", [&id], |r| r.get(0))
            .unwrap();
        assert!(!view.contains(fake));
        for json in [
            serde_json::to_string(&s.list().unwrap()).unwrap(),
            serde_json::to_string(&s.display_detail(&id).unwrap()).unwrap(),
        ] {
            assert!(!json.contains(fake));
        }
        assert_eq!(s.load(&id).unwrap().formats, c.formats);
        s.set_sensitive(std::slice::from_ref(&id), false, 10)
            .unwrap();
        let restored = indexed(&s, &id);
        assert!(restored.0.contains(fake));
        assert_t7_indexes(&s, &id, (&restored.0, &restored.1));
        s.save(&c).unwrap();
        assert_eq!(state(&s, &id), (-1, None));
        let results = s
            .list_page(
                &ListFilter {
                    query: Some("TESTONLY qianwen".into()),
                    ..Default::default()
                },
                0,
                100,
            )
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
    }
}

#[test]
fn html_and_rtf_formats_survive_masked_detail_and_explicit_reveal() {
    let f = Fixture::new();
    let mut s = f.open();
    let fake = "sk-TESTONLY1111111111111111abcd";
    let mut c = text(&format!("富文本前文 {fake} 后文"));
    c.formats.extend([
        model::FormatData {
            name: "HTML Format".into(),
            bytes: format!("<b>TEST_ONLY {fake}</b>").into_bytes(),
        },
        model::FormatData {
            name: "Rich Text Format".into(),
            bytes: format!("{{\\rtf1 TEST_ONLY {fake}}}").into_bytes(),
        },
    ]);
    s.save(&c).unwrap();
    let id = c.hash();
    let masked = s.display_detail(&id).unwrap();
    assert!(!serde_json::to_string(&masked).unwrap().contains(fake));
    assert!(masked.text.contains("富文本前文"));
    assert!(masked.files.is_empty());
    assert!(s.reveal_detail(&id).unwrap().text.contains(fake));
    let restored = s.load(&id).unwrap();
    for format in &c.formats {
        assert_eq!(restored.get(&format.name), Some(format.bytes.as_slice()));
    }
    assert_t7_indexes(&s, &id, ("", ""));
}

// 无 CF_UNICODETEXT 的 HTML 才会被现有 Clip::kind 分类为 rich。
#[test]
fn html_only_rich_manual_mark_has_safe_detail_and_preserves_original_format() {
    let f = Fixture::new();
    let mut s = f.open();
    let fake = "sk-TESTONLY8888888888888888abcd";
    let c = Clip {
        formats: vec![model::FormatData {
            name: "HTML Format".into(),
            bytes: format!("<b>TEST_ONLY {fake}</b>").into_bytes(),
        }],
    };
    assert_eq!(c.kind(), "rich");
    s.save(&c).unwrap();
    s.set_sensitive(&[c.hash()], true, 10).unwrap();
    let item = s
        .list_page(
            &ListFilter {
                kind: Some("text".into()),
                ..Default::default()
            },
            0,
            10,
        )
        .unwrap()
        .remove(0);
    assert_eq!(item.kind, "rich");
    assert!(item.sensitive && item.expires_at.is_some());
    for json in [
        serde_json::to_string(&item).unwrap(),
        serde_json::to_string(&s.display_detail(&c.hash()).unwrap()).unwrap(),
    ] {
        assert!(!json.contains(fake));
    }
    assert_t7_indexes(&s, &c.hash(), ("", ""));
    assert_eq!(s.load(&c.hash()).unwrap().formats, c.formats);
}

#[test]
fn fixed_capture_time_sets_ttl_and_respects_all_existing_protections() {
    const NOW: i64 = 1_800_000_000_000;
    for protection in 0..4 {
        let f = Fixture::new();
        let mut s = f.open();
        let c = text("sk-TESTONLY2222222222222222abcd");
        s.save_settings(&Settings {
            sensitive_detection: false,
            ..Default::default()
        })
        .unwrap();
        s.save(&c).unwrap();
        let ids = vec![c.hash()];
        match protection {
            1 => {
                s.set_pinned(&ids, true).unwrap();
            }
            2 => {
                s.set_favorite(&ids, true).unwrap();
            }
            3 => {
                let g = s.create_group("TEST_ONLY 保护").unwrap();
                s.move_to_group(&ids, Some(g.id)).unwrap();
            }
            _ => {}
        }
        let detection = sensitivity::Detection::for_clip(
            &c,
            &Settings {
                sensitive_ttl_minutes: 7,
                ..Default::default()
            },
        )
        .unwrap();
        sensitivity::apply_detection(&s.conn, &ids[0], &detection, NOW).unwrap();
        assert_eq!(
            state(&s, &ids[0]),
            (
                1,
                if protection == 0 {
                    Some(NOW + 420_000)
                } else {
                    None
                }
            )
        );
        assert_t7_indexes(&s, &ids[0], ("", ""));
    }
}

// AC-3/6：真实可解码的小图；不调用任何系统剪贴板 API。
#[test]
fn manual_image_mark_hides_both_previews_and_unmark_restores_them() {
    let f = Fixture::new();
    let mut s = f.open();
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        4,
        4,
        image::Rgba([20, 80, 160, 255]),
    ))
    .write_to(&mut encoded, image::ImageFormat::Png)
    .unwrap();
    let c = Clip {
        formats: vec![model::FormatData {
            name: "PNG".into(),
            bytes: encoded.into_inner(),
        }],
    };
    s.save(&c).unwrap();
    let original = s.list().unwrap().remove(0);
    let thumbnail = original.thumbnail.as_ref().unwrap();
    let preview = original.preview.as_ref().unwrap();
    assert!(s.read_thumbnail(thumbnail).is_ok());
    assert!(s.read_thumbnail(preview).is_ok());
    s.set_sensitive(&[c.hash()], true, 10).unwrap();
    let marked = s.list().unwrap().remove(0);
    assert_eq!(state(&s, &c.hash()).0, 1);
    assert!(marked.expires_at.is_some());
    assert!(marked.thumbnail.is_none() && marked.preview.is_none() && marked.placeholder.is_none());
    assert!(s.read_thumbnail(thumbnail).is_err());
    assert!(s.read_thumbnail(preview).is_err());
    assert!(s.display_detail(&c.hash()).unwrap().files.is_empty());
    assert_eq!(s.load(&c.hash()).unwrap().formats, c.formats);
    s.set_sensitive(&[c.hash()], false, 10).unwrap();
    let unmarked = s.list().unwrap().remove(0);
    assert_eq!(state(&s, &c.hash()), (-1, None));
    assert_eq!(unmarked.thumbnail, original.thumbnail);
    assert_eq!(unmarked.preview, original.preview);
    assert!(s.read_thumbnail(thumbnail).is_ok());
}

#[test]
fn manual_file_mark_hides_paths_then_restores_filename_and_path_search() {
    let f = Fixture::new();
    let mut s = f.open();
    let path =
        f.0.join("TEST_ONLY_file.txt")
            .to_string_lossy()
            .into_owned();
    let c = Clip {
        formats: vec![model::FormatData {
            name: "CF_HDROP".into(),
            bytes: model::encode_hdrop(std::slice::from_ref(&path)),
        }],
    };
    s.save(&c).unwrap();
    s.set_sensitive(&[c.hash()], true, 10).unwrap();
    let item = s.list().unwrap().remove(0);
    assert!(item.sensitive && item.expires_at.is_some());
    assert!(item.files.is_empty());
    assert!(s.display_detail(&c.hash()).unwrap().files.is_empty());
    assert!(!serde_json::to_string(&item).unwrap().contains(&path));
    assert_t7_indexes(&s, &c.hash(), ("", ""));
    s.set_sensitive(&[c.hash()], false, 10).unwrap();
    assert_eq!(state(&s, &c.hash()), (-1, None));
    assert_eq!(
        s.list().unwrap()[0].files.as_slice(),
        std::slice::from_ref(&path)
    );
    assert!(indexed(&s, &c.hash()).0.contains(&path));
    assert_eq!(
        s.list_page(
            &ListFilter {
                query: Some("TEST_ONLY_file".into()),
                ..Default::default()
            },
            0,
            10
        )
        .unwrap()
        .len(),
        1
    );
    s.save(&c).unwrap();
    assert_eq!(state(&s, &c.hash()), (-1, None));
}

#[test]
fn batch_mark_ignores_previously_deleted_and_duplicate_ids() {
    let f = Fixture::new();
    let mut s = f.open();
    let live = text("TEST_ONLY 活记录");
    let deleted = text("TEST_ONLY 已删除");
    s.save(&live).unwrap();
    s.save(&deleted).unwrap();
    s.delete(&[deleted.hash()]).unwrap();
    let ids = [
        deleted.hash(),
        live.hash(),
        live.hash(),
        "TEST_ONLY_MISSING".into(),
    ];
    assert_eq!(s.set_sensitive(&ids, true, 1).unwrap(), 1);
    assert_eq!(s.set_sensitive(&ids, false, 1).unwrap(), 1);
    assert_eq!(state(&s, &live.hash()), (-1, None));
    assert_eq!(s.set_sensitive(&[], true, 1).unwrap(), 0);
    assert_eq!(s.list().unwrap().len(), 1);
}

#[test]
fn same_deadline_deletes_all_due_items_and_keeps_later_and_retained_items() {
    const NOW: i64 = 1_800_000_000_000;
    let f = Fixture::new();
    let mut s = f.open();
    let clips: Vec<_> = (0..5)
        .map(|i| text(&format!("sk-TESTONLY3333333333333333abcd case={i}")))
        .collect();
    for c in &clips {
        s.save(c).unwrap();
    }
    for c in &clips[..3] {
        s.conn
            .execute(
                "UPDATE items SET expires_at=?2 WHERE id=?1",
                params![c.hash(), NOW],
            )
            .unwrap();
    }
    s.conn
        .execute(
            "UPDATE items SET expires_at=?2 WHERE id=?1",
            params![clips[3].hash(), NOW + 1],
        )
        .unwrap();
    s.set_favorite(&[clips[4].hash()], true).unwrap();
    assert_eq!(s.next_expiry().unwrap(), Some(NOW));
    let report = s.delete_expired(NOW).unwrap();
    assert_eq!(report.deleted_items, 3);
    assert_eq!(
        report.deleted_ids.into_iter().collect::<HashSet<_>>(),
        clips[..3].iter().map(|c| c.hash()).collect()
    );
    assert_eq!(s.next_expiry().unwrap(), Some(NOW + 1));
    assert_eq!(s.delete_expired(NOW).unwrap().deleted_items, 0);
    assert_eq!(s.delete_expired(NOW + 1).unwrap().deleted_items, 1);
    assert_eq!(s.list().unwrap()[0].id, clips[4].hash());
}

#[test]
fn expiry_and_capacity_cleanup_counts_agree_in_either_order() {
    const NOW: i64 = 1_800_000_000_000;
    for expiry_first in [false, true] {
        let f = Fixture::new();
        let mut s = f.open();
        let clips: Vec<_> = (0..5)
            .map(|i| text(&format!("TEST_ONLY 容量样例 {i}")))
            .collect();
        for c in &clips {
            s.save(c).unwrap();
        }
        for c in [&clips[0], &clips[4]] {
            s.conn
                .execute(
                    "UPDATE items SET sensitive=1,expires_at=?2 WHERE id=?1",
                    params![c.hash(), NOW],
                )
                .unwrap();
        }
        let policy = RetentionPolicy {
            max_items: 2,
            max_days: 0,
            ..Default::default()
        };
        let first = if expiry_first {
            s.delete_expired(NOW)
        } else {
            s.cleanup(&policy, NOW)
        }
        .unwrap();
        let second = if expiry_first {
            s.cleanup(&policy, NOW)
        } else {
            s.delete_expired(NOW)
        }
        .unwrap();
        assert_eq!(first.deleted_items + second.deleted_items, 3);
        let deleted: HashSet<_> = first
            .deleted_ids
            .iter()
            .chain(&second.deleted_ids)
            .collect();
        assert_eq!(deleted.len(), 3);
        assert_eq!(
            s.list()
                .unwrap()
                .iter()
                .map(|v| v.id.clone())
                .collect::<HashSet<_>>(),
            [clips[2].hash(), clips[3].hash()].into_iter().collect()
        );
        assert_eq!(s.preview_cleanup(&policy, NOW).unwrap(), 0);
    }
}

#[test]
fn reopen_then_startup_cleanup_removes_due_newest_before_listing() {
    const NOW: i64 = 1_800_000_000_000;
    let f = Fixture::new();
    let mut s = f.open();
    let id = save(&mut s);
    s.conn
        .execute(
            "UPDATE items SET expires_at=?2 WHERE id=?1",
            params![id, NOW],
        )
        .unwrap();
    drop(s);
    let mut s = f.open();
    // 与 runtime::start 相同的存储调用；不启动应用、热键或桌面监听。
    assert_eq!(
        s.cleanup(&Settings::default().retention_policy(), NOW)
            .unwrap()
            .deleted_items,
        1
    );
    assert!(s.list().unwrap().is_empty());
    assert!(s.next_expiry().unwrap().is_none());
    assert_eq!(
        s.conn
            .query_row("SELECT COUNT(*) FROM items_fts_map", [], |r| r
                .get::<_, usize>(0))
            .unwrap(),
        0
    );
}

#[test]
fn expiry_reclaims_shared_blob_only_after_its_last_reference_is_deleted() {
    const NOW: i64 = 1_800_000_000_000;
    let f = Fixture::new();
    let mut s = f.open();
    let first = text(&format!(
        "sk-TESTONLY4444444444444444abcd {}",
        " TEST_ONLY".repeat(8000)
    ));
    let mut second = first.clone();
    second.formats.push(model::FormatData {
        name: "HTML Format".into(),
        bytes: b"<b>TEST_ONLY</b>".to_vec(),
    });
    s.save(&first).unwrap();
    s.save(&second).unwrap();
    s.set_pinned(&[second.hash()], true).unwrap();
    let blobs: Vec<String> = s
        .conn
        .prepare("SELECT blob FROM formats WHERE name='CF_UNICODETEXT'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(blobs.len(), 2);
    assert_eq!(blobs[0], blobs[1]);
    let blob = s.blobs.join(&blobs[0]);
    s.conn
        .execute(
            "UPDATE items SET expires_at=?2 WHERE id=?1",
            params![first.hash(), NOW],
        )
        .unwrap();
    let report = s.delete_expired(NOW).unwrap();
    assert_eq!(report.deleted_items, 1);
    assert_eq!(report.reclaimed_blobs, 0);
    assert!(blob.is_file());
    assert_eq!(
        s.load(&second.hash()).unwrap().get("CF_UNICODETEXT"),
        first.get("CF_UNICODETEXT")
    );
    let report = s.delete(&[second.hash()]).unwrap();
    assert_eq!(report.reclaimed_blobs, 1);
    assert!(!blob.exists());
}

#[test]
fn unmark_transaction_failure_restores_all_states_views_and_fts() {
    let f = Fixture::new();
    let mut s = f.open();
    let a = text("sk-TESTONLY5555555555555555abcd");
    let b = text("sk-TESTONLY6666666666666666abcd");
    s.save(&a).unwrap();
    s.save(&b).unwrap();
    let before = [state(&s, &a.hash()), state(&s, &b.hash())];
    s.conn.execute_batch(&format!("CREATE TRIGGER reject_t7_unmark BEFORE UPDATE OF sensitive ON items WHEN old.id='{}' AND new.sensitive=-1 BEGIN SELECT RAISE(ABORT,'TEST_ONLY'); END;", b.hash())).unwrap();
    assert!(s.set_sensitive(&[a.hash(), b.hash()], false, 10).is_err());
    assert_eq!([state(&s, &a.hash()), state(&s, &b.hash())], before);
    for c in [&a, &b] {
        assert_t7_indexes(&s, &c.hash(), ("", ""));
        assert!(!s
            .display_detail(&c.hash())
            .unwrap()
            .text
            .contains(&c.text().unwrap()));
    }
}

#[test]
fn unmark_non_object_view_rolls_back_prior_updates() {
    for invalid in ["[]", "null", "42", "\"invalid\""] {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        let first = text("sk-TESTONLY5555555555555555abcd");
        let second = text("sk-TESTONLY6666666666666666abcd");
        store.save(&first).unwrap();
        store.save(&second).unwrap();
        let before = [state(&store, &first.hash()), state(&store, &second.hash())];
        store
            .conn
            .execute(
                "UPDATE items SET view=?2 WHERE id=?1",
                params![second.hash(), invalid],
            )
            .unwrap();
        assert!(store
            .set_sensitive(&[first.hash(), second.hash()], false, 10)
            .is_err());
        assert_eq!(
            [state(&store, &first.hash()), state(&store, &second.hash())],
            before
        );
        assert_t7_indexes(&store, &first.hash(), ("", ""));
        assert_t7_indexes(&store, &second.hash(), ("", ""));
    }
}

#[test]
fn detail_reveal_keeps_corrupt_view_behavior_and_storage_state() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let clip = text(&token());
    let id = clip.hash();
    storage.save(&clip).unwrap();
    let before_state = state(&storage, &id);
    let before_index = indexed(&storage, &id);
    storage
        .conn
        .execute("UPDATE items SET view='invalid-json' WHERE id=?1", [&id])
        .unwrap();
    assert!(storage.display_detail(&id).is_err());
    assert_eq!(storage.reveal_detail(&id).unwrap().text, token());
    assert_eq!(state(&storage, &id), before_state);
    assert_eq!(indexed(&storage, &id), before_index);
    storage
        .conn
        .execute("UPDATE items SET sensitive=-1 WHERE id=?1", [&id])
        .unwrap();
    assert_eq!(storage.display_detail(&id).unwrap().text, token());
    assert!(storage.display_detail("missing").is_err());
    assert!(storage.reveal_detail("missing").is_err());
}

#[test]
fn rich_html_preview_is_hidden_until_reveal_and_payload_roundtrips() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let clip = crate::model::Clip {
        formats: vec![
            crate::model::FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: "合成图文"
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            },
            crate::model::FormatData {
                name: "HTML Format".into(),
                bytes: b"before<img src='file:///C:/synthetic/missing.png'>after".to_vec(),
            },
        ],
    };
    storage.save(&clip).unwrap();
    let id = clip.hash();
    assert!(storage.display_detail(&id).unwrap().rich.is_some());
    storage
        .set_sensitive(std::slice::from_ref(&id), true, 0)
        .unwrap();
    assert!(storage.display_detail(&id).unwrap().rich.is_none());
    assert!(storage.reveal_detail(&id).unwrap().rich.is_some());
    assert_eq!(storage.load(&id).unwrap().formats, clip.formats);
    let shared = std::sync::Mutex::new(storage);
    let (_, masked) = crate::item_detail::read::display(&shared, &id).unwrap();
    assert!(masked.rich.is_none());
    let (_, mut revealed) = crate::item_detail::read::reveal(&shared, &id).unwrap();
    let preview = revealed.rich.take().unwrap();
    assert!(serde_json::to_string(&preview)
        .unwrap()
        .contains("原图片已失效"));
    assert!(!serde_json::to_string(&preview)
        .unwrap()
        .contains("C:/synthetic"));
}
