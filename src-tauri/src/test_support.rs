//! 真实剪贴板测试的隔离窗口站；不操作用户桌面、键鼠或应用窗口。
use std::ffi::c_void;

// 仅测试需要窗口站 API，保持产品依赖特性不变。
#[link(name = "user32")]
unsafe extern "system" {
    fn GetProcessWindowStation() -> *mut c_void;
    fn CreateWindowStationW(
        name: *const u16,
        flags: u32,
        access: u32,
        security: *const c_void,
    ) -> *mut c_void;
    fn SetProcessWindowStation(station: *mut c_void) -> i32;
    fn CloseWindowStation(station: *mut c_void) -> i32;
}
pub(crate) struct PrivateClipboard {
    original: *mut c_void,
    station: *mut c_void,
}
impl PrivateClipboard {
    pub(crate) fn new() -> Self {
        unsafe {
            let original = GetProcessWindowStation();
            assert!(!original.is_null());
            // 未命名窗口站独占剪贴板；失败时直接终止，不回退到用户剪贴板。
            let station = CreateWindowStationW(std::ptr::null(), 0, 0x37f, std::ptr::null());
            assert!(
                !station.is_null(),
                "创建私有窗口站失败：{}",
                std::io::Error::last_os_error()
            );
            if SetProcessWindowStation(station) == 0 {
                let error = std::io::Error::last_os_error();
                CloseWindowStation(station);
                panic!("切换私有窗口站失败：{error}");
            }
            Self { original, station }
        }
    }
}
impl Drop for PrivateClipboard {
    fn drop(&mut self) {
        unsafe {
            assert_ne!(SetProcessWindowStation(self.original), 0);
            CloseWindowStation(self.station);
        }
    }
}
