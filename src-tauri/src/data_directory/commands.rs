use super::{
    config::{self, Configuration, Migration},
    dialog, files, paths, DirectoryControl,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::WebviewWindow;

#[derive(Clone, Serialize)]
pub(crate) struct Plan {
    pub(super) ticket: String,
    source: PathBuf,
    target: PathBuf,
    bytes: u64,
    displaced: Option<PathBuf>,
}
#[derive(Serialize)]
pub(crate) struct Status {
    current: PathBuf,
    default: PathBuf,
    custom: bool,
    enabled: bool,
    previous: Option<PathBuf>,
    displaced: Option<PathBuf>,
}
fn main_only(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("仅主窗口可更改数据目录".into());
    }
    Ok(())
}
impl DirectoryControl {
    fn active_configuration(&self) -> Result<Configuration> {
        let config = self.locations.read().with_context(|| {
            format!(
                "启动定位不可用，迁移未执行。当前数据仍在 {}；请勿清空目录",
                self.root.display()
            )
        })?;
        if fs::canonicalize(&config.root)? != self.root {
            bail!("启动定位与当前数据目录不一致，迁移未执行。当前数据仍在 {}；请通过恢复入口选择现有数据，不要清空目录", self.root.display());
        }
        Ok(config)
    }
    pub(super) fn preview(&self, target: &Path) -> Result<Plan> {
        if self.locations.config.is_none() {
            bail!("当前隔离环境禁止更改数据目录");
        }
        if self.active_configuration()?.pending.is_some() {
            bail!("已有待执行的迁移，请先重启完成或恢复");
        }
        let target = paths::target_path(&self.root, target)?;
        let _guard = files::lock(&target)?;
        let displaced = super::restore::preview(&target, &self.locations)?;
        paths::writable(&target)?;
        let bytes = files::measure(&self.root)?;
        paths::capacity(&target, bytes)?;
        let plan = Plan {
            ticket: config::nonce()?,
            source: self.root.clone(),
            target,
            bytes,
            displaced,
        };
        *self
            .plan
            .lock()
            .map_err(|_| anyhow::anyhow!("迁移预检状态不可用"))? = Some(plan.clone());
        Ok(plan)
    }
    pub(super) fn schedule(&self, ticket: &str) -> Result<()> {
        let mut saved = self
            .plan
            .lock()
            .map_err(|_| anyhow::anyhow!("迁移预检状态不可用"))?;
        let plan = saved
            .as_ref()
            .filter(|plan| plan.ticket == ticket)
            .context("迁移预检已失效，请重新选择目录")?;
        let target = paths::target_path(&self.root, &plan.target)?;
        let _guard = files::lock(&target)?;
        super::restore::confirm(&target, plan.displaced.as_deref(), &self.locations)?;
        paths::writable(&target)?;
        paths::capacity(&target, files::measure(&self.root)?)?;
        let mut config = self.active_configuration()?;
        if config.pending.is_some() {
            bail!(
                "已有待执行的迁移，请勿重复提交；当前数据仍在 {}",
                self.root.display()
            );
        }
        config.root = self.root.clone();
        config.pending = Some(Migration {
            source: self.root.clone(),
            target,
            id: config::nonce()?,
            displaced: plan.displaced.clone(),
        });
        self.locations.save(&config)?;
        *saved = None;
        Ok(())
    }
}
#[tauri::command]
pub(crate) fn data_directory_status(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<DirectoryControl>>,
) -> Result<Status, String> {
    main_only(&window)?;
    let config = state
        .locations
        .read()
        .map_err(|error| format!("{error:#}"))?;
    let default =
        fs::canonicalize(&state.locations.default_root).map_err(|error| error.to_string())?;
    Ok(Status {
        current: state.root.clone(),
        custom: state.root != default,
        default,
        enabled: state.locations.config.is_some(),
        previous: config.previous,
        displaced: config.displaced,
    })
}
#[tauri::command]
pub(crate) async fn choose_data_directory(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<DirectoryControl>>,
) -> Result<Option<Plan>, String> {
    main_only(&window)?;
    if state.locations.config.is_none() {
        return Err("当前隔离环境禁止更改数据目录".into());
    }
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as isize;
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let selection = std::thread::spawn(move || dialog::select_destination(Some(owner)))
            .join()
            .map_err(|_| anyhow::anyhow!("目录选择任务异常"))??;
        selection.map(|target| shared.preview(&target)).transpose()
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| format!("{error:#}"))
}
#[tauri::command]
pub(crate) async fn restore_data_directory(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<DirectoryControl>>,
) -> Result<Plan, String> {
    main_only(&window)?;
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || shared.preview(&shared.locations.default_root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))
}
#[tauri::command]
pub(crate) async fn migrate_data_directory(
    ticket: String,
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<DirectoryControl>>,
) -> Result<(), String> {
    main_only(&window)?;
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || shared.schedule(&ticket))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))?;
    app.request_restart();
    Ok(())
}
