//! 版本锁只保护内存发布，不跨越 SQL 或系统调用。
use super::{Control, Panel, State};
use anyhow::{anyhow, Result};
use std::sync::Mutex;

#[derive(Clone, Copy, Default)]
pub(super) struct Stamp {
    pub revision: u64,
    pub active: usize,
}
#[derive(Default)]
pub(super) struct Gate(pub Mutex<Stamp>);
impl Gate {
    pub(super) fn stamp(&self) -> Result<Stamp> {
        Ok(*self.0.lock().map_err(|_| anyhow!("历史版本不可用"))?)
    }
}
pub(crate) struct Change<'a>(&'a State);
pub(crate) fn begin(state: &State) -> Result<Change<'_>> {
    let mut stamp = state
        .history_gate
        .0
        .lock()
        .map_err(|_| anyhow!("历史版本不可用"))?;
    let mut panel = state.panel.lock().map_err(|_| anyhow!("面板快照不可用"))?;
    stamp.revision += 1;
    stamp.active += 1;
    sanitize(&mut panel, *stamp);
    panel.serial = state
        .panel_serial
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
        + 1;
    drop(panel);
    drop(stamp);
    notify(state);
    Ok(Change(state))
}
pub(super) fn sanitize(panel: &mut Panel, stamp: Stamp) {
    if stamp.active == 0 && panel.history_revision == stamp.revision {
        return;
    }
    panel.items.clear();
    panel.selected = 0;
    panel.loading = panel.visible;
    if panel.visible {
        panel.error = None;
    }
}
fn notify(state: &State) {
    crate::app_events::record(
        "history version notification failed",
        crate::app_events::send_control(&state.control, Control::HistoryChanged),
    );
}
impl Drop for Change<'_> {
    fn drop(&mut self) {
        match self.0.history_gate.0.lock() {
            Ok(mut stamp) => {
                stamp.active -= 1;
                stamp.revision += 1;
            }
            Err(error) => crate::diag::write(format!("history version completion failed: {error}")),
        }
        notify(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::ItemView,
        runtime::{panel_snapshot, store_panel_snapshot, test_support::Fixture},
    };

    fn visible_panel() -> Panel {
        Panel {
            visible: true,
            items: vec![ItemView {
                id: "synthetic".into(),
                summary: "旧正文".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn mutation_blocks_stale_snapshot_before_notification_and_after_completion() {
        let fixture = Fixture::new("history-gate");
        let state = fixture.state();
        let old = visible_panel();
        store_panel_snapshot(&state, &old).unwrap();
        let change = begin(&state).unwrap();
        let serial = panel_snapshot(&state).unwrap().serial;
        assert!(serial > old.serial);
        assert!(panel_snapshot(&state).unwrap().items.is_empty());
        store_panel_snapshot(&state, &old).unwrap();
        assert!(panel_snapshot(&state).unwrap().items.is_empty());
        drop(change);
        store_panel_snapshot(&state, &old).unwrap();
        assert!(panel_snapshot(&state).unwrap().items.is_empty());
        let mut current = old;
        current.history_revision = state.history_gate.stamp().unwrap().revision;
        store_panel_snapshot(&state, &current).unwrap();
        assert_eq!(panel_snapshot(&state).unwrap().items.len(), 1);
    }

    #[test]
    fn overlapping_changes_cannot_reenable_views_until_last_completion() {
        let fixture = Fixture::new("history-overlap");
        let state = fixture.state();
        let first = begin(&state).unwrap();
        let second = begin(&state).unwrap();
        drop(first);
        let stamp = state.history_gate.stamp().unwrap();
        assert_eq!(stamp.active, 1);
        let mut panel = visible_panel();
        panel.history_revision = stamp.revision;
        store_panel_snapshot(&state, &panel).unwrap();
        assert!(panel_snapshot(&state).unwrap().items.is_empty());
        drop(second);
        panel.history_revision = state.history_gate.stamp().unwrap().revision;
        store_panel_snapshot(&state, &panel).unwrap();
        assert_eq!(panel_snapshot(&state).unwrap().items.len(), 1);
    }

    #[test]
    fn safety_sanitization_preserves_hidden_operation_error() {
        let mut panel = Panel {
            error: Some("记录已删除".into()),
            ..Default::default()
        };
        sanitize(
            &mut panel,
            Stamp {
                revision: 7,
                active: 1,
            },
        );
        assert_eq!(panel.error.as_deref(), Some("记录已删除"));
        assert!(!panel.loading);
    }
}
