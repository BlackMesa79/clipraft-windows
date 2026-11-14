use anyhow::{anyhow, bail, Result};
use std::{
    cell::Cell,
    path::PathBuf,
    time::{Duration, Instant},
};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::Gdi::*,
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
};
mod key;
pub use key::{master_key, DataError};

// 前台跟踪与粘贴目标校验共用同一规则，菜单窗口也不能成为目标。
const SHELL_WINDOW_CLASSES: &[&str] = &[
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Progman",
    "WorkerW",
    "#32768",
];
pub fn is_shell_window_class(name: &str) -> bool {
    SHELL_WINDOW_CLASSES.contains(&name)
}
pub fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}
pub fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}
pub fn main_has_foreground(raw: isize) -> bool {
    belongs_to_main(raw, foreground())
}
fn belongs_to_main(raw: isize, foreground: isize) -> bool {
    let front = hwnd(foreground);
    raw != 0
        && unsafe {
            front == hwnd(raw)
                || GetAncestor(front, GA_ROOT) == hwnd(raw)
                || GetAncestor(front, GA_ROOTOWNER) == hwnd(raw)
        }
}
pub fn should_hide_main(enabled: bool, main_in_foreground: bool) -> bool {
    enabled && !main_in_foreground
}
pub fn valid_external(raw: isize) -> bool {
    unsafe {
        if raw == 0 || !IsWindow(Some(hwnd(raw))).as_bool() {
            return false;
        }
        let mut process = 0;
        GetWindowThreadProcessId(hwnd(raw), Some(&mut process));
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd(raw), &mut class);
        let name = String::from_utf16_lossy(&class[..len.max(0) as usize]);
        process != std::process::id() && !is_shell_window_class(&name)
    }
}
pub fn data_root() -> Result<PathBuf> {
    // 测试时显式指定隔离目录，正常运行才使用 APPDATA。
    if let Some(p) = std::env::var_os("CLIPBOARD_M0_DATA_DIR") {
        return Ok(PathBuf::from(p));
    }
    Ok(
        PathBuf::from(std::env::var_os("APPDATA").ok_or_else(|| anyhow!("APPDATA 不可用"))?)
            .join("ClipboardM0"),
    )
}
// 仅主窗口 UI 线程读写；短时保护不影响 Alt+空格、鼠标菜单或正常尺寸调整。
thread_local! {
    static MAIN_MENU_SHOWN_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}
const MAIN_MENU_SUBCLASS: usize = 0x4D30;
fn suppress_alt_menu(command: usize, parameter: isize, elapsed: Option<Duration>) -> bool {
    command & 0xFFF0 == SC_KEYMENU as usize
        && parameter == 0
        && elapsed.is_some_and(|duration| duration < Duration::from_millis(300))
}
pub fn arm_main_menu_guard() {
    MAIN_MENU_SHOWN_AT.with(|shown| shown.set(Some(Instant::now())));
}
unsafe extern "system" fn main_menu_subclass(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    let elapsed = MAIN_MENU_SHOWN_AT.with(|shown| shown.get().map(|time| time.elapsed()));
    if message == WM_SYSCOMMAND
        && w.0 & 0xFFF0 == SC_KEYMENU as usize
        && l.0 == 0
        && crate::hook::recording()
    {
        return LRESULT(0);
    }
    if message == WM_SYSCOMMAND && suppress_alt_menu(w.0, l.0, elapsed) {
        crate::diag::write("toggle alt_menu_suppressed=true".into());
        return LRESULT(0);
    }
    if message == WM_NCDESTROY {
        MAIN_MENU_SHOWN_AT.with(|shown| shown.set(None));
        let _ = RemoveWindowSubclass(window, Some(main_menu_subclass), id);
    }
    DefSubclassProc(window, message, w, l)
}
pub fn prepare_main(raw: isize) -> Result<()> {
    // setup 在窗口所属线程安装；销毁时移除，并将其余消息继续交给 tao/WebView。
    if unsafe { SetWindowSubclass(hwnd(raw), Some(main_menu_subclass), MAIN_MENU_SUBCLASS, 0) }
        .as_bool()
    {
        Ok(())
    } else {
        bail!("主窗口菜单保护安装失败")
    }
}

pub fn prepare_quick(raw: isize) -> Result<()> {
    unsafe {
        let h = hwnd(raw);
        let style = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            h,
            GWL_EXSTYLE,
            (style | WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0) as isize,
        );
        SetWindowPos(
            h,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )?;
        Ok(())
    }
}
pub fn prepare_preview(raw: isize) -> Result<()> {
    unsafe {
        let h = hwnd(raw);
        let style = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            h,
            GWL_EXSTYLE,
            (style | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize,
        );
    }
    Ok(())
}
fn preview_position_flags(
    moved: bool,
    resized: bool,
    visible: bool,
) -> Option<SET_WINDOW_POS_FLAGS> {
    if !moved && !resized && visible {
        return None;
    }
    let mut flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOZORDER;
    if !moved {
        flags |= SWP_NOMOVE;
    }
    if !resized {
        flags |= SWP_NOSIZE;
    }
    if !visible {
        flags |= SWP_SHOWWINDOW;
    }
    Some(flags)
}
/// 显示与定位使用同一次不激活的原生调用；相同几何不重复触发窗口消息。
pub fn position_preview(raw: isize, x: i32, y: i32, width: u32, height: u32) -> Result<()> {
    unsafe {
        let h = hwnd(raw);
        let mut current = Default::default();
        GetWindowRect(h, &mut current)?;
        let moved = current.left != x || current.top != y;
        let resized = current.right - current.left != width as i32
            || current.bottom - current.top != height as i32;
        if let Some(flags) = preview_position_flags(moved, resized, IsWindowVisible(h).as_bool()) {
            SetWindowPos(h, None, x, y, width as i32, height as i32, flags)?;
        }
    }
    Ok(())
}
pub fn show_quick(raw: isize, target: isize) -> Result<()> {
    if foreground() != target {
        bail!("前台窗口已变化，取消面板");
    }
    position_quick(raw, target)
}
pub fn show_notice(raw: isize) -> Result<()> {
    // 提示不执行粘贴，不依赖原目标窗口仍在前台。
    position_quick(raw, foreground())
}
// 只调整现有窗口大小与屏内位置，不激活、不显示已经隐藏的窗口。
pub fn resize_quick(raw: isize, height: i32) -> Result<()> {
    unsafe {
        let h = hwnd(raw);
        let mut rect = Default::default();
        GetWindowRect(h, &mut rect)?;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mut top = rect.top;
        let mut height = height;
        if GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool() {
            let area = monitor.rcWork;
            height = height.min(area.bottom - area.top);
            top = top.clamp(area.top, (area.bottom - height).max(area.top));
        }
        SetWindowPos(
            h,
            None,
            rect.left,
            top,
            rect.right - rect.left,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
        Ok(())
    }
}
fn position_quick(raw: isize, target: isize) -> Result<()> {
    unsafe {
        let mut point = POINT::default();
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let tid = GetWindowThreadProcessId(hwnd(target), None);
        let caret = GetGUIThreadInfo(tid, &mut info).is_ok() && !info.hwndCaret.0.is_null();
        if caret {
            point = POINT {
                x: info.rcCaret.left,
                y: info.rcCaret.bottom + 8,
            };
            if !ClientToScreen(info.hwndCaret, &mut point).as_bool() {
                GetCursorPos(&mut point)?;
            }
        } else {
            GetCursorPos(&mut point)?;
            point.y += 16;
        }
        let h = hwnd(raw);
        let mut rect = Default::default();
        GetWindowRect(h, &mut rect)?;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(
            MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            let r = monitor.rcWork;
            point.x = point.x.clamp(r.left, (r.right - width).max(r.left));
            point.y = point.y.clamp(r.top, (r.bottom - height).max(r.top));
        }
        SetWindowPos(
            h,
            Some(HWND_TOPMOST),
            point.x,
            point.y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )?;
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        Ok(())
    }
}
pub fn unattended_startup(smoke: bool, data_dir: bool, diag: bool) -> bool {
    smoke || data_dir || diag
}
pub fn show_startup_error(message: &str) {
    // 使用现有 user32 导出，无新增依赖；缺失时只诊断，不退回无限等待的对话框。
    type MessageBoxTimeout = unsafe extern "system" fn(
        HWND,
        windows::core::PCWSTR,
        windows::core::PCWSTR,
        u32,
        u16,
        u32,
    ) -> i32;
    let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        let function = GetModuleHandleW(w!("user32.dll"))
            .ok()
            .and_then(|module| GetProcAddress(module, windows::core::s!("MessageBoxTimeoutW")));
        if let Some(function) = function {
            let show: MessageBoxTimeout = std::mem::transmute(function);
            show(
                HWND::default(),
                windows::core::PCWSTR(wide.as_ptr()),
                w!("剪贴板 M1 启动失败"),
                (MB_OK | MB_ICONERROR).0,
                0,
                60_000,
            );
        } else {
            crate::diag::startup_failure("MessageBoxTimeoutW 不可用，启动错误仅写日志");
        }
    }
}
pub fn hide(raw: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(raw), SW_HIDE);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_position_is_idempotent_and_never_activates_owner() {
        assert!(preview_position_flags(false, false, true).is_none());
        for moved in [false, true] {
            for resized in [false, true] {
                for visible in [false, true] {
                    if let Some(flags) = preview_position_flags(moved, resized, visible) {
                        assert!(flags.contains(SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOZORDER));
                        assert_eq!(flags.contains(SWP_SHOWWINDOW), !visible);
                        assert_eq!(flags.contains(SWP_NOMOVE), !moved);
                        assert_eq!(flags.contains(SWP_NOSIZE), !resized);
                    }
                }
            }
        }
    }
    #[test]
    fn main_foreground_includes_host_children_and_owned_windows_only() {
        // 都是本测试创建的隐藏窗口；不激活、不调用 SetFocus/SetForegroundWindow。
        struct Hidden(HWND);
        impl Drop for Hidden {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        fn hidden(parent: Option<HWND>, style: WINDOW_STYLE) -> Hidden {
            Hidden(unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("focus-policy-test"),
                    style,
                    0,
                    0,
                    0,
                    0,
                    parent,
                    None,
                    None,
                    None,
                )
                .unwrap()
            })
        }
        let main = hidden(None, WINDOW_STYLE(0));
        let external = hidden(None, WINDOW_STYLE(0));
        let child = hidden(Some(main.0), WS_CHILD);
        let owned = hidden(Some(main.0), WS_POPUP);
        let raw = main.0 .0 as isize;
        for window in [main.0, child.0, owned.0] {
            assert!(belongs_to_main(raw, window.0 as isize));
        }
        assert!(!belongs_to_main(raw, external.0 .0 as isize));
        assert!(!belongs_to_main(raw, 0));
        assert!(!belongs_to_main(0, 0));
    }
    #[test]
    fn main_blur_only_hides_when_enabled_and_foreground_has_left() {
        assert!(!should_hide_main(false, false));
        assert!(!should_hide_main(false, true));
        assert!(!should_hide_main(true, true));
        assert!(should_hide_main(true, false));
    }
    #[test]
    fn unattended_environment_never_prompts() {
        for bits in 0..8 {
            assert_eq!(
                unattended_startup(bits & 1 != 0, bits & 2 != 0, bits & 4 != 0),
                bits != 0
            );
        }
    }
    #[test]
    fn shell_blacklist_includes_popup_menu_and_all_shell_classes() {
        for name in [
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
            "Progman",
            "WorkerW",
            "#32768",
        ] {
            assert!(is_shell_window_class(name));
        }
        assert!(!is_shell_window_class("Notepad"));
    }
}

#[cfg(test)]
mod menu_guard_tests {
    use super::*;
    #[test]
    fn suppresses_only_alt_menu_during_first_300ms() {
        for ms in [0, 100, 299] {
            assert!(suppress_alt_menu(
                SC_KEYMENU as usize,
                0,
                Some(Duration::from_millis(ms))
            ));
        }
        assert!(!suppress_alt_menu(
            SC_KEYMENU as usize,
            0,
            Some(Duration::from_millis(300))
        ));
        assert!(!suppress_alt_menu(SC_KEYMENU as usize, 0, None));
    }
    #[test]
    fn preserves_alt_space_mouse_and_other_commands() {
        let elapsed = Some(Duration::from_millis(10));
        assert!(!suppress_alt_menu(SC_KEYMENU as usize, 32, elapsed));
        assert!(!suppress_alt_menu(SC_KEYMENU as usize, -1, elapsed));
        for command in [SC_MOUSEMENU, SC_SIZE, SC_MOVE, SC_CLOSE] {
            assert!(!suppress_alt_menu(command as usize, 0, elapsed));
        }
        assert!(suppress_alt_menu(SC_KEYMENU as usize | 3, 0, elapsed));
    }
}
