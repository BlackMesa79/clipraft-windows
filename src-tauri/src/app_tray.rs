use crate::{
    app_events,
    runtime::{self, Control},
    tray_status::{icon, paused_icon, PauseMenu},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
fn menu(app: &tauri::App) -> anyhow::Result<Menu<tauri::Wry>> {
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
    Ok(Menu::with_items(app, &[&show, &pause, &settings, &quit])?)
}

fn on_menu_event(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    match event.id.as_ref() {
        "show" => app_events::tray_control(app, Control::Toggle),
        "pause" => app_events::tray_control(app, Control::TogglePause),
        "settings" => app_events::tray_control(app, Control::OpenSettings),
        "quit" => app.exit(0),
        _ => {} // 非本菜单的事件不执行动作。
    }
}

fn on_icon_event(tray: &tauri::tray::TrayIcon, event: TrayIconEvent) {
    if matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    ) {
        app_events::tray_control(tray.app_handle(), Control::Toggle);
    }
}

pub(crate) fn create(app: &tauri::App) -> anyhow::Result<()> {
    let menu = menu(app)?;
    let icon = if runtime::paused() {
        paused_icon()?
    } else {
        icon()?
    };
    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip(if runtime::paused() {
            "ClipRaft · 已暂停监听"
        } else {
            "ClipRaft · 正在监听"
        })
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(on_icon_event)
        .build(app)?;
    Ok(())
}
