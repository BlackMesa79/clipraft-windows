//! 后台导出/恢复的运行时入口，参数中的密码不进入任务报告。
use crate::{
    archive::{
        control::Control,
        jobs::{Kind, Manager, Success},
        restore_job,
    },
    runtime::archive_task,
};
use anyhow::{Context, Result};
use std::{path::PathBuf, sync::Arc};
use tauri::Manager as _;
use zeroize::Zeroizing;

pub struct Service {
    pub(crate) jobs: Arc<Manager>,
    root: PathBuf,
    cleanup_started: std::sync::atomic::AtomicBool,
}
impl Service {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            jobs: Arc::new(Manager::default()),
            root,
            cleanup_started: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub(crate) fn claim_cleanup_start(&self) -> bool {
        !self
            .cleanup_started
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    }
    pub fn start_export(
        &self,
        app: tauri::AppHandle,
        destination: PathBuf,
        password: Zeroizing<String>,
    ) -> Result<u64> {
        crate::archive::validate_password(password.as_bytes())?;
        let root = self.root.clone();
        self.jobs.start(Kind::Export, move |control| {
            control.check()?;
            if !root.join("history.db").is_file() {
                anyhow::bail!("当前数据库不存在，停止导出");
            }
            let master = crate::platform::master_key(&root)?;
            let parent = destination.parent().context("导出目标缺少父目录")?;
            let result = archive_task::export(
                &app,
                &archive_task::ExportRequest {
                    temporary_parent: parent,
                    destination: &destination,
                    master: &master,
                    password: password.as_bytes(),
                },
                control,
            )?;
            Ok(Success {
                restored_root: None,
                replace_current: false,
                cleanup_warning: result.cleanup_warning,
            })
        })
    }
    pub fn start_restore_current(
        &self,
        archive: PathBuf,
        password: Zeroizing<String>,
    ) -> Result<u64> {
        crate::archive::validate_password(password.as_bytes())?;
        let root = self.root.clone();
        self.jobs.start(Kind::Restore, move |control| {
            let parent = root.parent().context("当前数据目录缺少父目录")?;
            let directory = crate::archive::staging::Directory::create(parent, "restore")?;
            let target = directory.path().join("database");
            let result = restore(archive, target, password, control);
            match result {
                Ok(mut result) => {
                    result.replace_current = true;
                    Ok(result)
                }
                Err(error) => {
                    directory
                        .cleanup()
                        .map_err(|cleanup| anyhow::anyhow!("{error:#}；暂存保留：{cleanup:#}"))?;
                    Err(error)
                }
            }
        })
    }
    pub fn start_restore(
        &self,
        archive: PathBuf,
        destination: PathBuf,
        password: Zeroizing<String>,
    ) -> Result<u64> {
        crate::archive::validate_password(password.as_bytes())?;
        self.jobs.start(Kind::Restore, move |control| {
            restore(archive, destination, password, control)
        })
    }
}
fn restore(
    archive: PathBuf,
    destination: PathBuf,
    password: Zeroizing<String>,
    control: &Control,
) -> Result<Success> {
    let result = restore_job::run(
        &restore_job::Request {
            archive: &archive,
            destination: &destination,
            password: password.as_bytes(),
        },
        control,
    )?;
    Ok(Success {
        restored_root: Some(result.root),
        replace_current: false,
        cleanup_warning: result.cleanup_warning,
    })
}

pub(super) fn main_only(label: &str) -> Result<(), String> {
    if label != "main" {
        return Err("仅主窗口可管理归档任务".into());
    }
    Ok(())
}
#[tauri::command]
pub(crate) fn archive_task_status(
    window: tauri::WebviewWindow,
    service: tauri::State<'_, Service>,
) -> Result<Option<crate::archive::jobs::Report>, String> {
    main_only(window.label())?;
    service.jobs.status().map_err(|error| format!("{error:#}"))
}
#[tauri::command]
pub(crate) fn cancel_archive_task(
    id: u64,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, Service>,
) -> Result<bool, String> {
    main_only(window.label())?;
    service
        .jobs
        .cancel(id)
        .map_err(|error| format!("{error:#}"))
}

/// 内部调用入口；启动IPC与原生路径选择将在用户入口阶段接入。
pub fn service(app: &tauri::AppHandle) -> Result<tauri::State<'_, Service>> {
    app.try_state::<Service>().context("归档后台服务尚未初始化")
}
#[cfg(test)]
mod tests;
