mod autostart;
mod capture_policy;
mod capture_source;
mod clipboard;
mod clipboard_diag;
mod clipboard_owner;
mod diag;
mod file_status;
pub mod hook;
mod image_normalize;
pub mod model;
mod owner_flush;
pub mod paster;
pub mod platform;
mod runtime;
pub mod sensitive;
pub mod settings;
pub mod storage;
pub mod thumbnail;

use runtime::{Control, Job, Panel, State};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

// 仅接收已经过 read_thumbnail 白名单校验的内容。
fn thumbnail_response(data: Option<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let mime = data.as_deref().and_then(thumbnail::preview_mime);
    tauri::http::Response::builder()
        .status(if mime.is_some() { 200 } else { 404 })
        .header("Content-Type", mime.unwrap_or("application/octet-stream"))
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(if mime.is_some() {
            data.unwrap_or_default()
        } else {
            Vec::new()
        })
        .unwrap()
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
        shared.files.annotate(&mut items);
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
async fn set_favorite(
    ids: Vec<String>,
    value: bool,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<usize, String> {
    update_history(window, state.inner().clone(), move |s| {
        s.set_favorite(&ids, value)
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
fn get_settings(state: tauri::State<'_, Arc<State>>) -> settings::Settings {
    state.settings.lock().unwrap().clone()
}
#[tauri::command]
fn get_setting_options() -> settings::SettingOptions {
    settings::options()
}
#[tauri::command]
fn startup_error(state: tauri::State<'_, Arc<State>>) -> Option<String> {
    state.startup_error.lock().unwrap().clone()
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
#[derive(serde::Serialize)]
struct FileDetail {
    path: String,
    exists: Option<bool>,
}
#[derive(serde::Serialize)]
struct ItemDetail {
    text: String,
    truncated: bool,
    files: Vec<FileDetail>,
}
fn detail(clip: &model::Clip) -> ItemDetail {
    let full = clip.text().unwrap_or_default();
    let mut chars = full.chars();
    let text = chars.by_ref().take(20000).collect();
    let truncated = chars.next().is_some();
    let files = clip
        .get("CF_HDROP")
        .and_then(model::decode_hdrop)
        .unwrap_or_default()
        .into_iter()
        .map(|path| FileDetail {
            exists: std::path::Path::new(&path).try_exists().ok(),
            path,
        })
        .collect();
    ItemDetail {
        text,
        truncated,
        files,
    }
}
#[tauri::command]
async fn item_detail(
    id: String,
    state: tauri::State<'_, Arc<State>>,
) -> Result<ItemDetail, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        shared
            .storage
            .lock()
            .map_err(|_| "存储不可用")?
            .display_detail(&id, false)
            .map_err(|_| "记录读取失败".into())
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
        shared
            .storage
            .lock()
            .map_err(|_| "存储不可用")?
            .display_detail(&id, true)
            .map_err(|_| "记录读取失败".into())
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
fn panel_state(state: tauri::State<'_, Arc<State>>) -> Panel {
    state.panel.lock().unwrap().clone()
}
#[tauri::command]
fn resize_quick(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
    height: f64,
    notice: bool,
) -> Result<(), String> {
    if window.label() != "quick" || !height.is_finite() {
        return Err("无效的提示窗口尺寸".into());
    }
    let panel = state.panel.lock().map_err(|_| "面板状态不可用")?;
    // 丢弃前端旧提示的延迟请求，避免缩小刚重新打开的选择面板。
    if notice != (!panel.visible && panel.error.is_some()) {
        return Ok(());
    }
    let height = if notice {
        height.clamp(60.0, 530.0)
    } else {
        530.0
    };
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    platform::resize_quick(
        state.quick.load(Ordering::Acquire),
        (height * scale).ceil() as i32,
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
fn last_error(state: tauri::State<'_, Arc<State>>) -> Option<String> {
    state.error.lock().unwrap().take()
}
#[tauri::command]
fn frontend_ready(window: tauri::WebviewWindow, state: tauri::State<'_, Arc<State>>) {
    if window.label() == "quick" {
        state.quick_ready.store(true, Ordering::Release);
        state.ready.store(true, Ordering::Release);
        hook::READY.store(
            !state.smoke && !state.items.lock().unwrap().is_empty(),
            Ordering::Release,
        );
    } else {
        state.main_ready.store(true, Ordering::Release);
        if state.settings_requested.swap(false, Ordering::AcqRel) {
            let _ = window.emit("open-settings", ());
        }
    }
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
            state.settings.lock().unwrap().main_hotkey.label()
        ));
    }
    window.hide().map_err(|_| "主窗口隐藏失败")?;
    let _ = window.emit("main-hidden", ());
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
        let _ = state.control.send(Control::Hide);
    }
}
fn icon() -> tauri::image::Image<'static> {
    let mut rgba = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let i = (y * 32 + x) * 4;
            let paper = (8..24).contains(&x) && (6..27).contains(&y);
            let line = (11..21).contains(&x) && [12, 16, 20].contains(&y);
            let color = if line {
                [34, 48, 70, 255]
            } else if paper {
                [226, 237, 246, 255]
            } else {
                [43, 112, 127, 255]
            };
            rgba[i..i + 4].copy_from_slice(&color);
        }
    }
    tauri::image::Image::new_owned(rgba, 32, 32)
}
fn smoke_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| matches!(value, "1" | "true" | "yes"))
}

fn startup_failed(error: &dyn std::fmt::Display, smoke: bool) -> ! {
    let message = format!("剪贴板 M0 启动失败：{error}");
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
pub fn run() {
    let smoke = smoke_enabled(std::env::var("CLIPBOARD_M0_SMOKE").ok().as_deref());
    // 数据错误必须在进入 Tauri 的 setup 回调前处理，避免被事件循环转成 panic。
    let data = (|| -> anyhow::Result<_> {
        let root = platform::data_root()?;
        diag::init(&root)?;
        let key = platform::master_key(&root)?;
        let storage = storage::Storage::open(&root, &key)?;
        // 迁移完成之后、runtime 启动之前扫描；维护失败只记录诊断。
        if let Err(e) = storage.scan_orphan_blobs() {
            diag::write(format!("startup orphan scan failed: {e}"));
        }
        Ok((root, storage))
    })();
    let (root, mut storage) = match data {
        Ok(data) => data,
        Err(error) => startup_failed(&error, smoke),
    };
    let result = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_settings,
            get_setting_options,
            update_settings,
            preview_cleanup,
            startup_error,
            list_items,
            delete_items,
            set_pinned,
            set_favorite,
            list_groups,
            create_group,
            rename_group,
            delete_group,
            move_to_group,
            item_detail,
            reveal_item,
            set_sensitive,
            image_failure_reason,
            panel_state,
            resize_quick,
            last_error,
            frontend_ready,
            thumbnail_loaded,
            preview_loaded,
            paste_item,
            hide_main
        ])
        .register_uri_scheme_protocol("thumb", |context, request| {
            let id = request.uri().path().trim_start_matches('/');
            let state = context.app_handle().try_state::<Arc<State>>();
            let data = if request.method() == tauri::http::Method::GET {
                state.and_then(|s| {
                    s.storage
                        .lock()
                        .ok()
                        .and_then(|db| db.read_thumbnail(id).ok())
                })
            } else {
                None
            };
            thumbnail_response(data)
        })
        .setup(move |app| {
            if smoke {
                // 隔离检查使用合成透明图片，不读取真实剪贴板。
                let mut encoded = std::io::Cursor::new(Vec::new());
                let image =
                    image::RgbaImage::from_pixel(1000, 400, image::Rgba([32, 128, 96, 128]));
                image::DynamicImage::ImageRgba8(image)
                    .write_to(&mut encoded, image::ImageFormat::Png)?;
                storage.save(&model::Clip {
                    formats: vec![model::FormatData {
                        name: "PNG".into(),
                        bytes: encoded.into_inner(),
                    }],
                })?;
            }
            let state = runtime::start(app.handle().clone(), storage, smoke)?;
            app.manage(state.clone());
            let main = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("剪贴板 M0")
                .inner_size(1000.0, 700.0)
                .visible(false)
                .data_directory(root.join("webview"))
                .build()?;
            let quick = WebviewWindowBuilder::new(
                app,
                "quick",
                WebviewUrl::App("index.html?panel=quick".into()),
            )
            .title("快速粘贴")
            .inner_size(420.0, 530.0)
            .visible(false)
            .focused(false)
            .focusable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .data_directory(root.join("webview"))
            .build()?;
            // 忽略鼠标命中，避免 WebView 子窗口激活。
            quick.set_ignore_cursor_events(true)?;
            let main_hwnd = main.hwnd()?.0 as isize;
            let quick_hwnd = quick.hwnd()?.0 as isize;
            state.main.store(main_hwnd, Ordering::Release);
            state.quick.store(quick_hwnd, Ordering::Release);
            platform::prepare_quick(quick_hwnd)?;
            platform::prepare_main(main_hwnd)?;
            if state.startup_error.lock().unwrap().is_some() {
                // 注册失败仍启动主界面，错误持久显示，允许用户修复设置。
                main.show()?;
            }
            use tauri::{
                menu::{Menu, MenuItem},
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
            };
            if !smoke {
                let show = MenuItem::with_id(app, "show", "显示 / 隐藏", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
                let pause = MenuItem::with_id(
                    app,
                    "pause",
                    if runtime::paused() {
                        "恢复监听"
                    } else {
                        "暂停监听"
                    },
                    true,
                    None::<&str>,
                )?;
                app.manage(PauseMenu(pause.clone()));
                let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show, &pause, &settings, &quit])?;
                TrayIconBuilder::with_id("main-tray")
                    .icon(tray_icon(runtime::paused()))
                    .tooltip(if runtime::paused() {
                        "剪贴板 M1 · 已暂停监听"
                    } else {
                        "剪贴板 M1 · 正在监听"
                    })
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            let _ = app.state::<Arc<State>>().control.send(Control::Toggle);
                        }
                        "pause" => {
                            let _ = app.state::<Arc<State>>().control.send(Control::TogglePause);
                        }
                        "settings" => {
                            let _ = app
                                .state::<Arc<State>>()
                                .control
                                .send(Control::OpenSettings);
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if matches!(
                            event,
                            TrayIconEvent::Click {
                                button: MouseButton::Left,
                                button_state: MouseButtonState::Up,
                                ..
                            }
                        ) {
                            let _ = tray
                                .app_handle()
                                .state::<Arc<State>>()
                                .control
                                .send(Control::Toggle);
                        }
                    })
                    .build(app)?;
            } else {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    let ready = || {
                        state.main_ready.load(Ordering::Acquire)
                            && state.quick_ready.load(Ordering::Acquire)
                            && state.thumbnail_ready.load(Ordering::Acquire)
                            && state.preview_ready.load(Ordering::Acquire)
                    };
                    for _ in 0..100 {
                        std::thread::sleep(Duration::from_millis(100));
                        if ready() {
                            break;
                        }
                    }
                    let report = serde_json::json!({
                        "main_frontend_ready": state.main_ready.load(Ordering::Acquire),
                        "quick_frontend_ready": state.quick_ready.load(Ordering::Acquire),
                        "thumbnail_protocol_loaded": state.thumbnail_ready.load(Ordering::Acquire),
                        "preview_protocol_loaded": state.preview_ready.load(Ordering::Acquire),
                        "clipboard_listener_enabled": false,
                        "keyboard_hook_enabled": false,
                        "visual_verified": false
                    });
                    let _ = std::fs::write(
                        root.join("smoke-result.json"),
                        serde_json::to_vec_pretty(&report).unwrap(),
                    );
                    handle.exit(if ready() { 0 } else { 1 });
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                    let _ = window.emit("main-hidden", ());
                    diag::panel(false, false);
                }
            }
        })
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|_, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                runtime::stop();
            }
        }),
        Err(error) => startup_failed(&error, smoke),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // AC-T8-2：比较实际图标像素；不创建托盘或操作真实桌面。
    #[test]
    fn t8_paused_tray_icon_is_visibly_distinct_and_same_size() {
        let active = tray_icon(false);
        let paused = tray_icon(true);
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

struct PauseMenu(tauri::menu::MenuItem<tauri::Wry>);
fn tray_icon(paused: bool) -> tauri::image::Image<'static> {
    if !paused {
        return icon();
    }
    let mut rgba = icon().rgba().to_vec();
    for y in 8..25 {
        for x in [11, 12, 13, 18, 19, 20] {
            let i = (y * 32 + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[220, 120, 20, 255]);
        }
    }
    tauri::image::Image::new_owned(rgba, 32, 32)
}
fn update_tray(app: &tauri::AppHandle, paused: bool) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_icon(Some(tray_icon(paused)));
        let _ = tray.set_tooltip(Some(if paused {
            "剪贴板 M1 · 已暂停监听"
        } else {
            "剪贴板 M1 · 正在监听"
        }));
    }
    if let Some(menu) = app.try_state::<PauseMenu>() {
        let _ = menu.0.set_text(if paused {
            "恢复监听"
        } else {
            "暂停监听"
        });
    }
}
