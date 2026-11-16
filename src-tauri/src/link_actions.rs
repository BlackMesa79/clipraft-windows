//! 链接操作只接受记录 ID；使用完整存储内容校验，不信任前端摘要或 URL。
use crate::{runtime::State, storage::Storage};
use std::sync::Arc;
use tauri::WebviewWindow;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
};

fn target(storage: &Storage, id: &str) -> Result<tauri::Url, String> {
    let item = storage.preview_item(id).map_err(|_| "链接记录不存在")?;
    if item.sensitive {
        return Err("请先取消此链接记录的敏感标记".into());
    }
    let clip = storage
        .load_for_paste(id)
        .map_err(|_| "记录已删除、到期或内容不可用，请刷新后重试")?;
    let text = clip.text().unwrap_or_default();
    if !crate::storage::is_link(clip.kind(), &text) {
        return Err("此记录不是可打开的 HTTP 或 HTTPS 链接".into());
    }
    tauri::Url::parse(text.trim()).map_err(|_| "链接地址无效".into())
}

struct ShellApartment;
impl Drop for ShellApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
fn launch(url: &tauri::Url) -> Result<(), String> {
    // 与文件操作一致，使用独立 STA 线程，避免线程池 COM 模式影响系统处理器。
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|_| "系统浏览器接口初始化失败")?;
    }
    let _apartment = ShellApartment;
    let wide: Vec<u16> = url.as_str().encode_utf16().chain([0]).collect();
    let code = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
        .0 as isize
    };
    launch_result(code)
}
fn launch_result(code: isize) -> Result<(), String> {
    if code > 32 {
        return Ok(());
    }
    Err(match code {
        5 => "系统拒绝打开此链接",
        31 => "未设置可用的默认浏览器，请在 Windows 设置中选择默认浏览器",
        _ => "无法启动默认浏览器，请检查系统默认应用设置后重试",
    }
    .into())
}

#[tauri::command]
pub async fn open_link(
    id: String,
    window: WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("仅主窗口可打开链接记录".into());
    }
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::Builder::new()
            .name("clipraft-open-link".into())
            .spawn(move || {
                let url = {
                    let storage = shared.storage.lock().map_err(|_| "存储不可用")?;
                    target(&storage, &id)?
                };
                launch(&url)
            })
            .map_err(|_| "浏览器操作线程启动失败")?
            .join()
            .map_err(|_| "浏览器操作失败".to_string())?
    })
    .await
    .map_err(|_| "链接操作任务失败".to_string())?
}

#[cfg(test)]
mod tests;
