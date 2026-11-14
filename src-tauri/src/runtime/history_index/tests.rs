use super::*;
use crate::runtime::test_support::{poison, Fixture};

#[test]
fn pending_empty_ready_and_failure_are_explicit() {
    let cache = Cache::default();
    assert!(cache
        .snapshot()
        .err()
        .unwrap()
        .to_string()
        .contains("正在准备"));
    cache.replace(Ok(Vec::new())).unwrap();
    let empty = cache.snapshot().unwrap();
    assert_eq!(empty.version, 1);
    assert!(empty.ids.is_empty());
    assert!(cache.replace(Err(anyhow!("synthetic failure"))).is_err());
    assert!(cache
        .snapshot()
        .err()
        .unwrap()
        .to_string()
        .contains("synthetic failure"));
}

#[test]
fn failed_refresh_keeps_complete_version_and_active_snapshot_then_recovers() {
    let cache = Cache::default();
    cache
        .replace(Ok(vec!["first".into(), "second".into()]))
        .unwrap();
    let active = cache.snapshot().unwrap();
    assert!(cache.replace(Err(anyhow!("query failed"))).is_err());
    assert!(cache.snapshot().is_err());
    assert_eq!(
        cache.0.lock().unwrap().complete.as_ref().unwrap().version,
        1
    );
    assert_eq!(&*active.ids, ["first", "second"]);
    cache.replace(Ok(vec!["new".into()])).unwrap();
    let latest = cache.snapshot().unwrap();
    assert_eq!(latest.version, 2);
    assert_eq!(&*latest.ids, ["new"]);
    assert_eq!(&*active.ids, ["first", "second"]);
}

#[test]
fn initialize_queries_storage_but_snapshot_read_does_not_take_storage_lock() {
    let fixture = Fixture::new("u3-index");
    let state = fixture.state();
    assert!(state.history_index.snapshot().is_err());
    initialize(&state).unwrap();
    let storage = state.storage.lock().unwrap();
    assert!(state.history_index.snapshot().unwrap().ids.is_empty());
    let captured = crate::runtime::test_support::captured();
    drop(storage);
    let id = captured.clip.hash();
    state.storage.lock().unwrap().save(&captured.clip).unwrap();
    initialize(&state).unwrap();
    let storage = state.storage.lock().unwrap();
    assert_eq!(&*state.history_index.snapshot().unwrap().ids, [id]);
    drop(storage);
}

#[test]
fn poisoned_storage_marks_initialization_failed_and_does_not_publish_empty_success() {
    let fixture = Fixture::new("u3-index-poison");
    let state = fixture.state();
    poison(&state.storage);
    assert!(initialize(&state).is_err());
    assert!(state
        .history_index
        .snapshot()
        .err()
        .unwrap()
        .to_string()
        .contains("存储不可用"));
}

#[test]
fn poisoned_cache_does_not_panic_or_return_old_success() {
    let cache = Cache::default();
    cache.replace(Ok(vec!["first".into()])).unwrap();
    poison(&cache.0);
    assert!(cache.snapshot().is_err());
    assert!(cache.replace(Ok(vec!["second".into()])).is_err());
}
