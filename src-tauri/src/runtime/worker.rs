use super::{
    capture_paused,
    maintenance::CaptureMaintenance,
    paste_job::{self, PasteRequest},
    platform, queue_capture, refresh, report, save_capture, Job, State, CAPTURE_GATE,
    CAPTURE_PENDING, LISTENER,
};
use crate::{capture_policy::CaptureError, clipboard, diag};
use anyhow::Result;
use std::{
    sync::{atomic::Ordering, mpsc::Receiver, Arc},
    thread,
};
use tauri::AppHandle;

fn ignored_apps(state: &State) -> Result<Vec<String>, CaptureError> {
    state
        .settings
        .lock()
        .map(|settings| settings.ignored_apps.clone())
        .map_err(|_| CaptureError::SettingsUnavailable)
}

fn store_capture(
    app: &AppHandle,
    state: &State,
    maintenance: &mut CaptureMaintenance,
    captured: clipboard::Captured,
) {
    let _change = match super::history_gate::begin(state) {
        Ok(change) => change,
        Err(error) => {
            report(app, state, &error.to_string());
            return;
        }
    };
    let saved = match save_capture(&state.storage, &state.capture_context, captured) {
        Ok(saved) => saved,
        Err(error) => {
            diag::write(format!("capture save failed: {error}"));
            report(app, state, "记录保存失败");
            return;
        }
    };
    let Some(inserted) = saved else {
        return;
    };
    if inserted {
        maintenance.record_insertion(state);
    }
    // 保存与清理共用一次刷新；去重更新仍需要重排列表。
    if let Err(error) = refresh(app, state) {
        diag::write(format!("capture refresh failed: {error}"));
        report(app, state, "历史列表刷新失败");
    }
}

fn capture_job(app: &AppHandle, state: &State, maintenance: &mut CaptureMaintenance) {
    let gate = match CAPTURE_GATE.lock() {
        Ok(gate) => gate,
        Err(_) => {
            CAPTURE_PENDING.store(false, Ordering::Release);
            report(app, state, "采集门不可用，本次复制未记录");
            return;
        }
    };
    if capture_paused() {
        CAPTURE_PENDING.store(false, Ordering::Release);
        return;
    }
    match clipboard::capture_pending_with_settings(
        platform::hwnd(LISTENER.load(Ordering::Acquire)),
        || ignored_apps(state),
    ) {
        Ok(Some(captured)) => {
            store_capture(app, state, maintenance, captured);
        }
        Ok(None) => {}
        Err(error) => report(app, state, &error.to_string()),
    }
    drop(gate);
    CAPTURE_PENDING.store(false, Ordering::Release);
    // 等待期间的变化可能被合并，补排尚未处理的新序列。
    queue_capture();
}

pub(super) fn start(app: AppHandle, state: Arc<State>, receiver: Receiver<Job>) -> Result<()> {
    thread::Builder::new()
        .name("clipboard-storage-worker".into())
        .spawn(move || {
            if let Err(error) = super::history_index::initialize(&state) {
                diag::write(format!("history index initialization failed: {error}"));
                report(&app, &state, "历史索引准备失败，请稍后重试");
            }
            let mut maintenance = CaptureMaintenance::default();
            while let Ok(job) = receiver.recv() {
                match job {
                    Job::Capture => capture_job(&app, &state, &mut maintenance),
                    Job::Paste {
                        id,
                        target,
                        restore,
                        plain,
                        generation,
                    } => paste_job::run(
                        &app,
                        &state,
                        PasteRequest {
                            id,
                            target,
                            restore,
                            plain,
                            generation,
                        },
                    ),
                }
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_support::{captured, poison, Fixture};

    #[test]
    fn ignored_settings_are_reloaded_and_poison_never_falls_back_to_empty_list() {
        let fixture = Fixture::new("capture-settings");
        let state = fixture.state();
        state.settings.lock().unwrap().ignored_apps = vec!["synthetic.exe".into()];
        assert_eq!(ignored_apps(&state).unwrap(), ["synthetic.exe"]);
        state.settings.lock().unwrap().ignored_apps = vec!["updated.exe".into()];
        assert_eq!(ignored_apps(&state).unwrap(), ["updated.exe"]);
        poison(&state.settings);
        assert_eq!(ignored_apps(&state), Err(CaptureError::SettingsUnavailable));
    }

    #[test]
    fn poisoned_capture_context_or_storage_cannot_commit_new_context() {
        for target in ["context", "storage"] {
            let fixture = Fixture::new("capture-poison");
            let state = fixture.state();
            match target {
                "context" => poison(&state.capture_context),
                "storage" => poison(&state.storage),
                _ => unreachable!(),
            }
            assert!(save_capture(&state.storage, &state.capture_context, captured()).is_err());
            if !state.storage.is_poisoned() {
                assert!(state.storage.lock().unwrap().list().unwrap().is_empty());
            }
            if !state.capture_context.is_poisoned() {
                assert!(state.capture_context.try_lock().unwrap().is_none());
            }
        }
    }
}
