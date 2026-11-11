use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../.test-data")
                .join(format!(
                    "t5-groups-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                )),
        )
    }
    fn open(&self) -> Storage {
        Storage::open(&self.0, &[85; 32]).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn seed(s: &mut Storage, n: usize) -> Vec<String> {
    (0..n)
        .map(|i| {
            let clip = Clip {
                formats: vec![model::FormatData {
                    name: "CF_UNICODETEXT".into(),
                    bytes: format!("Hello 世界 sample {i}")
                        .encode_utf16()
                        .chain([0])
                        .flat_map(u16::to_le_bytes)
                        .collect(),
                }],
            };
            s.save(&clip).unwrap();
            clip.hash()
        })
        .collect()
}
fn ids(s: &Storage, filter: &ListFilter, offset: usize, limit: usize) -> Vec<String> {
    s.list_page(filter, offset, limit)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect()
}
fn foreign_keys_ok(s: &Storage) {
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

#[test]
fn flags_batch_counts_duplicates_missing_and_noops() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 3);
    let requested = vec![
        all[0].clone(),
        all[1].clone(),
        all[0].clone(),
        "missing".into(),
    ];
    assert_eq!(s.set_pinned(&requested, true).unwrap(), 2);
    assert_eq!(s.set_pinned(&requested, true).unwrap(), 0);
    assert_eq!(s.set_pinned(&[], false).unwrap(), 0);
    assert_eq!(s.set_favorite(&requested, true).unwrap(), 2);
    assert_eq!(s.set_favorite(&requested, true).unwrap(), 0);
    assert_eq!(s.set_favorite(&requested, false).unwrap(), 2);
    assert_eq!(s.set_pinned(&requested, false).unwrap(), 2);
}
#[test]
fn pin_sort_and_unpin_restore_order_without_touch() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 4);
    let before = ids(&s, &ListFilter::default(), 0, 20);
    let timestamps = |s: &Storage| {
        s.conn
            .prepare("SELECT id,touched,used_at FROM items ORDER BY id")
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let times = timestamps(&s);
    s.set_pinned(&all[..2], true).unwrap();
    assert_eq!(
        ids(&s, &ListFilter::default(), 0, 20),
        vec![
            all[1].clone(),
            all[0].clone(),
            all[3].clone(),
            all[2].clone()
        ]
    );
    s.set_pinned(&all[..2], false).unwrap();
    assert_eq!(ids(&s, &ListFilter::default(), 0, 20), before);
    assert_eq!(timestamps(&s), times);
}
#[test]
fn quick_cache_loader_is_all_history_with_pins_first() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 205);
    let g = s.create_group("缓存分组").unwrap();
    s.move_to_group(&all[..1], Some(g.id)).unwrap();
    s.set_pinned(&all[..1], true).unwrap();
    s.set_favorite(&all[1..2], true).unwrap();
    let cache = s.list().unwrap();
    assert_eq!(cache.len(), QUICK_PAGE_SIZE);
    assert_eq!(cache[0].id, all[0]);
    assert_eq!(cache[1].id, all[204]);
    assert_eq!(cache[0].group_id, Some(g.id));
    assert_eq!(
        cache.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
        ids(&s, &ListFilter::default(), 0, QUICK_PAGE_SIZE)
    );
}
#[test]
fn flags_and_group_persist_after_reopen() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 1);
    let g = s.create_group("持久化").unwrap();
    s.set_pinned(&all, true).unwrap();
    s.set_favorite(&all, true).unwrap();
    s.move_to_group(&all, Some(g.id)).unwrap();
    drop(s);
    let s = f.open();
    let row = &s.list().unwrap()[0];
    assert!(row.pinned && row.favorite);
    assert_eq!(row.group_id, Some(g.id));
    assert_eq!(s.list_groups().unwrap()[0].count, 1);
}
#[test]
fn create_trim_names_and_order_counts() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 2);
    let a = s.create_group(" \u{a0}工作\u{3000} ").unwrap();
    let b = s.create_group("私人").unwrap();
    assert_eq!(a.name, "工作");
    assert!(a.sort < b.sort);
    s.move_to_group(&all, Some(a.id)).unwrap();
    let groups = s.list_groups().unwrap();
    assert_eq!(
        groups.iter().map(|g| (g.id, g.count)).collect::<Vec<_>>(),
        vec![(a.id, 2), (b.id, 0)]
    );
}
#[test]
fn names_validate_empty_and_unicode_char_length() {
    let f = Fixture::new();
    let mut s = f.open();
    for name in ["", " \u{a0} ", "　"] {
        assert!(s
            .create_group(name)
            .unwrap_err()
            .to_string()
            .contains("1 到 40"));
    }
    assert!(s.create_group(&"名".repeat(40)).is_ok());
    assert!(s.create_group(&"名".repeat(41)).is_err());
    assert!(s.create_group(&"😀".repeat(40)).is_ok());
    assert!(s.create_group(&"😀".repeat(41)).is_err());
}
#[test]
fn group_names_are_case_insensitive_including_unicode() {
    let f = Fixture::new();
    let mut s = f.open();
    for (first, duplicate) in [("Work", " work "), ("ÉTÉ", "été"), ("РАБОТА", "работа")]
    {
        s.create_group(first).unwrap();
        assert!(s
            .create_group(duplicate)
            .unwrap_err()
            .to_string()
            .contains("同名"));
    }
}
#[test]
fn rename_validates_duplicate_and_preserves_id_sort_items() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 1);
    let a = s.create_group("Work").unwrap();
    let b = s.create_group("Other").unwrap();
    s.move_to_group(&all, Some(a.id)).unwrap();
    s.rename_group(a.id, " work ").unwrap();
    for name in ["", "OTHER"] {
        assert!(s.rename_group(a.id, name).is_err());
    }
    assert!(s.rename_group(a.id, &"x".repeat(41)).is_err());
    assert!(s.rename_group(999, "不存在").is_err());
    let groups = s.list_groups().unwrap();
    assert_eq!(groups[0].name, "work");
    assert_eq!(groups[0].sort, a.sort);
    assert_eq!(groups[0].count, 1);
    assert_eq!(groups[1].id, b.id);
}
#[test]
fn group_limit_is_100_but_rename_and_replacement_allowed() {
    let f = Fixture::new();
    let mut s = f.open();
    for i in 0..100 {
        s.create_group(&format!("分组{i}")).unwrap();
    }
    assert!(s
        .create_group("第101组")
        .unwrap_err()
        .to_string()
        .contains("100"));
    let g = s.list_groups().unwrap()[0].id;
    s.rename_group(g, "新名字").unwrap();
    s.delete_group(g).unwrap();
    s.create_group("替补").unwrap();
    assert_eq!(s.list_groups().unwrap().len(), 100);
}
#[test]
fn delete_group_unsets_references_without_deleting_records_or_formats() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 3);
    let g = s.create_group("删除").unwrap();
    s.move_to_group(&all, Some(g.id)).unwrap();
    s.set_pinned(&all[..1], true).unwrap();
    s.set_favorite(&all[1..2], true).unwrap();
    s.delete_group(g.id).unwrap();
    assert!(s.list_groups().unwrap().is_empty());
    let rows = s.list().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|i| i.group_id.is_none()));
    for id in all {
        assert!(s.load(&id).unwrap().text().unwrap().contains("Hello"));
    }
    foreign_keys_ok(&s);
    assert!(s.delete_group(g.id).is_err());
}
#[test]
fn delete_group_rolls_back_item_updates_if_delete_fails() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 2);
    let g = s.create_group("事务").unwrap();
    s.move_to_group(&all, Some(g.id)).unwrap();
    s.conn.execute_batch("CREATE TRIGGER fail_group_delete BEFORE DELETE ON groups BEGIN SELECT RAISE(ABORT,'test rollback'); END;").unwrap();
    assert!(s.delete_group(g.id).is_err());
    assert!(s.list().unwrap().iter().all(|i| i.group_id == Some(g.id)));
    assert_eq!(s.list_groups().unwrap()[0].count, 2);
    foreign_keys_ok(&s);
}
#[test]
fn move_batch_and_remove_count_only_actual_changes() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 3);
    let a = s.create_group("A").unwrap();
    let b = s.create_group("B").unwrap();
    let requested = vec![
        all[0].clone(),
        all[1].clone(),
        all[0].clone(),
        "missing".into(),
    ];
    assert_eq!(s.move_to_group(&requested, Some(a.id)).unwrap(), 2);
    assert_eq!(s.move_to_group(&requested, Some(a.id)).unwrap(), 0);
    assert_eq!(s.move_to_group(&requested, Some(b.id)).unwrap(), 2);
    assert_eq!(s.move_to_group(&requested, None).unwrap(), 2);
    assert_eq!(s.move_to_group(&requested, None).unwrap(), 0);
    foreign_keys_ok(&s);
}
#[test]
fn nonexistent_group_rejected_before_any_modification() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 3);
    let g = s.create_group("原组").unwrap();
    s.move_to_group(&all[..1], Some(g.id)).unwrap();
    assert!(s
        .move_to_group(&all, Some(999))
        .unwrap_err()
        .to_string()
        .contains("不存在"));
    assert!(s.move_to_group(&[], Some(-1)).is_err());
    assert_eq!(s.list_groups().unwrap()[0].count, 1);
    assert_eq!(
        ids(
            &s,
            &ListFilter {
                group_id: Some(-1),
                ..Default::default()
            },
            0,
            100
        )
        .len(),
        2
    );
    foreign_keys_ok(&s);
}
#[test]
fn batch_mutations_roll_back_on_midway_failure() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 3);
    let g = s.create_group("事务").unwrap();
    s.conn.execute_batch(&format!("CREATE TRIGGER fail_update BEFORE UPDATE ON items WHEN old.id='{}' BEGIN SELECT RAISE(ABORT,'test rollback'); END;",all[1])).unwrap();
    assert!(s.set_pinned(&all, true).is_err());
    assert!(s.set_favorite(&all, true).is_err());
    assert!(s.move_to_group(&all, Some(g.id)).is_err());
    assert!(s
        .list()
        .unwrap()
        .iter()
        .all(|i| !i.pinned && !i.favorite && i.group_id.is_none()));
    foreign_keys_ok(&s);
}
fn check_pages(s: &Storage, filter: &ListFilter, expected: &[String]) {
    assert_eq!(ids(s, filter, 0, PAGE_LIMIT), expected);
    let paged = (0..expected.len() + 7)
        .step_by(7)
        .flat_map(|offset| ids(s, filter, offset, 7))
        .collect::<Vec<_>>();
    assert_eq!(paged, expected);
    assert!(ids(s, filter, 0, 0).is_empty());
    assert!(ids(s, filter, usize::MAX, 100).is_empty());
}
#[test]
fn favorite_query_kind_and_pagination_combine() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 130);
    s.set_favorite(&all[..120], true).unwrap();
    s.set_pinned(&all[..1], true).unwrap();
    for id in &all[60..] {
        s.conn
            .execute("UPDATE items SET kind='files' WHERE id=?1", [id])
            .unwrap();
    }
    let filter = ListFilter {
        query: Some("Hello 世界".into()),
        kind: Some("text".into()),
        favorite_only: true,
        ..Default::default()
    };
    let expected = std::iter::once(all[0].clone())
        .chain(all[1..60].iter().rev().cloned())
        .collect::<Vec<_>>();
    check_pages(&s, &filter, &expected);
}
#[test]
fn group_query_and_pagination_combine() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 125);
    let g = s.create_group("过滤").unwrap();
    s.move_to_group(&all[..110], Some(g.id)).unwrap();
    let filter = ListFilter {
        query: Some("hello sj".into()),
        group_id: Some(g.id),
        ..Default::default()
    };
    check_pages(
        &s,
        &filter,
        &all[..110].iter().rev().cloned().collect::<Vec<_>>(),
    );
    assert!(ids(
        &s,
        &ListFilter {
            query: Some("不存在".into()),
            ..filter
        },
        0,
        100
    )
    .is_empty());
}
#[test]
fn ungrouped_query_kind_favorites_and_pagination_combine() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 125);
    let g = s.create_group("过滤").unwrap();
    s.move_to_group(&all[..10], Some(g.id)).unwrap();
    s.set_favorite(&all[..120], true).unwrap();
    let filter = ListFilter {
        query: Some("Hello".into()),
        kind: Some("text".into()),
        favorite_only: true,
        group_id: Some(-1),
    };
    check_pages(
        &s,
        &filter,
        &all[10..120].iter().rev().cloned().collect::<Vec<_>>(),
    );
    check_pages(
        &s,
        &ListFilter {
            group_id: Some(-1),
            ..Default::default()
        },
        &all[10..].iter().rev().cloned().collect::<Vec<_>>(),
    );
}
#[test]
fn just_grouped_items_are_excluded_from_cleanup_count_and_bytes() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 5);
    let g = s.create_group("保护").unwrap();
    let policy = RetentionPolicy {
        max_items: 1,
        max_bytes: 1_000_000,
        max_days: 0,
    };
    assert_eq!(s.preview_cleanup(&policy, unix_millis()).unwrap(), 4);
    s.move_to_group(&all[..1], Some(g.id)).unwrap();
    s.set_pinned(&all[1..2], true).unwrap();
    s.set_favorite(&all[2..3], true).unwrap();
    assert_eq!(s.preview_cleanup(&policy, unix_millis()).unwrap(), 1);
    let newest_size = s
        .list()
        .unwrap()
        .iter()
        .find(|i| i.id == all[4])
        .unwrap()
        .size as u64;
    let policy = RetentionPolicy {
        max_items: 10,
        max_bytes: newest_size,
        max_days: 0,
    };
    assert_eq!(s.preview_cleanup(&policy, unix_millis()).unwrap(), 1);
    assert_eq!(s.cleanup(&policy, unix_millis()).unwrap().deleted_items, 1);
    assert!(s.load(&all[3]).is_err());
    for id in [&all[0], &all[1], &all[2], &all[4]] {
        assert!(s.load(id).is_ok());
    }
    foreign_keys_ok(&s);
}
#[test]
fn manual_delete_allows_pinned_favorite_and_grouped_records() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 3);
    let g = s.create_group("允许手删").unwrap();
    s.set_pinned(&all, true).unwrap();
    s.set_favorite(&all, true).unwrap();
    s.move_to_group(&all, Some(g.id)).unwrap();
    assert_eq!(s.delete(&all).unwrap().deleted_items, 3);
    assert_eq!(s.list_groups().unwrap()[0].count, 0);
    foreign_keys_ok(&s);
}

#[test]
fn names_fullwidth_and_diaeresis_case_duplicates_and_self_rename() {
    let f = Fixture::new();
    let mut s = f.open();
    for (upper, lower) in [("ＡＢＣ", "ａｂｃ"), ("Ä", "ä")] {
        let group = s.create_group(upper).unwrap();
        assert!(s.create_group(lower).is_err());
        s.rename_group(group.id, upper).unwrap();
        s.rename_group(group.id, &format!(" {lower} ")).unwrap();
        assert_eq!(
            s.list_groups()
                .unwrap()
                .iter()
                .find(|g| g.id == group.id)
                .unwrap()
                .name,
            lower
        );
        assert!(s.create_group(upper).is_err());
    }
    // 当前只做 Unicode 小写转换，不做宽度归一化、规范等价或完整 case folding。
    for name in ["ABC", "A\u{308}", "Straße", "STRASSE"] {
        s.create_group(name).unwrap();
    }
}

#[test]
fn names_limit_counts_unicode_scalars_for_create_and_rename() {
    let f = Fixture::new();
    let mut s = f.open();
    // 每个可见字形由两个 Unicode scalar 组成，共 40 chars、60 个 UTF-8 字节。
    let boundary = "a\u{308}".repeat(20);
    assert_eq!(boundary.chars().count(), 40);
    assert_eq!(boundary.len(), 60);
    let group = s.create_group(&boundary).unwrap();
    let too_long = format!("{boundary}b");
    assert!(s.create_group(&too_long).is_err());
    assert!(s.rename_group(group.id, &too_long).is_err());
    s.rename_group(group.id, &"😀".repeat(40)).unwrap();
    assert!(s.rename_group(group.id, &"😀".repeat(41)).is_err());
    assert_eq!(s.list_groups().unwrap()[0].name, "😀".repeat(40));
}

#[test]
fn names_collapse_whitespace_and_reject_controls_and_zero_width_characters() {
    let f = Fixture::new();
    let mut s = f.open();
    let group = s.create_group("  a  \u{a0}\u{3000}b  ").unwrap();
    assert_eq!(group.name, "a b");
    assert!(s.create_group("A   B").is_err());
    s.rename_group(group.id, " a\u{2003}  b ").unwrap();
    assert_eq!(s.list_groups().unwrap()[0].name, "a b");
    let other = s.create_group("other").unwrap();
    assert!(s.rename_group(other.id, " A  B ").is_err());

    // 首尾和内部都必须拒绝，不能先 trim 或折叠后放过控制字符。
    for c in [
        '\0', '\t', '\n', '\r', '\u{7f}', '\u{85}', '\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}',
        '\u{feff}',
    ] {
        for name in [
            format!("{c}ab"),
            format!("a{c}b"),
            format!("ab{c}"),
            c.to_string(),
        ] {
            assert!(s.create_group(&name).is_err(), "{name:?}");
            assert!(s.rename_group(group.id, &name).is_err(), "{name:?}");
        }
    }
    let groups = s.list_groups().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].name, "a b");
    assert_eq!(groups[1].name, "other");
}

#[test]
fn names_validate_length_after_collapsing_whitespace() {
    let f = Fixture::new();
    let mut s = f.open();
    let group = s.create_group(&format!("a{}b", " ".repeat(100))).unwrap();
    assert_eq!(group.name, "a b");
    s.rename_group(group.id, &format!("c{}d", "\u{3000}".repeat(100)))
        .unwrap();
    assert_eq!(s.list_groups().unwrap()[0].name, "c d");
    assert!(s.create_group(" \u{a0}\u{3000} ").is_err());
}

#[test]
fn invalid_group_filters_are_empty_and_moves_leave_existing_assignments_intact() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 3);
    let group = s.create_group("已有分组").unwrap();
    s.move_to_group(&all[..1], Some(group.id)).unwrap();
    let before = s
        .list()
        .unwrap()
        .into_iter()
        .map(|item| (item.id, item.group_id))
        .collect::<Vec<_>>();
    for invalid in [i64::MIN, -2, 0] {
        let filter = ListFilter {
            group_id: Some(invalid),
            ..Default::default()
        };
        assert!(ids(&s, &filter, 0, 100).is_empty());
        assert!(s.move_to_group(&all, Some(invalid)).is_err());
        assert!(s.move_to_group(&[], Some(invalid)).is_err());
        let after = s
            .list()
            .unwrap()
            .into_iter()
            .map(|item| (item.id, item.group_id))
            .collect::<Vec<_>>();
        assert_eq!(after, before);
        foreign_keys_ok(&s);
    }
}

#[test]
fn ungrouped_filter_sentinel_cannot_be_written_as_a_foreign_key() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 2);
    let group = s.create_group("哨兵隔离").unwrap();
    s.move_to_group(&all[..1], Some(group.id)).unwrap();
    let ungrouped = ListFilter {
        group_id: Some(-1),
        ..Default::default()
    };
    assert!(s.move_to_group(&all, Some(-1)).is_err());
    assert_eq!(ids(&s, &ungrouped, 0, 100), all[1..]);
    assert_eq!(s.list_groups().unwrap()[0].count, 1);
    foreign_keys_ok(&s);
    assert_eq!(s.move_to_group(&all, None).unwrap(), 1);
    assert_eq!(
        ids(&s, &ungrouped, 0, 100),
        [all[1].clone(), all[0].clone()]
    );
    foreign_keys_ok(&s);
}

#[test]
fn delete_and_move_interleavings_preserve_foreign_keys_and_item_counts() {
    for move_before_delete in [false, true] {
        let f = Fixture::new();
        let mut s = f.open();
        let all = seed(&mut s, 3);
        let a = s.create_group("待删除").unwrap();
        let b = s.create_group("保留").unwrap();
        s.move_to_group(&all[..1], Some(a.id)).unwrap();
        if move_before_delete {
            assert_eq!(s.move_to_group(&all, Some(a.id)).unwrap(), 2);
            foreign_keys_ok(&s);
        }
        s.delete_group(a.id).unwrap();
        foreign_keys_ok(&s);
        assert!(s.move_to_group(&all, Some(a.id)).is_err());
        assert!(s.list().unwrap().iter().all(|item| item.group_id.is_none()));
        foreign_keys_ok(&s);
        assert_eq!(s.move_to_group(&all, Some(b.id)).unwrap(), 3);
        foreign_keys_ok(&s);
        assert_eq!(s.list_groups().unwrap()[0].count, 3);
        assert_eq!(s.list().unwrap().len(), 3);
    }
}

#[test]
fn flags_empty_and_missing_only_batches_preserve_rows_for_both_values() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, 2);
    s.set_pinned(&all[..1], true).unwrap();
    s.set_favorite(&all[1..], true).unwrap();
    for value in [false, true] {
        assert_eq!(s.set_pinned(&[], value).unwrap(), 0);
        assert_eq!(s.set_favorite(&[], value).unwrap(), 0);
        let missing = vec!["missing".into(), "missing".into()];
        assert_eq!(s.set_pinned(&missing, value).unwrap(), 0);
        assert_eq!(s.set_favorite(&missing, value).unwrap(), 0);
    }
    let rows = s.list().unwrap();
    assert_eq!(rows.len(), 2);
    let first = rows.iter().find(|item| item.id == all[0]).unwrap();
    let second = rows.iter().find(|item| item.id == all[1]).unwrap();
    assert!(first.pinned && !first.favorite);
    assert!(!second.pinned && second.favorite);
}

fn assert_group_protection_lifecycle(delete_group: bool) {
    const NOW: i64 = 100 * 86_400_000;
    for policy in [
        RetentionPolicy {
            max_items: 1,
            max_bytes: 1_000_000,
            max_days: 0,
        },
        RetentionPolicy {
            max_items: 100,
            max_bytes: 1,
            max_days: 0,
        },
        RetentionPolicy {
            max_items: 100,
            max_bytes: 1_000_000,
            max_days: 1,
        },
    ] {
        let f = Fixture::new();
        let mut s = f.open();
        let all = seed(&mut s, 2);
        s.conn
            .execute("UPDATE items SET used_at=0,expires_at=NULL", [])
            .unwrap();
        let group = s.create_group("清理生命周期").unwrap();
        assert_eq!(s.preview_cleanup(&policy, NOW).unwrap(), 1);
        assert_eq!(s.move_to_group(&all[..1], Some(group.id)).unwrap(), 1);
        assert_eq!(s.preview_cleanup(&policy, NOW).unwrap(), 0);
        assert_eq!(s.cleanup(&policy, NOW).unwrap().deleted_items, 0);
        assert!(s.load(&all[0]).is_ok());
        if delete_group {
            s.delete_group(group.id).unwrap();
            assert!(s.list_groups().unwrap().is_empty());
        } else {
            assert_eq!(s.move_to_group(&all[..1], None).unwrap(), 1);
            assert_eq!(s.list_groups().unwrap()[0].count, 0);
        }
        assert_eq!(s.preview_cleanup(&policy, NOW).unwrap(), 1);
        assert!(s.load(&all[0]).is_ok()); // preview 不得执行删除。
        let report = s.cleanup(&policy, NOW).unwrap();
        assert_eq!(report.deleted_items, 1);
        assert_eq!(report.deleted_ids, all[..1]);
        assert!(s.load(&all[0]).is_err());
        assert_eq!(ids(&s, &ListFilter::default(), 0, 100), all[1..]);
        foreign_keys_ok(&s);
    }
}

#[test]
fn moving_out_of_group_restores_count_bytes_and_age_cleanup_eligibility() {
    assert_group_protection_lifecycle(false);
}

#[test]
fn deleting_group_restores_count_bytes_and_age_cleanup_eligibility() {
    assert_group_protection_lifecycle(true);
}

#[test]
fn quick_cache_over_200_pins_keeps_newest_pins_and_excludes_unpinned() {
    let f = Fixture::new();
    let mut s = f.open();
    let all = seed(&mut s, QUICK_PAGE_SIZE + 7);
    let pinned = &all[..QUICK_PAGE_SIZE + 5];
    assert_eq!(s.set_pinned(pinned, true).unwrap(), pinned.len());
    let cache = s.list().unwrap();
    assert_eq!(cache.len(), QUICK_PAGE_SIZE);
    assert!(cache.iter().all(|item| item.pinned));
    assert_eq!(
        cache.into_iter().map(|item| item.id).collect::<Vec<_>>(),
        pinned[5..].iter().rev().cloned().collect::<Vec<_>>()
    );
    // 超出缓存容量的置顶仍留在数据库，可由主窗口继续分页读取。
    assert_eq!(
        ids(&s, &ListFilter::default(), QUICK_PAGE_SIZE, 100),
        pinned[..5]
            .iter()
            .rev()
            .cloned()
            .chain(all[QUICK_PAGE_SIZE + 5..].iter().rev().cloned())
            .collect::<Vec<_>>()
    );
}
