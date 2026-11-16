use super::*;

#[test]
fn paste_load_rejects_expired_or_deleted_id_before_reading_formats() {
    let store = storage(2);
    store
        .conn
        .execute("UPDATE items SET expires_at=0 WHERE id='id-0000'", [])
        .unwrap();
    store
        .conn
        .execute("DELETE FROM items WHERE id='id-0001'", [])
        .unwrap();
    for id in ["id-0000", "id-0001", "missing"] {
        assert_eq!(
            store.load_for_paste(id).unwrap_err().to_string(),
            "记录已删除或到期，请重新选择"
        );
    }
}

#[test]
fn expired_rows_are_excluded_without_waiting_for_physical_cleanup() {
    let store = storage(3);
    store
        .conn
        .execute("UPDATE items SET expires_at=0 WHERE id='id-0001'", [])
        .unwrap();
    let ids = ["id-0000", "id-0001", "id-0002"];
    assert_eq!(store.panel_id_order().unwrap(), ["id-0002", "id-0000"]);
    assert_eq!(
        store.existing_panel_ids(&ids).unwrap(),
        ["id-0000".into(), "id-0002".into()].into_iter().collect()
    );
    let items = store.panel_items(&ids).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["id-0000", "id-0002"]
    );
    assert_eq!(
        store
            .conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row
                .get::<_, usize>(0))
            .unwrap(),
        3
    );
}

fn storage(count: usize) -> Storage {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE items (
            id TEXT PRIMARY KEY, touched INTEGER, pinned INTEGER DEFAULT 0,
            view TEXT, kind TEXT DEFAULT 'text', created INTEGER DEFAULT 0,
            used_at INTEGER DEFAULT 0, size INTEGER DEFAULT 0,
            favorite INTEGER DEFAULT 0, group_id INTEGER, source_exe TEXT,
            source_title TEXT DEFAULT 'synthetic title', sensitive INTEGER DEFAULT 0,
            expires_at INTEGER, thumbnail TEXT DEFAULT 'synthetic-thumb',
            preview TEXT DEFAULT 'synthetic-preview', is_link INTEGER NOT NULL DEFAULT 0
        )",
    )
    .unwrap();
    let view = serde_json::to_string(&ItemView {
        summary: "synthetic content".into(),
        placeholder: Some("synthetic placeholder".into()),
        files: vec!["synthetic-file".into()],
        ..Default::default()
    })
    .unwrap();
    let transaction = conn.unchecked_transaction().unwrap();
    for index in 0..count {
        transaction
            .execute(
                "INSERT INTO items(id,touched,view) VALUES (?1,?2,?3)",
                params![format!("id-{index:04}"), index, view],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    // 故意不创建 formats/blob；这些接口不得加载原始格式或解密文件。
    Storage {
        conn,
        blobs: PathBuf::new(),
        key: Zeroizing::new([0; 32]),
    }
}

#[test]
fn full_id_order_matches_paged_list_with_pinning_and_stable_ties() {
    let store = storage(1201);
    store
        .conn
        .execute("UPDATE items SET pinned=1 WHERE id='id-0000'", [])
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE items SET touched=2000 WHERE id IN ('id-0001','id-0002')",
            [],
        )
        .unwrap();
    let order = store.panel_id_order().unwrap();
    let paged: Vec<_> = (0..1201)
        .step_by(PAGE_LIMIT)
        .flat_map(|offset| {
            store
                .list_page(&ListFilter::default(), offset, PAGE_LIMIT)
                .unwrap()
        })
        .map(|item| item.id)
        .collect();
    assert_eq!(order, paged);
    assert_eq!(order.len(), 1201);
    assert_eq!(&order[..3], ["id-0000", "id-0001", "id-0002"]);
}

#[test]
fn index_does_not_parse_views_or_read_formats_and_blobs() {
    let store = storage(301);
    store
        .conn
        .execute("UPDATE items SET view='invalid json'", [])
        .unwrap();
    assert_eq!(store.panel_id_order().unwrap().len(), 301);
    assert_eq!(
        store
            .existing_panel_ids(&["id-0000", "id-0300"])
            .unwrap()
            .len(),
        2
    );
    assert!(store.panel_items(&["id-0000"]).is_err());
}

#[test]
fn validity_query_batches_more_than_sqlite_legacy_variable_limit() {
    let store = storage(1201);
    let order = store.panel_id_order().unwrap();
    let mut requested: Vec<_> = order.iter().map(String::as_str).collect();
    requested.extend(["missing", "id-0000", "' OR 1=1 --"]);
    assert_eq!(
        store.existing_panel_ids(&requested).unwrap(),
        order.iter().cloned().collect()
    );
    store
        .conn
        .execute("DELETE FROM items WHERE id='id-0000'", [])
        .unwrap();
    let remaining = store.existing_panel_ids(&requested).unwrap();
    assert_eq!(remaining.len(), 1200);
    assert!(!remaining.contains("id-0000"));
    store.conn.execute("DELETE FROM items", []).unwrap();
    assert!(store.existing_panel_ids(&requested).unwrap().is_empty());
}

#[test]
fn neighborhood_preserves_requested_order_and_skips_missing_records() {
    let store = storage(1001);
    let items = store
        .panel_items(&["id-1000", "id-0000", "missing", "id-0999"])
        .unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["id-1000", "id-0000", "id-0999"]
    );
    assert!(store.panel_items(&[]).unwrap().is_empty());
    assert!(store.panel_items(&["id-0000"; 15]).is_ok());
    assert!(store.panel_items(&["id-0000"; 16]).is_err());
}

#[test]
fn deep_sensitive_neighborhood_uses_masked_view_without_thumbnail_or_metadata() {
    let store = storage(1001);
    store
        .conn
        .execute("UPDATE items SET sensitive=1 WHERE id='id-0000'", [])
        .unwrap();
    let items = store.panel_items(&["id-0000", "id-0001"]).unwrap();
    let masked = &items[0];
    assert!(masked.sensitive);
    assert_eq!(masked.summary, "••••••••");
    assert!(masked.thumbnail.is_none());
    assert!(masked.preview.is_none());
    assert!(masked.placeholder.is_none());
    assert!(masked.source_title.is_none());
    assert!(masked.files.is_empty());
    assert_eq!(items[1].summary, "synthetic content");
    assert!(items[1].thumbnail.is_some());
}

#[test]
fn malformed_requested_view_returns_error_instead_of_partial_neighborhood() {
    let store = storage(301);
    store
        .conn
        .execute("UPDATE items SET view='invalid' WHERE id='id-0001'", [])
        .unwrap();
    assert!(store.panel_items(&["id-0000", "id-0001"]).is_err());
    assert_eq!(store.panel_items(&["id-0000"]).unwrap().len(), 1);
}

#[test]
fn empty_and_failed_index_are_distinct() {
    let store = storage(0);
    assert!(store.panel_id_order().unwrap().is_empty());
    store.conn.execute("DROP TABLE items", []).unwrap();
    assert!(store.panel_id_order().is_err());
    assert!(store.existing_panel_ids(&["missing"]).is_err());
    assert!(store.panel_items(&["missing"]).is_err());
}

#[test]
fn neighborhood_coordinator_roundtrip_reloads_deep_sensitive_view_and_removes_deleted_ids() {
    use crate::panel_neighborhood::{read, Coordinator, ViewState};
    use crate::panel_session::Direction;

    let store = storage(1001);
    let (mut panel, mut request) =
        Coordinator::open(1, store.panel_id_order().unwrap().into(), 5).unwrap();
    for _ in 0..800 {
        request = panel.navigate(Direction::Next).unwrap();
    }
    let selected_id = panel.selected_id().unwrap().to_owned();
    let stale = read(&store, request.clone());
    panel.apply(read(&store, request));
    assert!(matches!(panel.view(), ViewState::Ready { .. }));
    store
        .conn
        .execute("UPDATE items SET sensitive=1 WHERE id=?1", [&selected_id])
        .unwrap();
    store
        .conn
        .execute("DELETE FROM items WHERE id='id-1000'", [])
        .unwrap();
    let request = panel.history_changed().unwrap();
    panel.apply(stale);
    assert!(matches!(panel.view(), ViewState::Loading));
    let request = panel.apply(read(&store, request)).unwrap();
    assert_eq!(panel.total(), 1000);
    assert_eq!(panel.selected_id(), Some(selected_id.as_str()));
    panel.apply(read(&store, request));
    let ViewState::Ready { items, selected } = panel.view() else {
        panic!("not ready");
    };
    assert!(items[*selected].sensitive);
    assert_eq!(items[*selected].summary, "••••••••");
    assert!(items[*selected].thumbnail.is_none());
    store
        .conn
        .execute("DELETE FROM items WHERE id=?1", [&selected_id])
        .unwrap();
    let request = panel.history_changed().unwrap();
    assert!(panel.apply(read(&store, request)).is_none());
    assert_eq!(panel.commit(), None);
}
