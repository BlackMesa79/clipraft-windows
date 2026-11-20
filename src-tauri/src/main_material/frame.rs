use anyhow::{bail, Result};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            IsZoomed, SetWindowPos, NCCALCSIZE_PARAMS, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_NCCALCSIZE, WM_NCDESTROY,
        },
    },
};

const FRAME_SUBCLASS: usize = 0x4D31;

pub(super) fn extend_client_top(window: HWND) -> Result<()> {
    if !unsafe { SetWindowSubclass(window, Some(frame_subclass), FRAME_SUBCLASS, 0) }.as_bool() {
        bail!("无法安装 Fluent 主窗边缘处理");
    }
    if let Err(error) = refresh_frame(window) {
        unsafe {
            let _ = RemoveWindowSubclass(window, Some(frame_subclass), FRAME_SUBCLASS);
        }
        return Err(error);
    }
    Ok(())
}

pub(super) fn restore(window: HWND) -> Result<()> {
    if unsafe { RemoveWindowSubclass(window, Some(frame_subclass), FRAME_SUBCLASS) }.as_bool() {
        refresh_frame(window)?;
    }
    Ok(())
}

fn refresh_frame(window: HWND) -> Result<()> {
    unsafe {
        SetWindowPos(
            window,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        )?;
    }
    Ok(())
}

unsafe extern "system" fn frame_subclass(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(window, Some(frame_subclass), id);
    }
    if message != WM_NCCALCSIZE || l.0 == 0 || IsZoomed(window).as_bool() {
        return DefSubclassProc(window, message, w, l);
    }

    let client = if w.0 == 0 {
        l.0 as *mut RECT
    } else {
        std::ptr::addr_of_mut!((*(l.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0])
    };
    let top = (*client).top;
    let result = DefSubclassProc(window, message, w, l);
    // 保留 Tao 的其余边缘和缩放行为，只让客户区覆盖顶部的白线。
    if result.0 == 0 {
        (*client).top = top;
    }
    result
}
