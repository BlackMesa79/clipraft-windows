mod capture_policy;
mod clipboard;
mod clipboard_diag;
mod clipboard_owner;
mod diag;
pub mod hook;
mod image_normalize;
pub mod model;
mod owner_flush;
pub mod paster;
pub mod platform;
mod runtime;
pub mod storage;
pub mod thumbnail;

use runtime::{Control, Job, Panel, State};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

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
        shared
            .storage
            .lock()
            .map_err(|_| "存储不可用")?
            .list_page(&filter, offset, limit)
            .map_err(|e| format!("历史列表读取失败：{e}"))
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
        let clip = shared
            .storage
            .lock()
            .map_err(|_| "存储不可用")?
            .load(&id)
            .map_err(|_| "记录读取失败")?;
        Ok(detail(&clip))
    })
    .await
    .map_err(|_| "预览任务失败")?
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
        return Err("没有可用的目标窗口，请先激活目标程序，再按 Alt+V".into());
    }
    window.hide().map_err(|_| "主窗口隐藏失败")?;
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
    // 冒烟检查只写日志，正常启动失败时也让无控制台的构建显示原因。
    if !smoke {
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
            list_items,
            delete_items,
            item_detail,
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
            use tauri::{
                menu::{Menu, MenuItem},
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
            };
            if !smoke {
                let show = MenuItem::with_id(app, "show", "显示 / 隐藏", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show, &quit])?;
                TrayIconBuilder::new()
                    .icon(icon())
                    .tooltip("剪贴板 M0 · Alt+V")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            let _ = app.state::<Arc<State>>().control.send(Control::Toggle);
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
