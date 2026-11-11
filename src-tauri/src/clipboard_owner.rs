//! 剪贴板来源元数据；无需读取载荷，也供后续来源程序记录复用。
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::{CloseHandle, HWND},
        System::{
            DataExchange::GetClipboardOwner,
            Threading::{
                GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::WindowsAndMessaging::GetWindowThreadProcessId,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerStatus {
    Absent,
    Live,
    Exited,
    Unknown,
}

#[derive(Clone, Debug)]
pub(crate) struct ClipboardOwner {
    pub window: Option<isize>,
    pub pid: Option<u32>,
    pub exe: String,
    pub status: OwnerStatus,
}
impl ClipboardOwner {
    pub fn absent() -> Self {
        Self {
            window: None,
            pid: None,
            exe: "-".into(),
            status: OwnerStatus::Absent,
        }
    }
    pub fn is_real(&self) -> bool {
        self.status == OwnerStatus::Live && self.window.is_some() && self.pid.is_some()
    }
    pub fn is_ownerless(&self) -> bool {
        matches!(self.status, OwnerStatus::Absent | OwnerStatus::Exited)
    }
    pub fn same_process_window(&self, other: &Self) -> bool {
        self.pid.is_some()
            && self.window.is_some()
            && self.pid == other.pid
            && self.window == other.window
    }
}

pub(crate) fn exe_filename(path: Option<&str>) -> &str {
    path.and_then(|path| path.rsplit(['\\', '/']).next())
        .filter(|name| !name.is_empty() && !matches!(*name, "." | ".."))
        .unwrap_or("-")
}

/// 查询失败不等于进程退出：权限不足或状态不明时保留 Unknown，禁止据此跳过。
pub(crate) fn clipboard_owner() -> ClipboardOwner {
    window_process(unsafe { GetClipboardOwner() }.unwrap_or_default())
}

/// 与 owner 使用相同查询方式，但不改变 owner 的状态与退出固化语义。
pub(crate) fn window_process(hwnd: HWND) -> ClipboardOwner {
    let mut owner = ClipboardOwner::absent();
    unsafe {
        if hwnd.is_invalid() {
            return owner;
        }
        owner.window = Some(hwnd.0 as isize);
        owner.status = OwnerStatus::Unknown;
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return owner;
        }
        owner.pid = Some(pid);
        if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut exit_code = 0;
            if GetExitCodeProcess(process, &mut exit_code).is_ok() {
                owner.status = if exit_code == 259 {
                    OwnerStatus::Live
                } else {
                    OwnerStatus::Exited
                };
            }
            let mut buffer = vec![0u16; 32768];
            let mut len = buffer.len() as u32;
            let queried = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(process);
            if queried.is_ok() {
                owner.exe = exe_filename(Some(&String::from_utf16_lossy(&buffer[..len as usize])))
                    .to_owned();
            }
        } else if windows::core::Error::from_win32().code()
            == windows::Win32::Foundation::ERROR_INVALID_PARAMETER.to_hresult()
        {
            // 非零 PID 已不存在；拒绝访问等其它失败仍是 Unknown。
            owner.status = OwnerStatus::Exited;
        }
    }
    owner
}
