//! 应用启动的窗口装配；数据初始化和 IPC 注册仍由入口负责。
use crate::{
    app_smoke, app_tray, appearance, compact_preview, platform, runtime, storage::Storage,
};
use anyhow::{anyhow, Result};
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
use tauri::{Listener, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub(crate) fn initialize(app: &mut tauri::App, storage: Storage, cache: PathBuf) -> Result<()> {
    let state = runtime::start(app.handle().clone(), storage, false)?;
    create_windows(app, &state, &cache)?;
    app_tray::create(app)?;
    if let Some(directory) =
        app.try_state::<std::sync::Arc<crate::data_directory::DirectoryControl>>()
    {
        if let Err(error) = directory.acknowledge_restored_startup() {
            crate::diag::write(format!(
                "restored startup marker acknowledgement failed: {error:#}"
            ));
        }
    }
    Ok(())
}

pub(crate) fn initialize_smoke(
    app: &mut tauri::App,
    mut storage: Storage,
    root: PathBuf,
    cache: PathBuf,
) -> Result<()> {
    app_smoke::seed(&mut storage)?;
    let state = runtime::start(app.handle().clone(), storage, true)?;
    create_windows(app, &state, &cache)?;
    app_smoke::start_report(app.handle().clone(), state, root)
}

fn create_windows(
    app: &tauri::App,
    state: &std::sync::Arc<runtime::State>,
    root: &Path,
) -> Result<()> {
    app.manage(state.clone());
    app.manage(appearance::WindowLayout::default());
    app.manage(compact_preview::PreviewState::default());
    let main = create_main(app, root)?;
    create_preview(app, &main, root)?;
    let quick = create_quick(app, root)?;
    let main_hwnd = main.hwnd()?.0 as isize;
    let quick_hwnd = quick.hwnd()?.0 as isize;
    state.main.store(main_hwnd, Ordering::Release);
    state.quick.store(quick_hwnd, Ordering::Release);
    platform::prepare_quick(quick_hwnd)?;
    platform::prepare_main(main_hwnd)?;
    let startup_error = state
        .startup_error
        .lock()
        .map_err(|_| anyhow!("启动错误状态不可用"))?
        .is_some();
    if startup_error {
        main.show()?;
    }
    Ok(())
}

fn create_main(app: &tauri::App, root: &Path) -> Result<WebviewWindow> {
    Ok(
        WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
            .title("片筏剪贴板")
            .icon(crate::tray_status::icon()?)?
            .decorations(false)
            .inner_size(1000.0, 700.0)
            .visible(false)
            .focused(false)
            .data_directory(root.to_path_buf())
            .build()?,
    )
}

fn create_preview(app: &tauri::App, main: &WebviewWindow, root: &Path) -> Result<()> {
    let preview = WebviewWindowBuilder::new(
        app,
        "preview",
        WebviewUrl::App("index.html?panel=preview".into()),
    )
    .title("片筏剪贴板 · 内容预览")
    // 原生阴影会增加白边和客户区偏移，沿用透明客户区内的边框/阴影。
    .shadow(false)
    .transparent(true)
    .owner(main)?
    .inner_size(360.0, 420.0)
    .visible(false)
    .focused(false)
    .focusable(false)
    .decorations(false)
    .skip_taskbar(true)
    .resizable(false)
    .data_directory(root.to_path_buf())
    .build()?;
    platform::prepare_preview(preview.hwnd()?.0 as isize)?;
    for event in ["main-hidden", "history-changed"] {
        let handle = app.handle().clone();
        app.listen_any(event, move |_| {
            compact_preview::hide(&handle);
        });
    }
    Ok(())
}

fn create_quick(app: &tauri::App, root: &Path) -> Result<WebviewWindow> {
    let (width, height) = crate::quick_geometry::dimensions(5)?;
    let quick = WebviewWindowBuilder::new(
        app,
        "quick",
        WebviewUrl::App("index.html?panel=quick".into()),
    )
    .title("快速粘贴")
    .inner_size(width, height)
    .transparent(true)
    .shadow(false)
    .visible(false)
    .focused(false)
    .focusable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .data_directory(root.to_path_buf())
    .build()?;
    // 忽略鼠标命中，避免 WebView 子窗口激活。
    quick.set_ignore_cursor_events(true)?;
    Ok(quick)
}
