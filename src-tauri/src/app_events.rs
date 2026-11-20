//! 原生应用事件边界：失败记入诊断，窗口调用在状态锁之外执行。
use crate::{
    compact_preview, diag, platform,
    runtime::{Control, State},
};
use anyhow::Result;
use std::{
    sync::{atomic::Ordering, mpsc::Sender, Arc},
    time::Duration,
};
use tauri::{Emitter, Manager};

pub(crate) fn record<T, E: std::fmt::Display>(operation: &str, result: Result<T, E>) {
    if let Err(error) = result {
        diag::write(format!("{operation}: {error}"));
    }
}

pub(crate) fn send_control(sender: &Sender<Control>, control: Control) -> Result<(), String> {
    sender.send(control).map_err(|_| "控制线程已退出".into())
}

pub(crate) fn tray_control(app: &tauri::AppHandle, control: Control) {
    record(
        "tray control failed",
        send_control(&app.state::<Arc<State>>().control, control),
    );
}

fn reposition_preview(window: &tauri::Window) -> Result<()> {
    if window.is_minimized()? {
        compact_preview::hide(window.app_handle());
        return Ok(());
    }
    if !window
        .state::<compact_preview::PreviewState>()
        .has_content()?
    {
        return Ok(());
    }
    let Some(main) = window.get_webview_window("main") else {
        return Ok(());
    };
    let main_hwnd = window.state::<Arc<State>>().main.load(Ordering::Acquire);
    if main.is_visible()? && platform::main_has_foreground(main_hwnd) {
        compact_preview::reposition(&main)?;
    }
    Ok(())
}

fn hide_main(window: &tauri::Window) -> Result<()> {
    window.hide()?;
    diag::panel(false, false);
    window.emit("main-hidden", ())?;
    Ok(())
}

fn check_blur(window: &tauri::Window) {
    let state = window.state::<Arc<State>>();
    let foreground = platform::main_has_foreground(state.main.load(Ordering::Acquire));
    if !foreground {
        compact_preview::hide(window.app_handle());
        record(
            "main deactivated event failed",
            window.emit("main-deactivated", ()),
        );
    }
    if platform::should_hide_main(state.hide_on_blur.load(Ordering::Acquire), foreground) {
        record("main blur hide failed", hide_main(window));
    }
}

fn schedule_blur_check(window: &tauri::Window) {
    let main = window.clone();
    // 等原生激活消息完成，避免把标题栏拖动或 WebView 焦点切换误判为离开应用。
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(120));
        let target = main.clone();
        record(
            "main blur dispatch failed",
            main.run_on_main_thread(move || check_blur(&target)),
        );
    });
}

pub(crate) fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" {
        return;
    }
    match event {
        tauri::WindowEvent::Focused(focused) => {
            diag::write(format!(
                "main_focus focused={} foreground_owned={}",
                focused,
                platform::main_has_foreground(
                    window.state::<Arc<State>>().main.load(Ordering::Acquire)
                )
            ));
            if *focused {
                record(
                    "main activated event failed",
                    window.emit("main-activated", ()),
                );
                return;
            }
            schedule_blur_check(window);
        }
        tauri::WindowEvent::Moved(_) => {
            record("preview reposition failed", reposition_preview(window));
        }
        tauri::WindowEvent::Resized(_) => {
            crate::main_material::resize_main(window);
            record("preview reposition failed", reposition_preview(window));
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            compact_preview::hide(window.app_handle());
            record("main close hide failed", hide_main(window));
        }
        _ => {} // Tauri 还会发送主题、缩放、销毁等无需此处理器动作的事件。
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_disconnect_is_reported_and_delivery_preserves_action() {
        let (sender, receiver) = std::sync::mpsc::channel();
        send_control(&sender, Control::Toggle).unwrap();
        assert!(matches!(receiver.recv().unwrap(), Control::Toggle));
        drop(receiver);
        assert_eq!(
            send_control(&sender, Control::Hide).unwrap_err(),
            "控制线程已退出"
        );
    }
}
