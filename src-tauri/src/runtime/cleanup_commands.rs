//! 主窗口暂存管理与迁移清理重试；只有预检票据可触发非空暂存删除。
use super::archive_service::main_only;
use crate::archive::{
    abandoned,
    cleanup_review::{Preview, Review},
};
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Default)]
pub(crate) struct Reviews(Mutex<Option<Review>>);
#[tauri::command]
pub(crate) async fn scan_archive_staging(
    parent: PathBuf,
    window: WebviewWindow,
) -> Result<abandoned::Report, String> {
    main_only(window.label())?;
    tauri::async_runtime::spawn_blocking(move || abandoned::scan(&parent))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))
}
#[tauri::command]
pub(crate) async fn preview_staging_cleanup(
    root: PathBuf,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<Preview, String> {
    main_only(window.label())?;
    let reviews = app.state::<Arc<Reviews>>().inner().clone();
    let operation = super::archive_service::service(&app)
        .map_err(|error| error.to_string())?
        .jobs
        .begin_restart()
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let mut saved = reviews
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("暂存清理状态不可用"))?;
        *saved = None;
        let review = Review::prepare(&root)?;
        let preview = review.preview.clone();
        *saved = Some(review);
        Ok(preview)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error: anyhow::Error| format!("{error:#}"))
}
#[tauri::command]
pub(crate) async fn confirm_staging_cleanup(
    ticket: String,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<(), String> {
    main_only(window.label())?;
    let reviews = app.state::<Arc<Reviews>>().inner().clone();
    let operation = super::archive_service::service(&app)
        .map_err(|error| error.to_string())?
        .jobs
        .begin_restart()
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let mut saved = reviews
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("暂存清理状态不可用"))?;
        if saved
            .as_ref()
            .is_none_or(|review| review.preview.ticket != ticket)
        {
            anyhow::bail!("清理预检已失效，请重新检查");
        }
        let review = saved.take().context("清理预检已失效，请重新检查")?;
        review.confirm(&ticket)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| format!("{error:#}"))
}
#[tauri::command]
pub(crate) fn cancel_staging_cleanup(
    ticket: String,
    window: WebviewWindow,
    state: tauri::State<'_, Arc<Reviews>>,
) -> Result<(), String> {
    main_only(window.label())?;
    let mut saved = state.0.lock().map_err(|_| "暂存清理状态不可用")?;
    if saved
        .as_ref()
        .is_some_and(|review| review.preview.ticket == ticket)
    {
        *saved = None;
    }
    Ok(())
}
#[tauri::command]
pub(crate) async fn retry_migration_cleanup(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<(), String> {
    main_only(window.label())?;
    let directory = app
        .state::<Arc<crate::data_directory::DirectoryControl>>()
        .inner()
        .clone();
    let operation = super::archive_service::service(&app)
        .map_err(|error| error.to_string())?
        .jobs
        .begin_restart()
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        directory.mark_migration_initialized()?;
        directory.retry_source_cleanup()
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| format!("{error:#}"))
}
