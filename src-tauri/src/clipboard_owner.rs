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
pub(crate) fn clipboard_owner(report: impl FnMut(String)) -> ClipboardOwner {
    window_process(unsafe { GetClipboardOwner() }.unwrap_or_default(), report)
}

/// 查询失败仍保留 Unknown；释放失败只报告，不改变来源选择。
pub(crate) fn window_process(hwnd: HWND, mut report: impl FnMut(String)) -> ClipboardOwner {
    let mut owner = ClipboardOwner::absent();
    if hwnd.is_invalid() {
        return owner;
    }
    owner.window = Some(hwnd.0 as isize);
    owner.status = OwnerStatus::Unknown;
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 {
        return owner;
    }
    owner.pid = Some(pid);
    query_process(pid, &mut owner, &mut report);
    owner
}

fn query_process(pid: u32, owner: &mut ClipboardOwner, report: &mut impl FnMut(String)) {
    let process = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(process) => process,
        Err(error) => {
            owner.status = open_failure_status(error.code());
            return;
        }
    };
    let mut exit_code = 0;
    if unsafe { GetExitCodeProcess(process, &mut exit_code) }.is_ok() {
        owner.status = if exit_code == PROCESS_STILL_ACTIVE {
            OwnerStatus::Live
        } else {
            OwnerStatus::Exited
        };
    }
    let mut buffer = vec![0u16; PROCESS_IMAGE_CAPACITY];
    let mut len = buffer.len() as u32;
    let queried = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    };
    if let Err(error) = unsafe { CloseHandle(process) } {
        report(format!("clipboard owner process release failed: {error}"));
    }
    if queried.is_ok() {
        owner.exe =
            exe_filename(Some(&String::from_utf16_lossy(&buffer[..len as usize]))).to_owned();
    }
}

const PROCESS_STILL_ACTIVE: u32 = 259;
const PROCESS_IMAGE_CAPACITY: usize = 32768;
fn open_failure_status(code: windows::core::HRESULT) -> OwnerStatus {
    if code == windows::Win32::Foundation::ERROR_INVALID_PARAMETER.to_hresult() {
        return OwnerStatus::Exited;
    }
    OwnerStatus::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn access_denied_is_unknown_and_missing_process_is_exited() {
        use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER};
        assert_eq!(
            open_failure_status(ERROR_ACCESS_DENIED.to_hresult()),
            OwnerStatus::Unknown
        );
        assert_eq!(
            open_failure_status(ERROR_INVALID_PARAMETER.to_hresult()),
            OwnerStatus::Exited
        );
    }
    #[test]
    fn absent_window_never_reports_a_release_failure() {
        let owner = window_process(HWND::default(), |_| panic!("no process acquired"));
        assert_eq!(owner.status, OwnerStatus::Absent);
        assert_eq!(owner.pid, None);
    }
}
