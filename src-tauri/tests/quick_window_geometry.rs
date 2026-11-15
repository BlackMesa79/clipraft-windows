//! U5 原生隐藏窗口几何验证：只创建本测试窗口，不显示、不操作键鼠/剪贴板。
use clipboard_m0::platform;
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::Gdi::*,
        UI::WindowsAndMessaging::*,
    },
};

struct HiddenWindow(HWND);
impl Drop for HiddenWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0).unwrap();
        }
    }
}

#[test]
fn quick_resize_preserves_hidden_nonactivating_window_and_fits_work_area() {
    let flags = WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW;
    let window = HiddenWindow(unsafe {
        CreateWindowExW(
            flags,
            w!("STATIC"),
            w!("ClipRaft U5 hidden geometry"),
            WS_POPUP,
            20,
            20,
            420,
            408,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    });
    let raw = window.0 .0 as isize;
    platform::prepare_quick(raw).unwrap();
    for height in [60, 408, 1168, 20_000, 60, 408] {
        platform::resize_quick(raw, height).unwrap();
        let mut bounds = RECT::default();
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            GetWindowRect(window.0, &mut bounds).unwrap();
            assert!(GetMonitorInfoW(
                MonitorFromWindow(window.0, MONITOR_DEFAULTTONEAREST),
                &mut monitor
            )
            .as_bool());
            assert!(!IsWindowVisible(window.0).as_bool());
            assert_ne!(GetForegroundWindow(), window.0);
            assert_eq!(
                GetWindowLongPtrW(window.0, GWL_EXSTYLE) as u32 & flags.0,
                flags.0
            );
        }
        assert_eq!(bounds.right - bounds.left, 420);
        assert_eq!(
            bounds.bottom - bounds.top,
            height.min(monitor.rcWork.bottom - monitor.rcWork.top)
        );
        assert!(bounds.top >= monitor.rcWork.top && bounds.bottom <= monitor.rcWork.bottom);
    }
}
