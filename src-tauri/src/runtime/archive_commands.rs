//! 主窗口归档启动和原生路径选择；不隐式切换当前数据库。
use super::archive_service::{main_only, Service};
use serde::Deserialize;
use std::path::PathBuf;
use tauri::{AppHandle, State, WebviewWindow};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportInput {
    destination: PathBuf,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreInput {
    archive: PathBuf,
    #[serde(default)]
    destination: Option<PathBuf>,
    password: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Selection {
    Archive,
    Folder,
}

#[tauri::command]
pub(crate) fn start_archive_export(
    input: ExportInput,
    window: WebviewWindow,
    app: AppHandle,
    service: State<'_, Service>,
) -> Result<u64, String> {
    let password = Zeroizing::new(input.password);
    main_only(window.label())?;
    service
        .start_export(app, input.destination, password)
        .map_err(|error| format!("{error:#}"))
}
#[tauri::command]
pub(crate) fn start_archive_restore(
    input: RestoreInput,
    window: WebviewWindow,
    service: State<'_, Service>,
) -> Result<u64, String> {
    let password = Zeroizing::new(input.password);
    main_only(window.label())?;
    let result = match input.destination {
        Some(destination) => service.start_restore(input.archive, destination, password),
        None => service.start_restore_current(input.archive, password),
    };
    result.map_err(|error| format!("{error:#}"))
}
#[tauri::command]
pub(crate) async fn choose_archive_path(
    selection: Selection,
    window: WebviewWindow,
) -> Result<Option<PathBuf>, String> {
    main_only(window.label())?;
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as isize;
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::spawn(move || match selection {
            Selection::Archive => crate::data_directory::select_archive(Some(owner)),
            Selection::Folder => crate::data_directory::select_archive_folder(Some(owner)),
        })
        .join()
        .map_err(|_| anyhow::anyhow!("归档路径选择任务异常"))?
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub(crate) async fn activate_restored_archive(
    id: u64,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<(), String> {
    use tauri::Manager;
    main_only(window.label())?;
    let (success, restart) = super::archive_service::service(&app)
        .map_err(|error| error.to_string())?
        .jobs
        .restored_for_restart(id)
        .map_err(|error| error.to_string())?;
    let directory = app
        .state::<std::sync::Arc<crate::data_directory::DirectoryControl>>()
        .inner()
        .clone();
    let root = success.restored_root.ok_or("恢复任务缺少目录")?;
    let guard = tauri::async_runtime::spawn_blocking(move || {
        if success.replace_current {
            return directory.schedule_replacement(&root);
        }
        directory.activate_restored(&root)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| format!("{error:#}"))?;
    restart.commit();
    app.request_restart();
    drop(guard);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_inputs_reject_unknown_fields_and_invalid_selection() {
        assert!(serde_json::from_value::<ExportInput>(serde_json::json!({"destination":"D:/synthetic/history.clipraft", "password":"synthetic", "master":"not accepted"})).is_err());
        assert!(serde_json::from_value::<RestoreInput>(serde_json::json!({"archive":"D:/synthetic/history.clipraft", "destination":"D:/synthetic/restored", "password":"synthetic"})).is_ok());
        let default: RestoreInput = serde_json::from_value(
            serde_json::json!({"archive":"D:/synthetic/history.clipraft","password":"synthetic"}),
        )
        .unwrap();
        assert!(default.destination.is_none());
        assert!(serde_json::from_str::<Selection>("\"unknown\"").is_err());
    }
}
