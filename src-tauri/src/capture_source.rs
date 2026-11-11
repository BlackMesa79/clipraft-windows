//! 来源与忽略规则；只读取窗口/进程元数据，不访问剪贴板载荷。
use crate::clipboard_owner::{ClipboardOwner, OwnerStatus};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CaptureSource {
    pub exe: Option<String>,
    pub title: Option<String>,
}

pub(crate) fn ignored(exe: Option<&str>, apps: &[String]) -> bool {
    let Some(name) = exe.and_then(|s| crate::settings::normalize_ignored_app(s).ok().flatten())
    else {
        return false;
    };
    apps.iter().any(|app| {
        crate::settings::normalize_ignored_app(app)
            .ok()
            .flatten()
            .is_some_and(|app| app.to_lowercase() == name.to_lowercase())
    })
}

fn usable(window: &ClipboardOwner) -> bool {
    !matches!(window.status, OwnerStatus::Absent | OwnerStatus::Exited)
        && crate::settings::normalize_ignored_app(&window.exe)
            .ok()
            .flatten()
            .is_some()
}

/// 有效 owner 始终优先；仅回退前台时排除本进程，避免把管理器误记为来源。
fn select_window<'a>(
    owner: &'a ClipboardOwner,
    foreground: &'a ClipboardOwner,
    self_pid: u32,
) -> Option<&'a ClipboardOwner> {
    if usable(owner) {
        Some(owner)
    } else if foreground.pid != Some(self_pid) && usable(foreground) {
        Some(foreground)
    } else {
        None
    }
}

pub(crate) fn truncate_title(title: &str) -> Option<String> {
    let title: String = title.chars().take(256).collect();
    (!title.trim().is_empty()).then_some(title)
}

pub(crate) fn capture(owner: &ClipboardOwner) -> CaptureSource {
    let foreground = crate::clipboard_owner::window_process(crate::platform::hwnd(
        crate::platform::foreground(),
    ));
    let Some(window) = select_window(owner, &foreground, std::process::id()) else {
        return CaptureSource::default();
    };
    let exe = crate::settings::normalize_ignored_app(&window.exe)
        .ok()
        .flatten()
        .map(str::to_owned);
    // 固定缓冲区：最多 256 个 Unicode 字符；不向外进程发送自定义消息。
    let title = window
        .window
        .and_then(|hwnd| {
            let mut buffer = [0u16; 513];
            let len = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(
                    crate::platform::hwnd(hwnd),
                    &mut buffer,
                )
            };
            (len > 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
        })
        .and_then(|title| truncate_title(&title));
    CaptureSource { exe, title }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window(exe: &str, pid: u32, status: OwnerStatus) -> ClipboardOwner {
        ClipboardOwner {
            window: Some(pid as isize),
            pid: Some(pid),
            exe: exe.into(),
            status,
        }
    }
    #[test]
    fn ignored_case_insensitive() {
        assert!(ignored(Some("KEEPASSXC.EXE"), &["KeePassXC.exe".into()]));
    }
    #[test]
    fn ignored_paths_on_both_sides() {
        assert!(ignored(
            Some(r"C:\Tools\Bitwarden.exe"),
            &["D:/apps/BITWARDEN.exe".into()]
        ));
    }
    #[test]
    fn ignored_trims_paths_and_compares_unicode_case() {
        assert!(ignored(
            Some("  C:\\Apps\\ÄPP.EXE  "),
            &["  D:/tools/äpp.exe  ".into()]
        ));
    }
    #[test]
    fn ignored_empty_list() {
        assert!(!ignored(Some("notepad.exe"), &[]));
    }
    #[test]
    fn ignored_invalid_or_missing_exe() {
        for exe in [None, Some("-"), Some(""), Some("script.cmd"), Some("exe")] {
            assert!(!ignored(exe, &["script.cmd".into(), "notepad.exe".into()]));
        }
        assert!(!ignored(Some("other.exe"), &["notepad.exe".into()]));
    }
    #[test]
    fn owner_has_priority() {
        let owner = window("notepad.exe", 2, OwnerStatus::Live);
        let fg = window("browser.exe", 3, OwnerStatus::Live);
        assert_eq!(select_window(&owner, &fg, 1).unwrap().exe, "notepad.exe");
    }
    #[test]
    fn absent_owner_uses_foreground() {
        let fg = window("browser.exe", 3, OwnerStatus::Live);
        assert_eq!(
            select_window(&ClipboardOwner::absent(), &fg, 1)
                .unwrap()
                .exe,
            "browser.exe"
        );
    }
    #[test]
    fn exited_or_unqueryable_owner_uses_foreground() {
        let fg = window("KeePassXC.exe", 3, OwnerStatus::Live);
        for owner in [
            window("old.exe", 2, OwnerStatus::Exited),
            window("-", 2, OwnerStatus::Unknown),
            window("-", 2, OwnerStatus::Live),
        ] {
            let chosen = select_window(&owner, &fg, 1).unwrap();
            assert!(ignored(Some(&chosen.exe), &["keepassxc.exe".into()]));
        }
    }
    #[test]
    fn foreground_self_is_not_a_source() {
        assert!(select_window(
            &ClipboardOwner::absent(),
            &window("clipboard.exe", 1, OwnerStatus::Live),
            1
        )
        .is_none());
    }
    #[test]
    fn missing_both_is_null() {
        assert!(select_window(&ClipboardOwner::absent(), &ClipboardOwner::absent(), 1).is_none());
    }
    #[test]
    fn queried_exe_with_unknown_exit_status_is_usable() {
        let owner = window("app.exe", 2, OwnerStatus::Unknown);
        assert_eq!(
            select_window(&owner, &ClipboardOwner::absent(), 1)
                .unwrap()
                .exe,
            "app.exe"
        );
    }
    #[test]
    fn title_unicode_limit() {
        let s = "😀中".repeat(200);
        let out = truncate_title(&s).unwrap();
        assert_eq!(out.chars().count(), 256);
        assert!(s.starts_with(&out));
    }
    #[test]
    fn title_empty_is_null_and_short_unchanged() {
        assert_eq!(truncate_title(" 	"), None);
        assert_eq!(truncate_title("笔记 😀"), Some("笔记 😀".into()));
    }
}
