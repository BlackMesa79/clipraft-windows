use crate::{
    diag,
    settings::{Settings, Skin},
};
use anyhow::{Context, Result};
use std::mem::size_of;
use tauri::{WebviewWindow, Window};
use windows::Win32::{Foundation::HWND, Graphics::Dwm::*};

mod frame;
mod surface;

fn set(window: HWND, attribute: DWMWINDOWATTRIBUTE, value: i32) -> Result<()> {
    unsafe {
        DwmSetWindowAttribute(
            window,
            attribute,
            (&value as *const i32).cast(),
            size_of::<i32>() as u32,
        )?;
    }
    Ok(())
}

pub(crate) fn prepare_frame(raw: isize) {
    frame::prepare(crate::platform::hwnd(raw));
}

fn clear(window: HWND, raw: isize) {
    surface::clear(raw);
    for (attribute, value) in [
        (DWMWA_USE_HOSTBACKDROPBRUSH, 0),
        (DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE.0),
    ] {
        if let Err(error) = set(window, attribute, value) {
            diag::write(format!("main backdrop reset failed: {error:#}"));
        }
    }
}

pub fn sync_main(window: &WebviewWindow, settings: &Settings, custom_background: bool) -> bool {
    let Ok(window_handle) = window.hwnd() else {
        return false;
    };
    let raw = window_handle.0 as isize;
    let hwnd = crate::platform::hwnd(raw);
    if settings.skin == Skin::Clipraft || custom_background {
        clear(hwnd, raw);
        return false;
    }
    if surface::active(raw) {
        return surface::set_appearance(settings.theme, settings.backdrop_strength)
            .and_then(|()| surface::resize(raw))
            .is_ok();
    }

    // 透明 WebView 会遮住此窗口上的 DWM Mica，因此在其下方绘制原生模糊层。
    let _ = window_vibrancy::clear_mica(window);
    if let Err(error) = set(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE.0)
        .and_then(|()| set(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, 1))
        .context("启用 HostBackdropBrush 失败")
        .and_then(|()| surface::create(raw, settings.theme, settings.backdrop_strength))
    {
        clear(hwnd, raw);
        diag::write(format!(
            "main HostBackdrop unavailable, using solid Fluent surface: {error:#}"
        ));
        return false;
    }
    true
}

pub fn resize_main(window: &Window) {
    match window.hwnd() {
        Ok(hwnd) => {
            if let Err(error) = surface::resize(hwnd.0 as isize) {
                diag::write(format!("main backdrop resize failed: {error:#}"));
            }
        }
        Err(error) => diag::write(format!("main backdrop handle unavailable: {error}")),
    }
}
