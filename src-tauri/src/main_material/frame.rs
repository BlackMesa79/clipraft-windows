use super::set;
use crate::diag;
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    },
};

pub(super) fn prepare(window: HWND) {
    // 保留系统合成的圆角和阴影，窗口内容绘制到四边。
    for (attribute, value) in [
        (DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE as i32),
        (DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND.0),
    ] {
        if let Err(error) = set(window, attribute, value) {
            diag::write(format!("main DWM frame attribute update failed: {error:#}"));
        }
    }
}
