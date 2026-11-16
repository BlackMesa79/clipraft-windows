mod app_events;
mod app_setup;
mod app_smoke;
mod app_tray;
mod appearance;
pub mod archive;
mod autostart;
mod capture_policy;
mod capture_source;
mod clipboard;
mod clipboard_diag;
mod clipboard_owner;
mod compact_preview;
mod data_directory;
mod diag;
mod file_actions;
mod file_status;
pub mod hook;
mod hotkey_check;
mod image_normalize;
mod item_detail;
mod main_position;
pub mod model;
mod owned_files;
mod owner_flush;
pub mod panel_neighborhood;
pub mod panel_session;
pub mod paster;
pub mod platform;
mod quick_geometry;
mod rich_preview;
mod runtime;
pub use runtime::{archive_service, archive_task, snapshot_task};
pub mod sensitive;
pub mod settings;
pub mod storage;
pub mod thumbnail;
mod tray_status;

use item_detail::ItemDetail;
use runtime::{Control, Job, Panel, State};
use std::sync::{atomic::Ordering, Arc};
use tauri::{Emitter, Manager};

// 仅接收已经过 read_thumbnail 白名单校验的内容。
fn thumbnail_response(data: Option<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let mime = data.as_deref().and_then(thumbnail::preview_mime);
    let response = tauri::http::Response::builder()
        .status(if mime.is_some() { 200 } else { 404 })
        .header("Content-Type", mime.unwrap_or("application/octet-stream"))
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(if mime.is_some() {
            data.unwrap_or_default()
        } else {
            Vec::new()
        });
    match response {
        Ok(response) => response,
        Err(error) => {
            diag::write(format!("thumbnail response build failed: {error}"));
            let mut response = tauri::http::Response::new(Vec::new());
            *response.status_mut() = tauri::http::StatusCode::INTERNAL_SERVER_ERROR;
            response
        }
    }
}

#[tauri::command]
async fn list_items(
    filter: storage::ListFilter,
    offset: usize,
    limit: usize,
    state: tauri::State<'_, Arc<State>>,
) -> Result<Vec<model::ItemView>, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut items = shared
            .storage
            .lock()
            .map_err(|_| "存储不可用")?
            .list_page(&filter, offset, limit)
            .map_err(|e| format!("历史列表读取失败：{e}"))?;
        shared
            .files
            .annotate(&mut items)
            .map_err(|e| e.to_string())?;
        Ok(items)
    })
    .await
    .map_err(|_| "历史列表任务失败")?
}
#[tauri::command]
async fn delete_items(
    ids: Vec<String>,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<storage::DeleteReport, String> {
    if window.label() != "main" || state.smoke {
        return Err("此窗口不能执行删除".into());
    }
    let shared = state.inner().clone();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 与保存/自动清理一致：先 capture_context 后 storage，避免反向嵌套死锁。
        let _change = runtime::history_gate::begin(&shared).map_err(|e| e.to_string())?;
        let mut context = shared
            .capture_context
            .lock()
            .map_err(|_| "采集上下文不可用")?;
        let mut report = shared
            .storage
            .lock()
            .map_err(|_| "存储不可用")?
            .delete(&ids)
            .map_err(|e| format!("删除失败：{e}"))?;
        owner_flush::invalidate_deleted(&mut context, &report.deleted_ids);
        drop(context);
        if let Err(e) = runtime::refresh(&app, &shared) {
            let message = format!("记录已删除，但刷新失败：{e}");
            diag::write(message.clone());
            report.maintenance_errors.push(message);
        }
        Ok(report)
    })
    .await
    .map_err(|_| "删除任务失败")?
}
// 所有 T5 修改只允许主窗口调用；事务结束并释放 storage 锁后刷新缓存。
async fn update_history<T: Send + 'static>(
    window: tauri::WebviewWindow,
    state: Arc<State>,
    change: impl FnOnce(&mut storage::Storage) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, String> {
    if window.label() != "main" || state.smoke {
        return Err("此窗口不能修改历史或分组".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _change = runtime::history_gate::begin(&state).map_err(|e| e.to_string())?;
        let result = {
            let mut store = state.storage.lock().map_err(|_| "存储不可用")?;
            change(&mut store).map_err(|e| e.to_string())?
        };
        runtime::refresh(&app, &state).map_err(|e| format!("修改已保存，但刷新失败：{e}"))?;
        Ok(result)
    })
    .await
    .map_err(|_| "修改历史任务失败")?
}

#[tauri::command]
async fn set_pinned(
    ids: Vec<String>,
    value: bool,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<usize, String> {
    update_history(window, state.inner().clone(), move |s| {
        s.set_pinned(&ids, value)
    })
    .await
}
#[tauri::command]
async fn list_groups(
    state: tauri::State<'_, Arc<State>>,
) -> Result<Vec<storage::GroupView>, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        shared
            .storage
            .lock()
            .map_err(|_| "存储不可用")?
            .list_groups()
            .map_err(|e| format!("分组读取失败：{e}"))
    })
    .await
    .map_err(|_| "分组读取任务失败")?
}
#[tauri::command]
async fn create_group(
    name: String,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<storage::GroupView, String> {
    update_history(window, state.inner().clone(), move |s| {
        s.create_group(&name)
    })
    .await
}
#[tauri::command]
async fn rename_group(
    id: i64,
    name: String,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<(), String> {
    update_history(window, state.inner().clone(), move |s| {
        s.rename_group(id, &name)
    })
    .await
}
#[tauri::command]
async fn delete_group(
    id: i64,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<(), String> {
    update_history(window, state.inner().clone(), move |s| s.delete_group(id)).await
}
#[tauri::command]
async fn move_to_group(
    ids: Vec<String>,
    group_id: Option<i64>,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<usize, String> {
    update_history(window, state.inner().clone(), move |s| {
        s.move_to_group(&ids, group_id)
    })
    .await
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, Arc<State>>) -> Result<settings::Settings, String> {
    state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .map_err(|_| "设置不可用".into())
}
#[tauri::command]
fn get_setting_options() -> settings::SettingOptions {
    settings::options()
}
#[tauri::command]
fn startup_error(state: tauri::State<'_, Arc<State>>) -> Result<Option<String>, String> {
    state
        .startup_error
        .lock()
        .map(|error| error.clone())
        .map_err(|_| "启动错误状态不可用".into())
}
#[tauri::command]
async fn update_settings(
    settings: serde_json::Value,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<settings::Settings, String> {
    if window.label() != "main" || state.smoke {
        return Err("此窗口不能修改设置".into());
    }
    let settings: settings::Settings = serde_json::from_value(settings)
        .map_err(|_| "设置格式无效，请检查字段类型、数值及快捷键候选项".to_string())?;
    settings.validate().map_err(|e| e.to_string())?;
    let control = state.control.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (reply, wait) = std::sync::mpsc::channel();
        control
            .send(Control::UpdateSettings(settings, reply))
            .map_err(|_| "设置控制线程已退出")?;
        wait.recv().map_err(|_| "设置控制线程已退出")?
    })
    .await
    .map_err(|_| "设置任务失败")?
}
#[tauri::command]
fn set_hotkey_recording(
    active: bool,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<u64, String> {
    if window.label() != "main" || state.smoke {
        return Err("仅主窗口可录制快捷键".into());
    }
    if active {
        let raw = window.hwnd().map_err(|_| "窗口不可用")?.0 as isize;
        if platform::foreground() != raw {
            return Err("请先激活设置窗口".into());
        }
        let _guard = hook::reserve_configuration_when_hidden().map_err(|e| e.to_string())?;
        runtime::new_paste_intent();
        let session = hook::start_recording(raw);
        diag::write(format!("hotkey_record stage=start session={session}"));
        Ok(session)
    } else {
        if hook::recording() {
            app_events::record("recording shortcut mask failed", paster::mask_shortcut());
        }
        let session = hook::stop_recording();
        diag::write(format!("hotkey_record stage=stop session={session}"));
        Ok(session)
    }
}
#[tauri::command]
async fn check_hotkeys(
    main_hotkey: settings::Hotkey,
    quick_hotkey: settings::Hotkey,
    window: tauri::WebviewWindow,
) -> Result<hotkey_check::Report, String> {
    if window.label() != "main" {
        return Err("仅主窗口可检测快捷键".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        runtime::check_hotkeys(main_hotkey, quick_hotkey).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "热键检测任务失败")?
}
#[tauri::command]
async fn preview_cleanup(
    policy: storage::RetentionPolicy,
    state: tauri::State<'_, Arc<State>>,
) -> Result<usize, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        shared
            .storage
            .lock()
            .map_err(|_| "存储不可用")?
            .preview_cleanup(&policy, storage::unix_millis())
            .map_err(|e| format!("清理预估失败：{e}"))
    })
    .await
    .map_err(|_| "清理预估任务失败")?
}
#[tauri::command]
async fn item_detail(
    id: String,
    state: tauri::State<'_, Arc<State>>,
) -> Result<ItemDetail, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        item_detail::read::display(&shared.storage, &id)
            .map(|(_, detail)| detail)
            .map_err(|_| "记录读取失败或状态已变化".into())
    })
    .await
    .map_err(|_| "预览任务失败")?
}
#[tauri::command]
async fn reveal_item(
    id: String,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<ItemDetail, String> {
    if window.label() != "main" || state.smoke {
        return Err("此窗口不能显示原文".into());
    }
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        item_detail::read::reveal(&shared.storage, &id)
            .map(|(_, detail)| detail)
            .map_err(|_| "记录读取失败或状态已变化".into())
    })
    .await
    .map_err(|_| "显示原文任务失败")?
}
#[tauri::command]
async fn set_sensitive(
    ids: Vec<String>,
    value: bool,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<usize, String> {
    let ttl = state
        .settings
        .lock()
        .map_err(|_| "设置不可用")?
        .sensitive_ttl_minutes;
    update_history(window, state.inner().clone(), move |s| {
        s.set_sensitive(&ids, value, ttl)
    })
    .await
}
#[tauri::command]
async fn keep_sensitive(
    ids: Vec<String>,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<usize, String> {
    update_history(window, state.inner().clone(), move |storage| {
        storage.keep_sensitive(&ids)
    })
    .await
}
#[tauri::command]
async fn image_failure_reason(
    id: String,
    state: tauri::State<'_, Arc<State>>,
) -> Result<String, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        match shared
            .storage
            .lock()
            .map_err(|_| "存储不可用")?
            .read_thumbnail(&id)
        {
            Ok(_) => Ok("PNG 对象可读取，但浏览器加载或解码失败".into()),
            Err(error) => Ok(error.to_string()),
        }
    })
    .await
    .map_err(|_| "图片诊断任务失败")?
}
#[tauri::command]
fn panel_state(state: tauri::State<'_, Arc<State>>) -> Result<Panel, String> {
    runtime::panel_snapshot(&state).map_err(|error| error.to_string())
}
#[tauri::command]
fn resize_quick(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
    height: f64,
    notice: bool,
    session: u64,
) -> Result<(), String> {
    if window.label() != "quick" || !height.is_finite() {
        return Err("无效的提示窗口尺寸".into());
    }
    let panel = state.panel.lock().map_err(|_| "面板状态不可用")?;
    // 丢弃前端旧提示的延迟请求，避免缩小刚重新打开的选择面板。
    if !quick_geometry::accepts_resize(
        panel.visible,
        panel.error.is_some(),
        panel.session,
        notice,
        session,
    ) {
        return Ok(());
    }
    let height = if notice {
        height.clamp(60.0, 530.0)
    } else {
        quick_geometry::dimensions(panel.visible_count)
            .map_err(|e| e.to_string())?
            .1
    };
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    platform::resize_quick(
        state.quick.load(Ordering::Acquire),
        (height * scale).ceil() as i32,
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
fn last_error(state: tauri::State<'_, Arc<State>>) -> Result<Option<String>, String> {
    state
        .error
        .lock()
        .map(|mut error| error.take())
        .map_err(|_| "错误状态不可用".into())
}
#[tauri::command]
fn frontend_ready(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<bool, String> {
    if window.label() == "quick" {
        let has_items =
            !state.smoke && !state.items.lock().map_err(|_| "面板条目不可用")?.is_empty();
        state.quick_ready.store(true, Ordering::Release);
        state.ready.store(true, Ordering::Release);
        hook::READY.store(has_items, Ordering::Release);
    } else {
        state.main_ready.store(true, Ordering::Release);
        if state.settings_requested.swap(false, Ordering::AcqRel) {
            window
                .emit("open-settings", ())
                .inspect_err(|_| state.settings_requested.store(true, Ordering::Release))
                .map_err(|error| error.to_string())?;
        }
    }
    if window.label() == "main" {
        data_directory::cleanup::after_frontend_ready(window.app_handle());
    }
    if diag::enabled() {
        diag::write(format!(
            "frontend_ready window={} hook_ready={}",
            window.label(),
            hook::READY.load(Ordering::Acquire)
        ));
    }
    Ok(diag::enabled())
}
#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
enum PanelDiagnosticStage {
    Received,
    Frame,
}
#[tauri::command]
fn panel_diagnostic(
    window: tauri::WebviewWindow,
    stage: PanelDiagnosticStage,
    serial: u64,
    session: u64,
) -> Result<(), String> {
    if window.label() != "quick" {
        return Err("仅快速面板可报告诊断".into());
    }
    if diag::enabled() {
        diag::write(format!(
            "panel_frontend stage={stage:?} serial={serial} session={session}"
        ));
    }
    Ok(())
}
#[tauri::command]
fn thumbnail_loaded(state: tauri::State<'_, Arc<State>>) {
    if state.smoke {
        state.thumbnail_ready.store(true, Ordering::Release);
    }
}
#[tauri::command]
fn preview_loaded(state: tauri::State<'_, Arc<State>>) {
    if state.smoke {
        state.preview_ready.store(true, Ordering::Release);
    }
}
#[tauri::command]
fn paste_item(
    id: String,
    plain: bool,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<(), String> {
    if window.label() != "main" || state.smoke {
        return Err("此窗口不能执行粘贴".into());
    }
    let generation = runtime::paste_generation();
    let target = state.target.load(Ordering::Acquire);
    if !platform::valid_external(target) {
        return Err(format!(
            "没有可用的目标窗口，请先激活目标程序，再按 {}",
            state
                .settings
                .lock()
                .map_err(|_| "设置不可用")?
                .main_hotkey
                .label()
        ));
    }
    window.hide().map_err(|_| "主窗口隐藏失败")?;
    app_events::record("main hidden event failed", window.emit("main-hidden", ()));
    diag::panel(false, false);
    state
        .jobs
        .try_send(Job::Paste {
            id,
            target,
            restore: true,
            plain,
            generation,
        })
        .map_err(|_| {
            runtime::report(window.app_handle(), &state, "任务队列繁忙，请重试");
            "任务队列繁忙，请重试".into()
        })
}
#[tauri::command]
fn hide_main(window: tauri::WebviewWindow, state: tauri::State<'_, Arc<State>>) {
    if window.label() == "main" {
        if let Err(error) = state.control.send(Control::Hide) {
            diag::write(format!("main hide control unavailable: {error}"));
        }
    }
}
#[tauri::command]
fn sync_main_layout(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
    layout: tauri::State<'_, appearance::WindowLayout>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("仅主窗口可以调整布局".into());
    }
    let compact = state.settings.lock().map_err(|_| "设置不可用")?.compact;
    layout.apply(&window, compact).map_err(|e| e.to_string())
}
fn smoke_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| matches!(value, "1" | "true" | "yes"))
}

fn startup_failed(error: &dyn std::fmt::Display, smoke: bool) -> ! {
    let message = format!("片筏剪贴板 启动失败：{error}");
    eprintln!("{message}");
    diag::startup_failure(&message);
    if !platform::unattended_startup(
        smoke,
        std::env::var_os("CLIPBOARD_M0_DATA_DIR").is_some(),
        std::env::var_os("CLIPBOARD_M0_DIAG").is_some(),
    ) {
        platform::show_startup_error(&message);
    }
    std::process::exit(1);
}
fn read_protocol_thumbnail(app: &tauri::AppHandle, id: &str) -> Option<Vec<u8>> {
    let state = app.try_state::<Arc<State>>()?;
    let result = state
        .storage
        .lock()
        .map_err(|_| "存储不可用".to_string())
        .and_then(|db| db.read_thumbnail(id).map_err(|error| error.to_string()));
    match result {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            diag::write(format!("thumbnail request failed: {error}"));
            None
        }
    }
}

fn application_builder() -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            data_directory::commands::data_directory_status,
            data_directory::commands::choose_data_directory,
            data_directory::commands::restore_data_directory,
            data_directory::commands::migrate_data_directory,
            get_settings,
            get_setting_options,
            set_hotkey_recording,
            runtime::archive_commands::activate_restored_archive,
            runtime::cleanup_commands::scan_archive_staging,
            runtime::cleanup_commands::preview_staging_cleanup,
            runtime::cleanup_commands::confirm_staging_cleanup,
            runtime::cleanup_commands::cancel_staging_cleanup,
            runtime::cleanup_commands::retry_migration_cleanup,
            runtime::archive_commands::start_archive_export,
            runtime::archive_commands::start_archive_restore,
            runtime::archive_commands::choose_archive_path,
            archive_service::archive_task_status,
            archive_service::cancel_archive_task,
            check_hotkeys,
            update_settings,
            preview_cleanup,
            startup_error,
            list_items,
            delete_items,
            set_pinned,
            list_groups,
            create_group,
            rename_group,
            delete_group,
            move_to_group,
            item_detail,
            file_actions::file_action,
            reveal_item,
            set_sensitive,
            keep_sensitive,
            image_failure_reason,
            panel_state,
            resize_quick,
            last_error,
            frontend_ready,
            panel_diagnostic,
            thumbnail_loaded,
            preview_loaded,
            paste_item,
            hide_main,
            sync_main_layout,
            compact_preview::sync_compact_preview,
            compact_preview::position_compact_preview,
            compact_preview::resize_compact_preview,
            compact_preview::compact_preview_geometry,
            compact_preview::compact_preview_state
        ])
        .register_uri_scheme_protocol("thumb", |context, request| {
            let id = request.uri().path().trim_start_matches('/');
            let data = if request.method() == tauri::http::Method::GET {
                read_protocol_thumbnail(context.app_handle(), id)
            } else {
                None
            };
            thumbnail_response(data)
        })
}

fn open_storage() -> anyhow::Result<(data_directory::Session, storage::Storage)> {
    let session = data_directory::Session::open()?;
    let root = &session.root;
    diag::init(root)?;
    let key = platform::master_key(root)?;
    let storage = storage::Storage::open(root, &key)?;
    session.persist_location()?;
    // 迁移完成之后、runtime 启动之前扫描；维护失败只记录诊断。
    if let Err(e) = storage.scan_orphan_blobs() {
        diag::write(format!("startup orphan scan failed: {e}"));
    }
    Ok((session, storage))
}
pub fn run() {
    let smoke = smoke_enabled(std::env::var("CLIPBOARD_M0_SMOKE").ok().as_deref());
    // 数据错误必须在进入 setup 前处理，避免被事件循环转成 panic。
    let data = open_storage();
    let (session, storage) = match data {
        Ok(data) => data,
        Err(error) => startup_failed(&error, smoke),
    };
    let startup_capture = session.startup_capture;
    let root = session.root.clone();
    let cache = session.cache.clone();
    let control = session.control.clone();
    let result = application_builder()
        .setup(move |app| {
            app.manage(startup_capture);
            app.manage(control);
            app.manage(std::sync::Arc::new(
                runtime::cleanup_commands::Reviews::default(),
            ));
            app.manage(archive_service::Service::new(root.clone()));
            let result = if smoke {
                app_setup::initialize_smoke(app, storage, root, cache)
            } else {
                app_setup::initialize(app, storage, cache)
            };
            result.map_err(Into::into)
        })
        .on_window_event(app_events::on_window_event)
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|_, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                runtime::stop();
            }
        }),
        Err(error) => startup_failed(&error, smoke),
    }
    drop(session);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_detail::from_clip as detail;
    // AC-T8-2：比较实际图标像素；不创建托盘或操作真实桌面。
    #[test]
    fn t8_paused_tray_icon_is_visibly_distinct_and_same_size() {
        let active = tray_status::icon().unwrap();
        let paused = tray_status::paused_icon().unwrap();
        assert_eq!((active.width(), active.height()), (32, 32));
        assert_eq!((paused.width(), paused.height()), (32, 32));
        assert_ne!(active.rgba(), paused.rgba());
        assert_eq!(paused.rgba().len(), 32 * 32 * 4);
    }
    #[test]
    fn file_detail_reports_current_existence_and_full_paths() {
        let existing = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("Cargo.toml")
            .to_string_lossy()
            .into_owned();
        let missing = format!("{existing}.missing-{}", std::process::id());
        let clip = model::Clip {
            formats: vec![model::FormatData {
                name: "CF_HDROP".into(),
                bytes: model::encode_hdrop(&[existing.clone(), missing.clone()]),
            }],
        };
        let result = detail(&clip);
        assert_eq!(result.files[0].path, existing);
        assert_eq!(result.files[0].exists, Some(true));
        assert_eq!(result.files[1].path, missing);
        assert_eq!(result.files[1].exists, Some(false));
    }
    #[test]
    fn text_detail_limits_unicode_characters_without_splitting_surrogates() {
        let text = "😀".repeat(20001);
        let clip = model::Clip {
            formats: vec![model::FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: text
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            }],
        };
        let result = detail(&clip);
        assert_eq!(result.text.chars().count(), 20000);
        assert!(result.truncated);
        assert!(!result.text.contains('�'));
    }
    #[test]
    fn smoke_only_accepts_explicit_enabled_values() {
        for value in ["1", "true", "yes"] {
            assert!(smoke_enabled(Some(value)));
        }
        for value in ["0", "false", "no", "", "anything", "TRUE", " 1 "] {
            assert!(!smoke_enabled(Some(value)));
        }
        assert!(!smoke_enabled(None));
    }
}

#[cfg(test)]
mod paste_bench;

#[cfg(test)]
mod test_support;
