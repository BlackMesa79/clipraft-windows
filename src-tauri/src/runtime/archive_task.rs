//! 一次完整导出：独占快照、运行时维护、密码归档和快照回收。
use super::{snapshot_task, State};
use crate::{
    archive::{control::Control, export_job, staging::Directory},
    storage::SnapshotFiles,
};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::Manager;

pub struct ExportRequest<'a> {
    pub temporary_parent: &'a Path,
    pub destination: &'a Path,
    pub master: &'a [u8; 32],
    pub password: &'a [u8],
}

/// 不切换当前库。密码归档前快照维护已结束，源存储锁已释放。
pub fn export(
    app: &tauri::AppHandle,
    request: &ExportRequest<'_>,
    control: &Control,
) -> Result<export_job::Outcome> {
    let state = app.try_state::<Arc<State>>().context("运行时尚未初始化")?;
    export_with_snapshot(request, control, |files| {
        snapshot_task::create_owned(&state, files, request.master, control)
    })
}
fn export_with_snapshot(
    request: &ExportRequest<'_>,
    control: &Control,
    snapshot: impl FnOnce(&mut SnapshotFiles) -> Result<()>,
) -> Result<export_job::Outcome> {
    control.start()?;
    let result = run(request, control, snapshot);
    if result.is_err() {
        control.fail()?;
    }
    result
}
fn run(
    request: &ExportRequest<'_>,
    control: &Control,
    snapshot: impl FnOnce(&mut SnapshotFiles) -> Result<()>,
) -> Result<export_job::Outcome> {
    let destination = validate(request)?;
    control.check()?;
    let parent = fs::canonicalize(request.temporary_parent)?;
    let directory = Directory::create(&parent, "snapshot")?;
    let mut files = SnapshotFiles::new(&directory.path().join("database"));
    let result = snapshot(&mut files).and_then(|()| {
        control.snapshot_prepared()?;
        export_job::run_prepared(
            &export_job::Request {
                snapshot: files.root(),
                master: request.master,
                password: request.password,
                destination: &destination,
            },
            control,
        )
    });
    let cleanup = files.cleanup().and_then(|()| directory.cleanup());
    finish(result, cleanup)
}
fn validate(request: &ExportRequest<'_>) -> Result<PathBuf> {
    crate::archive::validate_password(request.password)?;
    if !request.temporary_parent.is_absolute() || !request.destination.is_absolute() {
        bail!("导出与暂存位置必须为绝对路径");
    }
    if request.destination.try_exists()? {
        bail!("导出目标已存在，不会覆盖已有文件");
    }
    let parent = fs::canonicalize(request.destination.parent().context("导出目标缺少父目录")?)?;
    Ok(parent.join(
        request
            .destination
            .file_name()
            .context("导出目标缺少文件名")?,
    ))
}
fn finish(result: Result<export_job::Outcome>, cleanup: Result<()>) -> Result<export_job::Outcome> {
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => Err(error.context(format!("快照暂存清理失败：{cleanup:#}"))),
        (Err(error), Ok(())) => Err(error),
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Ok(mut outcome), Err(cleanup)) => {
            let warning = format!("归档已发布，快照暂存清理失败：{cleanup:#}");
            outcome.cleanup_warning = Some(match outcome.cleanup_warning {
                Some(previous) => format!("{previous}；{warning}"),
                None => warning,
            });
            Ok(outcome)
        }
    }
}

#[cfg(test)]
mod tests;
