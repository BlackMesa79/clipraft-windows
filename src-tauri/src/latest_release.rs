//! 公开发布页回退，从 GitHub 固定地址读取标签及更新内容。
use anyhow::{bail, ensure, Context, Result};
use std::{
    ffi::c_void,
    mem::size_of,
    ptr::NonNull,
    time::{Duration, Instant},
};
use tauri::WebviewWindow;
use windows::{
    core::{w, Error, PCWSTR},
    Win32::Networking::WinHttp::*,
};

pub(crate) const PAGE: &str = "https://github.com/BlackMesa79/clipraft-windows/releases/latest";
pub(crate) const HISTORY_PAGE: &str = "https://github.com/BlackMesa79/clipraft-windows/releases";
const LATEST_PATH: &str = "/BlackMesa79/clipraft-windows/releases/latest";
const TAG_PATH: &str = "/BlackMesa79/clipraft-windows/releases/tag/";
const REQUEST_TIMEOUT_MS: i32 = 3_000;
const LOCATION_CAPACITY: usize = 512;
const READ_TIMEOUT: Duration = Duration::from_secs(6);
const READ_CHUNK: usize = 16 * 1024;
const MAX_PAGE_BYTES: usize = 1024 * 1024;

mod history;
mod notes;

struct HttpHandle(NonNull<c_void>);

impl HttpHandle {
    fn new(raw: *mut c_void) -> windows::core::Result<Self> {
        NonNull::new(raw).map(Self).ok_or_else(Error::from_win32)
    }

    fn raw(&self) -> *mut c_void {
        self.0.as_ptr()
    }
}

impl Drop for HttpHandle {
    fn drop(&mut self) {
        if let Err(error) = unsafe { WinHttpCloseHandle(self.raw()) } {
            crate::diag::write(format!("release HTTP handle close failed: {error}"));
        }
    }
}

fn tag_from_location(location: &str) -> Option<&str> {
    let path = location
        .strip_prefix("https://github.com")
        .unwrap_or(location);
    let tag = path.strip_prefix(TAG_PATH)?;
    valid_tag(tag).then_some(tag)
}

fn valid_tag(tag: &str) -> bool {
    if tag.len() > 64 {
        return false;
    }
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn response_status(request: &HttpHandle) -> Result<u32> {
    let mut code = 0u32;
    let mut length = size_of::<u32>() as u32;
    unsafe {
        WinHttpQueryHeaders(
            request.raw(),
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut code as *mut u32).cast()),
            &mut length,
            std::ptr::null_mut(),
        )?;
    }
    Ok(code)
}

fn redirect_tag(request: &HttpHandle) -> Result<String> {
    let mut location = [0u16; LOCATION_CAPACITY];
    let mut length = size_of::<[u16; LOCATION_CAPACITY]>() as u32;
    unsafe {
        WinHttpQueryHeaders(
            request.raw(),
            WINHTTP_QUERY_LOCATION,
            PCWSTR::null(),
            Some(location.as_mut_ptr().cast()),
            &mut length,
            std::ptr::null_mut(),
        )?;
    }
    let location = String::from_utf16(&location[..length as usize / size_of::<u16>()])?;
    tag_from_location(location.trim_end_matches('\0'))
        .map(str::to_owned)
        .context("GitHub 发布页返回了无法识别的标签地址")
}

fn open_request(connection: &HttpHandle, method: PCWSTR, path: &str) -> Result<HttpHandle> {
    let path: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let request = HttpHandle::new(unsafe {
        WinHttpOpenRequest(
            connection.raw(),
            method,
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        )
    })?;
    // 只读取当前响应，避免向重定向地址发出请求。
    unsafe {
        WinHttpSetOption(
            Some(request.raw()),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes()),
        )?;
    }
    Ok(request)
}

fn request<T>(
    method: PCWSTR,
    path: &str,
    read: impl FnOnce(&HttpHandle) -> Result<T>,
) -> Result<T> {
    let session = HttpHandle::new(unsafe {
        WinHttpOpen(
            w!("ClipRaft"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )
    })?;
    let timeout = REQUEST_TIMEOUT_MS;
    unsafe { WinHttpSetTimeouts(session.raw(), timeout, timeout, timeout, timeout)? };
    let connection =
        HttpHandle::new(unsafe { WinHttpConnect(session.raw(), w!("github.com"), 443, 0) })?;
    let request = open_request(&connection, method, path)?;
    unsafe {
        WinHttpSendRequest(request.raw(), None, None, 0, 0, 0)?;
        WinHttpReceiveResponse(request.raw(), std::ptr::null_mut())?;
    }
    read(&request)
}

fn query_latest_tag() -> Result<Option<String>> {
    request(w!("HEAD"), LATEST_PATH, |request| {
        match response_status(request)? {
            301 | 302 | 303 | 307 | 308 => redirect_tag(request).map(Some),
            404 => Ok(None),
            status => bail!("GitHub 发布页返回 HTTP {status}"),
        }
    })
}

fn read_html(request: &HttpHandle) -> Result<String> {
    let started = Instant::now();
    let mut body = Vec::new();
    let mut chunk = [0u8; READ_CHUNK];
    loop {
        ensure!(started.elapsed() < READ_TIMEOUT, "读取 GitHub 发布页超时");
        let mut read = 0u32;
        unsafe {
            WinHttpReadData(
                request.raw(),
                chunk.as_mut_ptr().cast(),
                READ_CHUNK as u32,
                &mut read,
            )?;
        }
        if read == 0 {
            return String::from_utf8(body).context("GitHub 发布页编码无效");
        }
        let read = read as usize;
        ensure!(
            body.len() + read <= MAX_PAGE_BYTES,
            "GitHub 发布页超出读取上限"
        );
        body.extend_from_slice(&chunk[..read]);
    }
}

fn read_release_html(path: &str) -> Result<String> {
    request(w!("GET"), path, |request| {
        let status = response_status(request)?;
        ensure!(status == 200, "GitHub 发布页返回 HTTP {status}");
        read_html(request)
    })
}

fn query_update_notes(tag: &str) -> Result<String> {
    ensure!(valid_tag(tag), "正式版本标签格式无效");
    let html = read_release_html(&format!("{TAG_PATH}{tag}"))?;
    Ok(notes::read(&html).unwrap_or_default())
}

#[tauri::command]
pub async fn get_latest_release_tag(window: WebviewWindow) -> Result<Option<String>, String> {
    if window.label() != "main" {
        return Err("仅主窗口可检查更新".into());
    }
    tauri::async_runtime::spawn_blocking(query_latest_tag)
        .await
        .map_err(|error| format!("检查更新任务退出：{error}"))?
        .map_err(|error| {
            crate::diag::write(format!("release page lookup failed: {error:#}"));
            "无法访问 GitHub 发布页，请检查网络后重试。".into()
        })
}

#[tauri::command]
pub async fn get_release_update_notes(
    tag: String,
    window: WebviewWindow,
) -> Result<String, String> {
    if window.label() != "main" {
        return Err("仅主窗口可读取更新内容".into());
    }
    tauri::async_runtime::spawn_blocking(move || query_update_notes(&tag))
        .await
        .map_err(|error| format!("读取更新内容任务退出：{error}"))?
        .map_err(|error| {
            crate::diag::write(format!("release notes lookup failed: {error:#}"));
            "无法读取更新内容".into()
        })
}

#[tauri::command]
pub async fn list_release_notes(window: WebviewWindow) -> Result<history::ReleaseHistory, String> {
    if window.label() != "main" {
        return Err("仅主窗口可读取发布历史".into());
    }
    tauri::async_runtime::spawn_blocking(history::query)
        .await
        .map_err(|error| format!("读取发布历史任务退出：{error}"))?
        .map_err(|error| {
            crate::diag::write(format!("release history lookup failed: {error:#}"));
            "无法读取发布历史".into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepts_tags_for_the_fixed_repository() {
        assert_eq!(
            tag_from_location(
                "https://github.com/BlackMesa79/clipraft-windows/releases/tag/v0.8.0"
            ),
            Some("v0.8.0"),
        );
        for location in [
            "https://example.com/BlackMesa79/clipraft-windows/releases/tag/v0.8.0",
            "https://github.com/another/clipraft-windows/releases/tag/v0.8.0",
            "/BlackMesa79/clipraft-windows/releases/tag/",
            "/BlackMesa79/clipraft-windows/releases/tag/v0.8.0/other",
        ] {
            assert_eq!(tag_from_location(location), None, "{location}");
        }
    }

    #[test]
    #[ignore = "独立联网探测 GitHub 的公开发布页"]
    fn live_public_release_redirect() {
        let tag = query_latest_tag().unwrap().expect("公开发布页应返回标签");
        let summary = query_update_notes(&tag).expect("公开发布页应能读取更新内容");
        assert!(!summary.is_empty());
        assert!(!summary.contains("下载与运行"));
        println!(
            "public release tag: {tag}; summary lines: {}",
            summary.lines().count()
        );
    }
}
