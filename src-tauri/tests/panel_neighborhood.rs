//! 纯协调器测试；不读取桌面、不安装钩子、不操作剪贴板。
use clipraft_windows::{
    model::ItemView,
    panel_neighborhood::{Coordinator, Request, Response, ViewState},
    panel_session::Direction,
};
use std::sync::Arc;

fn ids() -> Arc<[String]> {
    (0..301).map(|index| format!("id-{index}")).collect()
}

fn open(session: u64) -> (Coordinator, Request) {
    Coordinator::open(session, ids(), 5).unwrap()
}

fn views(request: Request) -> Response {
    let Request::Views(request) = request else {
        panic!("expected view request");
    };
    let items = request
        .ids
        .iter()
        .map(|id| ItemView {
            id: id.clone(),
            summary: format!("summary {id}"),
            ..Default::default()
        })
        .collect();
    Response::Views {
        request,
        result: Ok(items),
    }
}

fn validate(request: Request, removed: &[&str]) -> Response {
    let Request::Validate { version, ids } = request else {
        panic!("expected validation request");
    };
    let existing = ids
        .iter()
        .filter(|id| !removed.contains(&id.as_str()))
        .cloned()
        .collect();
    Response::Validated {
        version,
        result: Ok(existing),
    }
}

#[test]
fn open_loads_bounded_neighborhood_at_global_position() {
    let (mut panel, request) = open(1);
    assert!(matches!(panel.view(), ViewState::Loading));
    assert_eq!(panel.position(), Some(1));
    assert_eq!(panel.total(), 301);
    assert!(panel.apply(views(request)).is_none());
    let ViewState::Ready { items, selected } = panel.view() else {
        panic!("not ready")
    };
    assert_eq!(items.len(), 5);
    assert_eq!(items[*selected].id, "id-1");
}

#[test]
fn navigation_clears_previous_content_and_ignores_late_success_or_error() {
    let (mut panel, first) = open(1);
    panel.apply(views(first.clone()));
    let latest = panel.navigate(Direction::Next).unwrap();
    assert!(matches!(panel.view(), ViewState::Loading));
    panel.apply(views(first.clone()));
    panel.apply(first.failed("late error".into()));
    assert!(matches!(panel.view(), ViewState::Loading));
    panel.apply(views(latest));
    let ViewState::Ready { items, selected } = panel.view() else {
        panic!("not ready")
    };
    assert_eq!(items[*selected].id, "id-2");
}

#[test]
fn immediate_commit_uses_latest_id_while_view_is_loading() {
    let (mut panel, old) = open(1);
    for _ in 0..250 {
        panel.navigate(Direction::Next);
    }
    assert_eq!(panel.commit().as_deref(), Some("id-251"));
    panel.apply(views(old));
    assert!(matches!(panel.view(), ViewState::Empty));
    assert_eq!(panel.commit(), None);
}

#[test]
fn load_failure_never_reuses_content_and_retry_has_a_new_version() {
    let (mut panel, old) = open(1);
    panel.apply(views(old.clone()));
    let retry = panel.retry().unwrap();
    panel.apply(retry.failed("synthetic failure".into()));
    assert!(matches!(panel.view(), ViewState::Failed(message) if message == "synthetic failure"));
    panel.apply(views(old));
    assert!(matches!(panel.view(), ViewState::Failed(_)));
    let retry = panel.retry().unwrap();
    panel.apply(views(retry));
    assert!(matches!(panel.view(), ViewState::Ready { .. }));
}

#[test]
fn sensitive_change_clears_old_content_before_validation_and_reloads_views() {
    let (mut panel, old) = open(1);
    panel.apply(views(old.clone()));
    let validation = panel.history_changed().unwrap();
    assert!(matches!(panel.view(), ViewState::Loading));
    panel.apply(views(old));
    assert!(matches!(panel.view(), ViewState::Loading));
    let request = panel.apply(validate(validation, &[])).unwrap();
    let Response::Views {
        request,
        mut result,
    } = views(request)
    else {
        unreachable!()
    };
    for item in result.as_mut().unwrap() {
        item.summary = "••••••••".into();
        item.sensitive = true;
    }
    panel.apply(Response::Views { request, result });
    let ViewState::Ready { items, .. } = panel.view() else {
        panic!("not ready")
    };
    assert!(items.iter().all(|item| item.summary == "••••••••"));
}

#[test]
fn navigation_during_validation_keeps_latest_selection_and_reuses_full_check() {
    let (mut panel, _) = open(1);
    let validation = panel.history_changed().unwrap();
    for _ in 0..250 {
        assert!(panel.navigate(Direction::Next).is_none());
    }
    let request = panel
        .apply(validate(validation, &["id-0", "id-250"]))
        .unwrap();
    assert_eq!(panel.selected_id(), Some("id-251"));
    assert_eq!(panel.position(), Some(249));
    panel.apply(views(request));
    assert!(matches!(panel.view(), ViewState::Ready { .. }));
}

#[test]
fn deleting_deep_selection_or_clearing_history_cancels_without_substitution() {
    let removed_sets = [
        vec!["id-251".to_owned()],
        (0..301).map(|index| format!("id-{index}")).collect(),
    ];
    for removed in removed_sets {
        let (mut panel, _) = open(1);
        for _ in 0..250 {
            panel.navigate(Direction::Next);
        }
        let request = panel.history_changed().unwrap();
        let removed: Vec<_> = removed.iter().map(String::as_str).collect();
        panel.apply(validate(request, &removed));
        assert_eq!(panel.selected_id(), None);
        assert_eq!(panel.commit(), None);
        assert!(matches!(panel.view(), ViewState::Empty));
    }
}

#[test]
fn missing_neighbor_triggers_full_check_and_missing_selected_item_cancels() {
    let (mut panel, request) = open(1);
    let Response::Views { request, result } = views(request) else {
        unreachable!()
    };
    let mut items = result.unwrap();
    items.retain(|item| item.id != "id-0");
    let validation = panel
        .apply(Response::Views {
            request,
            result: Ok(items),
        })
        .unwrap();
    let request = panel.apply(validate(validation, &["id-0"])).unwrap();
    assert_eq!(panel.total(), 300);
    assert_eq!(panel.selected_id(), Some("id-1"));
    let Response::Views { request, result } = views(request) else {
        unreachable!()
    };
    let mut items = result.unwrap();
    items.retain(|item| item.id != "id-1");
    panel.apply(Response::Views {
        request,
        result: Ok(items),
    });
    assert_eq!(panel.commit(), None);
}

#[test]
fn repeated_history_changes_reject_old_validation_success_and_failure() {
    let (mut panel, _) = open(1);
    let old = panel.history_changed().unwrap();
    let current = panel.history_changed().unwrap();
    panel.apply(validate(old.clone(), &["id-1"]));
    panel.apply(old.failed("late validation failure".into()));
    assert_eq!(panel.selected_id(), Some("id-1"));
    assert!(matches!(panel.view(), ViewState::Loading));
    let request = panel.apply(validate(current, &[])).unwrap();
    panel.apply(views(request));
    assert!(matches!(panel.view(), ViewState::Ready { .. }));
}

#[test]
fn failed_validation_preserves_selection_without_treating_failure_as_deletion() {
    let (mut panel, _) = open(1);
    let request = panel.history_changed().unwrap();
    panel.apply(request.failed("storage unavailable".into()));
    assert_eq!(panel.total(), 301);
    assert_eq!(panel.selected_id(), Some("id-1"));
    assert!(matches!(panel.view(), ViewState::Failed(_)));
    let request = panel.retry().unwrap();
    let request = panel.apply(validate(request, &[])).unwrap();
    panel.apply(views(request));
    assert!(matches!(panel.view(), ViewState::Ready { .. }));
}

#[test]
fn closed_and_reopened_sessions_reject_old_view_and_validation() {
    let (mut old, old_view) = open(1);
    let old_validation = old.history_changed().unwrap();
    old.cancel();
    old.apply(views(old_view.clone()));
    old.apply(validate(old_validation.clone(), &[]));
    assert!(matches!(old.view(), ViewState::Empty));
    let (mut new, current) = open(2);
    new.apply(views(old_view));
    let validation = new.history_changed().unwrap();
    new.apply(validate(old_validation, &["id-1"]));
    assert!(matches!(new.view(), ViewState::Loading));
    let request = new.apply(validate(validation, &[])).unwrap();
    new.apply(views(current));
    assert!(matches!(new.view(), ViewState::Loading));
    new.apply(views(request));
    assert!(matches!(new.view(), ViewState::Ready { .. }));
}
