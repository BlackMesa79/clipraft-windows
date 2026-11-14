use super::{change_main_hotkey, check_hotkeys, refresh, State};
use crate::{
    diag, hook,
    owner_flush::CaptureContext,
    settings::Settings,
    storage::{RetentionPolicy, Storage},
};
use anyhow::{anyhow, Result};
use std::sync::{atomic::Ordering, MutexGuard};
use tauri::{AppHandle, Emitter};

struct SystemChange<'a> {
    previous: &'a Settings,
    next: &'a Settings,
    main: bool,
    autostart: bool,
    probe: bool,
}

struct StoredUpdate<'a> {
    // 策略锁须覆盖存储清理及上下文更新；字段按此顺序释放 guard。
    storage: MutexGuard<'a, Storage>,
    context: MutexGuard<'a, Option<CaptureContext>>,
    policy: MutexGuard<'a, RetentionPolicy>,
}

pub(super) fn apply_settings(
    app: &AppHandle,
    state: &State,
    next: Settings,
    panel_visible: bool,
) -> Result<Settings> {
    if hook::recording() {
        return Err(anyhow!("请先结束快捷键录制"));
    }
    let next = next.normalized()?;
    let _change = super::history_gate::begin(state)?;
    let mut current = state
        .settings
        .lock()
        .map_err(|_| anyhow!("设置状态不可用"))?;
    let mut startup = state
        .startup_error
        .lock()
        .map_err(|_| anyhow!("启动错误状态不可用"))?;
    let change = SystemChange::new(state, &current, &next, &startup);
    let reservation = change.reserve(panel_visible)?;
    change.apply(app, &mut startup)?;
    let mut stored = match StoredUpdate::save(state, &next) {
        Ok(stored) => stored,
        Err(error) => return Err(change.rollback(app, &mut startup, error)),
    };
    let main_changed = change.main;
    stored.commit(state, &mut current, &next);
    drop(current);
    drop(reservation);
    if main_changed {
        set_startup_error(app, &mut startup, None);
    }
    drop(startup);
    stored.cleanup(app);
    drop(stored);
    notify_saved(app, state, &next);
    Ok(next)
}

impl<'a> SystemChange<'a> {
    fn new(
        state: &State,
        previous: &'a Settings,
        next: &'a Settings,
        startup: &Option<String>,
    ) -> Self {
        let main = previous.main_hotkey != next.main_hotkey || startup.is_some();
        Self {
            previous,
            next,
            main,
            autostart: !state.smoke && previous.autostart != next.autostart,
            probe: main || previous.quick_hotkey != next.quick_hotkey,
        }
    }

    fn reserve(&self, panel_visible: bool) -> Result<Option<hook::ConfigGuard<'static>>> {
        if !self.probe {
            return Ok(None);
        }
        if panel_visible {
            return Err(anyhow!("快速面板正在激活，请关闭面板后重试"));
        }
        let reservation = hook::reserve_configuration_when_hidden()?;
        check_hotkeys(self.next.main_hotkey, self.next.quick_hotkey)?.validate()?;
        Ok(Some(reservation))
    }

    fn apply(&self, app: &AppHandle, startup: &mut Option<String>) -> Result<()> {
        if self.main {
            change_main_hotkey(self.next.main_hotkey).inspect_err(|error| {
                if error.to_string().contains("旧快捷键恢复失败") {
                    set_startup_error(app, startup, Some(error.to_string()));
                }
            })?;
        }
        if !self.autostart {
            return Ok(());
        }
        if let Err(error) = crate::autostart::set(self.next.autostart) {
            if let Err(rollback) = self.restore_main() {
                let message = format!("自启设置失败：{error}；热键恢复失败：{rollback}");
                set_startup_error(app, startup, Some(message.clone()));
                return Err(anyhow!(message));
            }
            return Err(error);
        }
        Ok(())
    }

    fn restore_main(&self) -> Result<()> {
        if !self.main {
            return Ok(());
        }
        change_main_hotkey(self.previous.main_hotkey)
    }

    fn rollback(
        &self,
        app: &AppHandle,
        startup: &mut Option<String>,
        error: anyhow::Error,
    ) -> anyhow::Error {
        let autostart_error = self
            .autostart
            .then(|| crate::autostart::set(self.previous.autostart))
            .transpose()
            .err();
        let main_error = self.restore_main().err();
        let mut message = format!("设置未保存：{error}");
        if let Some(rollback) = &main_error {
            message.push_str(&format!("；热键恢复失败：{rollback}"));
        }
        if let Some(rollback) = autostart_error {
            message.push_str(&format!("；自启恢复失败：{rollback}"));
        }
        if main_error.is_some() {
            set_startup_error(app, startup, Some(message.clone()));
        }
        anyhow!(message)
    }
}

impl StoredUpdate<'_> {
    fn save<'a>(state: &'a State, next: &Settings) -> Result<StoredUpdate<'a>> {
        // 与 worker 清理保持同序；任何获取/保存错误均交给外层回滚系统设置。
        let policy = state
            .retention_policy
            .lock()
            .map_err(|_| anyhow!("保留策略不可用"))?;
        let context = state
            .capture_context
            .lock()
            .map_err(|_| anyhow!("采集上下文不可用"))?;
        let mut storage = state.storage.lock().map_err(|_| anyhow!("存储不可用"))?;
        storage.save_settings(next)?;
        Ok(StoredUpdate {
            policy,
            context,
            storage,
        })
    }

    fn commit(&mut self, state: &State, current: &mut Settings, next: &Settings) {
        *self.policy = next.retention_policy();
        hook::configure_settings(next);
        state
            .hide_on_blur
            .store(next.hide_on_blur, Ordering::Release);
        *current = next.clone();
    }

    fn cleanup(&mut self, app: &AppHandle) {
        match self
            .storage
            .cleanup(&self.policy, crate::storage::unix_millis())
        {
            Ok(report) => {
                crate::owner_flush::invalidate_deleted(&mut self.context, &report.deleted_ids);
                if !report.failed_blobs.is_empty() || !report.maintenance_errors.is_empty() {
                    operation_error(
                        app,
                        "设置已保存，记录已清理，但部分文件回收或数据库整理失败，将在后续维护重试"
                            .into(),
                    );
                }
            }
            Err(error) => operation_error(app, format!("设置已保存，但立即清理失败：{error}")),
        }
    }
}

fn set_startup_error(app: &AppHandle, startup: &mut Option<String>, error: Option<String>) {
    *startup = error;
    if let Some(message) = startup.as_deref() {
        diag::write(message.into());
    }
    let _ = app.emit("startup-error", startup.as_deref().unwrap_or(""));
}

fn operation_error(app: &AppHandle, message: String) {
    diag::write(message.clone());
    let _ = app.emit("operation-error", message);
}

fn notify_saved(app: &AppHandle, state: &State, next: &Settings) {
    let _ = app.emit("settings-changed", next);
    if let Err(error) = refresh(app, state) {
        diag::write(format!("settings refresh failed: {error}"));
    }
}
