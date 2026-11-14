use super::{cleanup_due, refresh, State};
use crate::{diag, owner_flush};
use anyhow::{anyhow, Result};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tauri::AppHandle;

pub(super) struct CaptureMaintenance {
    last: Instant,
    inserted: u32,
}

impl Default for CaptureMaintenance {
    fn default() -> Self {
        Self {
            last: Instant::now(),
            inserted: 0,
        }
    }
}

impl CaptureMaintenance {
    pub(super) fn record_insertion(&mut self, state: &State) {
        self.inserted = self.inserted.saturating_add(1);
        if !cleanup_due(self.last.elapsed(), self.inserted) {
            return;
        }
        if let Err(error) = cleanup(state) {
            diag::write(format!("capture cleanup failed: {error}"));
        }
        // 失败也节流，避免故障时每次采集重复做昂贵维护。
        self.last = Instant::now();
        self.inserted = 0;
    }
}

pub(super) fn cleanup(state: &State) -> Result<()> {
    let policy = state
        .retention_policy
        .lock()
        .map_err(|_| anyhow!("保留策略不可用"))?;
    let mut context = state
        .capture_context
        .lock()
        .map_err(|_| anyhow!("采集上下文不可用"))?;
    let report = state
        .storage
        .lock()
        .map_err(|_| anyhow!("存储不可用"))?
        .cleanup(&policy, crate::storage::unix_millis())?;
    owner_flush::invalidate_deleted(&mut context, &report.deleted_ids);
    Ok(())
}

fn expire(state: &State) -> Result<(Duration, bool)> {
    let mut context = state
        .capture_context
        .lock()
        .map_err(|_| anyhow!("采集上下文不可用"))?;
    let mut store = state.storage.lock().map_err(|_| anyhow!("存储不可用"))?;
    let now = crate::storage::unix_millis();
    let delay = crate::sensitive::expiry_delay(now, store.next_expiry()?);
    if delay != Some(Duration::ZERO) {
        return Ok((
            delay
                .unwrap_or(Duration::from_secs(1))
                .min(Duration::from_secs(1)),
            false,
        ));
    }
    let report = store.delete_expired(now)?;
    owner_flush::invalidate_deleted(&mut context, &report.deleted_ids);
    Ok((Duration::from_millis(100), report.deleted_items > 0))
}

fn expiry_tick(app: &AppHandle, state: &State) -> Result<Duration> {
    let (delay, deleted) = expire(state)?;
    if deleted {
        refresh(app, state)?;
    }
    Ok(delay)
}

pub(super) fn start(app: AppHandle, state: &Arc<State>) -> Result<()> {
    let shared = Arc::downgrade(state);
    thread::Builder::new()
        .name("clipboard-expiry".into())
        .spawn(move || loop {
            let Some(state) = shared.upgrade() else {
                break;
            };
            let result = expiry_tick(&app, &state);
            drop(state);
            let delay = result.unwrap_or_else(|error| {
                diag::write(format!("sensitive expiry maintenance failed: {error}"));
                Duration::from_secs(1)
            });
            thread::sleep(delay);
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_support::{captured, poison, Fixture};

    #[derive(Clone, Copy)]
    enum PoisonTarget {
        Policy,
        Context,
        Storage,
    }

    #[test]
    fn poisoned_maintenance_locks_return_errors_and_release_healthy_locks() {
        for target in [
            PoisonTarget::Policy,
            PoisonTarget::Context,
            PoisonTarget::Storage,
        ] {
            let fixture = Fixture::new("maintenance-poison");
            let state = fixture.state();
            match target {
                PoisonTarget::Policy => poison(&state.retention_policy),
                PoisonTarget::Context => poison(&state.capture_context),
                PoisonTarget::Storage => poison(&state.storage),
            }
            assert!(cleanup(&state).is_err());
            match target {
                PoisonTarget::Policy => assert!(expire(&state).is_ok()),
                PoisonTarget::Context | PoisonTarget::Storage => assert!(expire(&state).is_err()),
            }
            if !state.retention_policy.is_poisoned() {
                assert!(state.retention_policy.try_lock().is_ok());
            }
            if !state.capture_context.is_poisoned() {
                assert!(state.capture_context.try_lock().is_ok());
            }
            if !state.storage.is_poisoned() {
                assert!(state.storage.try_lock().is_ok());
            }
        }
    }

    #[test]
    fn failed_capture_cleanup_is_throttled_and_next_capture_can_continue() {
        let fixture = Fixture::new("cleanup-throttle");
        let state = fixture.state();
        poison(&state.retention_policy);
        let mut maintenance = CaptureMaintenance {
            last: Instant::now(),
            inserted: 19,
        };
        maintenance.record_insertion(&state);
        assert_eq!(maintenance.inserted, 0);
        let last = maintenance.last;
        maintenance.record_insertion(&state);
        assert_eq!(maintenance.inserted, 1);
        assert_eq!(maintenance.last, last);
        assert!(state.storage.try_lock().is_ok());
        assert!(state.capture_context.try_lock().is_ok());
    }

    #[test]
    fn cleanup_invalidates_deleted_capture_context_and_empty_expiry_releases_locks() {
        let fixture = Fixture::new("cleanup-context");
        let state = fixture.state();
        crate::runtime::save_capture(&state.storage, &state.capture_context, captured()).unwrap();
        assert!(state.capture_context.lock().unwrap().is_some());
        let mut newer = captured().clip;
        newer.formats[0].bytes = "newer synthetic"
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        state.storage.lock().unwrap().save(&newer).unwrap();
        state.retention_policy.lock().unwrap().max_items = 1;
        cleanup(&state).unwrap();
        assert!(state.capture_context.lock().unwrap().is_none());
        assert_eq!(state.storage.lock().unwrap().list().unwrap().len(), 1);
        assert_eq!(expire(&state).unwrap(), (Duration::from_secs(1), false));
        assert!(state.capture_context.try_lock().is_ok());
        assert!(state.storage.try_lock().is_ok());
    }
}
