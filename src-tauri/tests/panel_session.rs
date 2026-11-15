//! 只驱动纯 ID 会话，不安装钩子，不访问数据库、桌面或剪贴板。

use clipboard_m0::panel_session::{Direction, Session, MAX_NEIGHBORHOOD_ITEMS};
use std::{collections::HashSet, sync::Arc};

fn ids(count: usize) -> Arc<[String]> {
    (0..count).map(|index| format!("id-{index}")).collect()
}

fn open(count: usize) -> Session {
    Session::open(1, ids(count)).unwrap()
}

#[test]
fn empty_history_has_no_session() {
    assert!(Session::open(1, ids(0)).is_none());
}

#[test]
fn initial_selection_and_small_histories() {
    for count in [1, 2, 201, 1001] {
        let mut session = open(count);
        let position = usize::from(count > 1);
        assert_eq!(session.position(), Some(position));
        assert_eq!(
            session.selected_id(),
            Some(format!("id-{position}").as_str())
        );
        let request = session.request(15).unwrap();
        assert_eq!(request.total, count);
        assert_eq!(request.ids.len(), count.min(15));
        assert_eq!(
            request.ids[request.selected],
            session.selected_id().unwrap()
        );
        assert_eq!(
            request.ids.iter().collect::<HashSet<_>>().len(),
            request.ids.len()
        );
    }
}

#[test]
fn both_directions_cover_entire_history_and_wrap() {
    for count in [1, 2, 201, 1001] {
        for direction in [Direction::Next, Direction::Previous] {
            let mut session = open(count);
            let initial = session.selected_id().unwrap().to_owned();
            let mut visited = HashSet::new();
            for _ in 0..count {
                visited.insert(session.selected_id().unwrap().to_owned());
                session.navigate(direction);
            }
            assert_eq!(visited.len(), count);
            assert_eq!(session.selected_id(), Some(initial.as_str()));
        }
    }
}

#[test]
fn continuous_navigation_commits_without_loading_views() {
    let mut session = open(301);
    for _ in 0..250 {
        session.navigate(Direction::Next);
    }
    for _ in 0..17 {
        session.navigate(Direction::Previous);
    }
    assert_eq!(session.position(), Some(234));
    assert_eq!(session.commit().as_deref(), Some("id-234"));
    assert_eq!(session.commit(), None);
}

#[test]
fn index_replacement_cannot_reorder_active_session() {
    let original = ids(301);
    let mut session = Session::open(1, original.clone()).unwrap();
    let mut replacement = original.iter().cloned().rev().collect::<Vec<_>>();
    replacement.insert(0, "new-id".to_owned());
    let next_index: Arc<[String]> = replacement.into();
    let existing = next_index.iter().cloned().collect();
    session.reconcile(&existing);
    assert_eq!(session.total(), 301);
    assert_eq!(session.selected_id(), Some("id-1"));
    session.navigate(Direction::Next);
    assert_eq!(session.selected_id(), Some("id-2"));
    let next_session = Session::open(2, next_index).unwrap();
    assert_eq!(next_session.selected_id(), Some("id-300"));
    assert_eq!(original[1], "id-1");
}

#[test]
fn removing_other_entries_preserves_id_and_survivor_order() {
    let mut session = open(301);
    for _ in 0..250 {
        session.navigate(Direction::Next);
    }
    let existing = ids(301)
        .iter()
        .filter(|id| !matches!(id.as_str(), "id-0" | "id-250" | "id-252"))
        .cloned()
        .collect();
    session.reconcile(&existing);
    assert_eq!(session.selected_id(), Some("id-251"));
    assert_eq!(session.position(), Some(249));
    assert_eq!(session.total(), 298);
    session.navigate(Direction::Next);
    assert_eq!(session.commit().as_deref(), Some("id-253"));
}

#[test]
fn deleting_or_expiring_deep_selection_cancels_instead_of_substituting() {
    let mut session = open(301);
    for _ in 0..250 {
        session.navigate(Direction::Next);
    }
    let request = session.request(5).unwrap();
    let existing = ids(301)
        .iter()
        .filter(|id| *id != "id-251")
        .cloned()
        .collect();
    session.reconcile(&existing);
    assert_eq!(session.selected_id(), None);
    assert!(!session.accepts(request.version));
    session.navigate(Direction::Next);
    assert_eq!(session.commit(), None);
    assert_eq!(session.request(5), None);
}

#[test]
fn clearing_history_cancels_session() {
    let mut session = open(301);
    session.reconcile(&HashSet::new());
    assert_eq!(session.commit(), None);
}

#[test]
fn neighborhood_is_bounded_centered_and_wraps_without_duplicates() {
    let mut session = open(301);
    session.navigate(Direction::Previous);
    let request = session.request(5).unwrap();
    assert_eq!(request.ids, ["id-299", "id-300", "id-0", "id-1", "id-2"]);
    assert_eq!(request.selected, 2);
    assert_eq!(request.position, 0);
    let largest = session.request(usize::MAX).unwrap();
    assert_eq!(largest.ids.len(), MAX_NEIGHBORHOOD_ITEMS);
}

#[test]
fn existing_even_visible_counts_remain_supported() {
    for count in 3..=15 {
        let mut session = open(301);
        let request = session.request(count).unwrap();
        assert_eq!(request.ids.len(), count);
        assert_eq!(request.ids[request.selected], "id-1");
    }
}

#[test]
fn late_responses_after_navigation_and_return_are_rejected() {
    let mut session = open(301);
    let old = session.request(5).unwrap();
    session.navigate(Direction::Next);
    assert!(!session.accepts(old.version));
    session.navigate(Direction::Previous);
    assert_eq!(session.position(), Some(old.position));
    assert!(!session.accepts(old.version));
    let latest = session.request(5).unwrap();
    assert!(session.accepts(latest.version));
    assert_eq!(session.commit().as_deref(), Some("id-1"));
    assert!(!session.accepts(latest.version));
}

#[test]
fn retries_at_same_position_reject_out_of_order_results() {
    let mut session = open(301);
    let old = session.request(5).unwrap();
    let latest = session.request(5).unwrap();
    assert!(!session.accepts(old.version));
    assert!(session.accepts(latest.version));
}

#[test]
fn security_changes_invalidate_pending_requests_without_changing_selection() {
    let mut session = open(301);
    let old = session.request(5).unwrap();
    session.invalidate();
    assert!(!session.accepts(old.version));
    assert_eq!(session.selected_id(), Some("id-1"));
    let refreshed = session.request(5).unwrap();
    assert!(session.accepts(refreshed.version));
    session.reconcile(&ids(301).iter().cloned().collect());
    assert!(!session.accepts(refreshed.version));
    assert_eq!(session.commit().as_deref(), Some("id-1"));
}

#[test]
fn cancelled_session_cannot_be_reactivated_by_refresh() {
    let mut session = open(301);
    let old = session.request(5).unwrap();
    session.cancel();
    session.reconcile(&ids(301).iter().cloned().collect());
    session.navigate(Direction::Next);
    assert!(!session.accepts(old.version));
    assert_eq!(session.request(5), None);
    assert_eq!(session.commit(), None);
}

#[test]
fn reopened_session_rejects_old_response_even_at_same_revision() {
    let mut old = open(301);
    let pending = old.request(5).unwrap();
    old.cancel();
    let mut reopened = Session::open(2, ids(301)).unwrap();
    let current = reopened.request(5).unwrap();
    assert!(!reopened.accepts(pending.version));
    assert!(reopened.accepts(current.version));
}

#[test]
fn wheel_slots_preserve_all_legacy_counts_without_duplicate_ids() {
    for visible in 3..=15 {
        for total in [1, 2, 3, 5, 201, 1001] {
            let mut session = open(total);
            for direction in [Direction::Previous, Direction::Next] {
                session.navigate(direction);
                let request = session.request(visible).unwrap();
                let count = visible.min(total);
                assert_eq!(request.ids.len(), count);
                assert_eq!(request.selected, count / 2);
                assert_eq!(
                    request.ids[request.selected],
                    session.selected_id().unwrap()
                );
                assert_eq!(request.ids.iter().collect::<HashSet<_>>().len(), count);
            }
        }
    }
}
