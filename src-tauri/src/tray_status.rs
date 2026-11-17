//! 托盘状态渲染；不依赖运行时控制或应用事件装配。
use crate::diag;
use tauri::Manager;
pub(crate) fn icon() -> anyhow::Result<tauri::image::Image<'static>> {
    let rgba = image::load_from_memory(include_bytes!("../icons/tray.png"))?.into_rgba8();
    Ok(tauri::image::Image::new_owned(rgba.into_raw(), 32, 32))
}
pub(crate) struct PauseMenu(pub(crate) tauri::menu::MenuItem<tauri::Wry>);
pub(crate) fn paused_icon() -> anyhow::Result<tauri::image::Image<'static>> {
    let mut rgba = icon()?.rgba().to_vec();
    for y in 8..25 {
        for x in [11, 12, 13, 18, 19, 20] {
            let i = (y * 32 + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[220, 120, 20, 255]);
        }
    }
    Ok(tauri::image::Image::new_owned(rgba, 32, 32))
}
pub(crate) fn update(app: &tauri::AppHandle, paused: bool) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let icon = if paused { paused_icon() } else { icon() };
        let result = icon.and_then(|icon| tray.set_icon(Some(icon)).map_err(anyhow::Error::from));
        if let Err(error) = result {
            diag::write(format!("tray icon update failed: {error:#}"));
        }
        if let Err(error) = tray.set_tooltip(Some(if paused {
            "ClipRaft · 已暂停监听"
        } else {
            "ClipRaft · 正在监听"
        })) {
            diag::write(format!("tray tooltip update failed: {error}"));
        }
    }
    if let Some(menu) = app.try_state::<PauseMenu>() {
        if let Err(error) = menu.0.set_text(if paused {
            "恢复监听"
        } else {
            "暂停监听"
        }) {
            diag::write(format!("tray pause menu update failed: {error}"));
        }
    }
}
