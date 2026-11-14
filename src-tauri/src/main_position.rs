//! 主窗口只保留记忆位置与鼠标定位；输入位置获取已移除。
use crate::{platform, settings::MainPosition};
use tauri::WebviewWindow;
use windows::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST},
    UI::{
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        WindowsAndMessaging::*,
    },
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
#[derive(Clone, Copy)]
pub struct Location {
    anchor: Rect,
    work: Rect,
    scale: f64,
    source: &'static str,
}
pub fn capture(mode: MainPosition, _target: isize) -> Option<Location> {
    if mode == MainPosition::Remember {
        return None;
    }
    let mut mouse = POINT::default();
    unsafe {
        GetCursorPos(&mut mouse).ok()?;
    }
    let anchor = Rect {
        left: mouse.x,
        top: mouse.y,
        right: mouse.x + 1,
        bottom: mouse.y + 1,
    };
    let source = "mouse";
    unsafe {
        let point = POINT {
            x: anchor.left + (anchor.right - anchor.left) / 2,
            y: anchor.top + (anchor.bottom - anchor.top) / 2,
        };
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let (mut dx, mut dy) = (96, 96);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        Some(Location {
            anchor,
            work: Rect {
                left: info.rcWork.left,
                top: info.rcWork.top,
                right: info.rcWork.right,
                bottom: info.rcWork.bottom,
            },
            scale: (f64::from(dx) / 96.0).clamp(0.5, 4.0),
            source,
        })
    }
}
/// 均为物理像素。优先下方，再上方/右侧/左侧，空间不足时约束在工作区。
pub fn arrange(anchor: Rect, width: u32, height: u32, work: Rect, gap: i32) -> Option<Rect> {
    let (aw, ah) = (
        i64::from(work.right) - i64::from(work.left),
        i64::from(work.bottom) - i64::from(work.top),
    );
    if aw <= 0 || ah <= 0 {
        return None;
    }
    let (w, h) = (i64::from(width).min(aw), i64::from(height).min(ah));
    let gap = i64::from(gap.max(0));
    let l = i64::from(work.left);
    let t = i64::from(work.top);
    let r = i64::from(work.right);
    let b = i64::from(work.bottom);
    let candidates = [
        (i64::from(anchor.left), i64::from(anchor.bottom) + gap),
        (i64::from(anchor.left), i64::from(anchor.top) - gap - h),
        (i64::from(anchor.right) - w, i64::from(anchor.top) - gap - h),
        (i64::from(anchor.right) - w, i64::from(anchor.bottom) + gap),
        (i64::from(anchor.right) + gap, i64::from(anchor.top)),
        (i64::from(anchor.left) - gap - w, i64::from(anchor.top)),
    ];
    let (x, y) = candidates
        .iter()
        .copied()
        .find(|(x, y)| *x >= l && *y >= t && *x + w <= r && *y + h <= b)
        .unwrap_or(candidates[0]);
    let x = x.clamp(l, r - w);
    let y = y.clamp(t, b - h);
    Some(Rect {
        left: x as i32,
        top: y as i32,
        right: (x + w) as i32,
        bottom: (y + h) as i32,
    })
}
pub fn apply(window: &WebviewWindow, location: Location) -> tauri::Result<()> {
    if window.is_maximized()? {
        window.unmaximize()?;
    }
    let old_scale = window.scale_factor()?;
    let current = window.outer_size()?;
    let ratio = location.scale / old_scale;
    let size = (
        (f64::from(current.width) * ratio).round() as u32,
        (f64::from(current.height) * ratio).round() as u32,
    );
    let gap = (12.0 * location.scale).round() as i32;
    let Some(rect) = arrange(location.anchor, size.0, size.1, location.work, gap) else {
        return Ok(());
    };
    unsafe {
        SetWindowPos(
            platform::hwnd(window.hwnd()?.0 as isize),
            None,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
        )
        .map_err(anyhow::Error::from)?;
        // WM_DPICHANGED可能按系统建议再次缩放；用已计算的目标尺寸做一次最终约束。
        let actual = window.outer_size()?;
        if let Some(final_rect) = arrange(location.anchor, size.0, size.1, location.work, gap) {
            let pos = window.outer_position()?;
            if pos.x != final_rect.left
                || pos.y != final_rect.top
                || actual.width != (final_rect.right - final_rect.left) as u32
                || actual.height != (final_rect.bottom - final_rect.top) as u32
            {
                SetWindowPos(
                    platform::hwnd(window.hwnd()?.0 as isize),
                    None,
                    final_rect.left,
                    final_rect.top,
                    final_rect.right - final_rect.left,
                    final_rect.bottom - final_rect.top,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
                )
                .map_err(anyhow::Error::from)?;
            }
        }
    }
    crate::diag::write(format!(
        "main_position source={} anchor=({},{}), scale={}",
        location.source, location.anchor.left, location.anchor.top, location.scale
    ));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    const WORK: Rect = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };
    #[test]
    fn prefers_below_then_flips_above_and_stays_on_negative_monitor() {
        let anchor = Rect {
            left: 100,
            top: 200,
            right: 102,
            bottom: 220,
        };
        assert_eq!(arrange(anchor, 390, 700, WORK, 12).unwrap().top, 232);
        let anchor = Rect {
            top: 950,
            bottom: 970,
            ..anchor
        };
        assert_eq!(arrange(anchor, 390, 700, WORK, 12).unwrap().top, 238);
        let work = Rect {
            left: -1920,
            top: -200,
            right: 0,
            bottom: 880,
        };
        let anchor = Rect {
            left: -20,
            top: 800,
            right: -19,
            bottom: 820,
        };
        let r = arrange(anchor, 585, 900, work, 18).unwrap();
        assert!(
            r.left >= work.left
                && r.right <= work.right
                && r.top >= work.top
                && r.bottom <= work.bottom
        );
    }
    #[test]
    fn bottom_right_anchor_flips_both_axes_and_avoids_the_input() {
        let a = Rect {
            left: 1900,
            top: 1000,
            right: 1901,
            bottom: 1020,
        };
        let r = arrange(a, 1000, 700, WORK, 12).unwrap();
        assert_eq!(r.right, a.right);
        assert!(r.bottom <= a.top - 12);
        assert!(r.left >= WORK.left);
    }
    #[test]
    fn oversize_windows_are_capped_and_empty_work_area_is_rejected() {
        let anchor = Rect {
            left: 1900,
            top: 1030,
            right: 1901,
            bottom: 1031,
        };
        assert_eq!(arrange(anchor, 5000, 4000, WORK, 24), Some(WORK));
        assert!(arrange(anchor, 390, 700, Rect { right: 0, ..WORK }, 12).is_none());
    }
}
