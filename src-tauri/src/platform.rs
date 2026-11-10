use anyhow::{anyhow, bail, Result};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use windows::{
    core::w,
    Win32::{
        Foundation::{LocalFree, HLOCAL, HWND, POINT},
        Graphics::Gdi::*,
        Security::Cryptography::*,
        UI::WindowsAndMessaging::*,
    },
};
use zeroize::{Zeroize, Zeroizing};

// 前台跟踪与粘贴目标校验共用同一规则，菜单窗口也不能成为目标。
const SHELL_WINDOW_CLASSES: &[&str] = &[
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Progman",
    "WorkerW",
    "#32768",
];
pub fn is_shell_window_class(name: &str) -> bool {
    SHELL_WINDOW_CLASSES.contains(&name)
}
#[derive(Debug, PartialEq, Eq)]
pub enum DataError {
    DpapiDecrypt,
    KeyLength,
    DatabaseKey,
}
impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DpapiDecrypt => "DPAPI 解密 key.bin 失败，可能是更换了 Windows 账户或电脑，也可能是密钥文件损坏。请勿删除数据，请保留完整数据目录并尝试在原账户、原电脑上恢复。",
            Self::KeyLength => "key.bin 长度异常（文件为空或解密后的主密钥不是 32 字节）。请勿删除数据或重新生成密钥，请保留完整数据目录并检查备份。",
            Self::DatabaseKey => "数据库密钥不匹配，或数据库文件已损坏，无法解密历史记录。请勿删除数据，请保留 key.bin、history.db 及其配套文件，并检查是否来自同一份备份。",
        })
    }
}
impl std::error::Error for DataError {}

pub fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}
pub fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}
pub fn valid_external(raw: isize) -> bool {
    unsafe {
        if raw == 0 || !IsWindow(Some(hwnd(raw))).as_bool() {
            return false;
        }
        let mut process = 0;
        GetWindowThreadProcessId(hwnd(raw), Some(&mut process));
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd(raw), &mut class);
        let name = String::from_utf16_lossy(&class[..len.max(0) as usize]);
        process != std::process::id() && !is_shell_window_class(&name)
    }
}
pub fn data_root() -> Result<PathBuf> {
    // 测试时显式指定隔离目录，正常运行才使用 APPDATA。
    if let Some(p) = std::env::var_os("CLIPBOARD_M0_DATA_DIR") {
        return Ok(PathBuf::from(p));
    }
    Ok(
        PathBuf::from(std::env::var_os("APPDATA").ok_or_else(|| anyhow!("APPDATA 不可用"))?)
            .join("ClipboardM0"),
    )
}
fn protect(input: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    unsafe {
        let data = CRYPT_INTEGER_BLOB {
            cbData: input.len() as u32,
            pbData: input.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB::default();
        if encrypt {
            CryptProtectData(
                &data,
                w!("ClipboardM0"),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )?;
        } else {
            CryptUnprotectData(
                &data,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )?;
        }
        let slice = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
        let bytes = slice.to_vec();
        slice.zeroize();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        Ok(bytes)
    }
}
pub fn master_key(root: &Path) -> Result<Zeroizing<[u8; 32]>> {
    fs::create_dir_all(root)?;
    let path = root.join("key.bin");
    if path.exists() {
        let encrypted = fs::read(path)?;
        if encrypted.is_empty() {
            return Err(DataError::KeyLength.into());
        }
        let decrypted =
            Zeroizing::new(protect(&encrypted, false).map_err(|_| DataError::DpapiDecrypt)?);
        if decrypted.len() != 32 {
            return Err(DataError::KeyLength.into());
        }
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&decrypted);
        return Ok(key);
    }
    if root.join("history.db").exists() {
        bail!("数据库已存在但密钥缺失，拒绝生成新密钥");
    }
    let key = crate::storage::crypto::random_key()?;
    let protected = protect(key.as_ref(), true)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&protected)?;
    file.sync_all()?;
    Ok(key)
}
pub fn prepare_quick(raw: isize) -> Result<()> {
    unsafe {
        let h = hwnd(raw);
        let style = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            h,
            GWL_EXSTYLE,
            (style | WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0) as isize,
        );
        SetWindowPos(
            h,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )?;
        Ok(())
    }
}
pub fn show_quick(raw: isize, target: isize) -> Result<()> {
    if foreground() != target {
        bail!("前台窗口已变化，取消面板");
    }
    position_quick(raw, target)
}
pub fn show_notice(raw: isize) -> Result<()> {
    // 提示不执行粘贴，不依赖原目标窗口仍在前台。
    position_quick(raw, foreground())
}
fn position_quick(raw: isize, target: isize) -> Result<()> {
    unsafe {
        let mut point = POINT::default();
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let tid = GetWindowThreadProcessId(hwnd(target), None);
        let caret = GetGUIThreadInfo(tid, &mut info).is_ok() && !info.hwndCaret.0.is_null();
        if caret {
            point = POINT {
                x: info.rcCaret.left,
                y: info.rcCaret.bottom + 8,
            };
            if !ClientToScreen(info.hwndCaret, &mut point).as_bool() {
                GetCursorPos(&mut point)?;
            }
        } else {
            GetCursorPos(&mut point)?;
            point.y += 16;
        }
        let h = hwnd(raw);
        let mut rect = Default::default();
        GetWindowRect(h, &mut rect)?;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(
            MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            let r = monitor.rcWork;
            point.x = point.x.clamp(r.left, (r.right - width).max(r.left));
            point.y = point.y.clamp(r.top, (r.bottom - height).max(r.top));
        }
        SetWindowPos(
            h,
            Some(HWND_TOPMOST),
            point.x,
            point.y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )?;
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        Ok(())
    }
}
pub fn show_startup_error(message: &str) {
    let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            windows::core::PCWSTR(wide.as_ptr()),
            w!("剪贴板 M0 启动失败"),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn hide(raw: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(raw), SW_HIDE);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dpapi_roundtrip() {
        let c = protect(&[53; 32], true).unwrap();
        assert_ne!(c, vec![53; 32]);
        assert_eq!(protect(&c, false).unwrap(), vec![53; 32]);
    }

    fn isolated_key_root(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!("key-{name}-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn dpapi_failure_has_actionable_message_and_preserves_file() {
        let root = isolated_key_root("dpapi");
        let bytes = b"synthetic-invalid-dpapi";
        fs::write(root.join("key.bin"), bytes).unwrap();
        let error = master_key(&root).unwrap_err();
        assert_eq!(
            error.downcast_ref::<DataError>(),
            Some(&DataError::DpapiDecrypt)
        );
        assert!(error.to_string().contains("Windows 账户或电脑"));
        assert!(error.to_string().contains("请勿删除数据"));
        assert_eq!(fs::read(root.join("key.bin")).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_key_length_is_distinct_and_preserves_file() {
        let root = isolated_key_root("length");
        for bytes in [
            Vec::new(),
            protect(&[7; 31], true).unwrap(),
            protect(&[7; 33], true).unwrap(),
        ] {
            fs::write(root.join("key.bin"), &bytes).unwrap();
            let error = master_key(&root).unwrap_err();
            assert_eq!(
                error.downcast_ref::<DataError>(),
                Some(&DataError::KeyLength)
            );
            assert!(error.to_string().contains("key.bin 长度异常"));
            assert!(error.to_string().contains("请勿删除数据"));
            assert_eq!(fs::read(root.join("key.bin")).unwrap(), bytes);
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_key_refuses_to_create_when_database_exists() {
        let root = isolated_key_root("nokey");
        let database = b"synthetic-history-database".to_vec();
        fs::write(root.join("history.db"), &database).unwrap();
        let error = master_key(&root).unwrap_err();
        assert!(error.to_string().contains("拒绝生成新密钥"));
        assert!(!root.join("key.bin").exists(), "存在数据库时不得生成新密钥");
        assert_eq!(fs::read(root.join("history.db")).unwrap(), database);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fresh_root_generates_stable_key_and_leaves_database_absent() {
        let root = isolated_key_root("fresh");
        let first = master_key(&root).unwrap();
        assert!(root.join("key.bin").exists());
        let stored = fs::read(root.join("key.bin")).unwrap();
        assert!(!stored.is_empty());
        assert!(!root.join("history.db").exists(), "生成密钥不得创建数据库");
        let second = master_key(&root).unwrap();
        assert_eq!(*first, *second, "重复读取必须得到同一主密钥");
        assert_eq!(
            fs::read(root.join("key.bin")).unwrap(),
            stored,
            "已有密钥不得被改写"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn key_failures_leave_existing_database_untouched() {
        let root = isolated_key_root("preserve-db");
        let database = b"synthetic-history-database-bytes".to_vec();
        fs::write(root.join("history.db"), &database).unwrap();
        for bytes in [
            b"synthetic-invalid-dpapi".to_vec(),
            protect(&[7; 31], true).unwrap(),
        ] {
            fs::write(root.join("key.bin"), &bytes).unwrap();
            let error = master_key(&root).unwrap_err();
            // 两类失败都必须保留 key.bin 与 history.db 原样，不得删除或重建。
            assert!(error.downcast_ref::<DataError>().is_some());
            assert_eq!(fs::read(root.join("key.bin")).unwrap(), bytes);
            assert_eq!(
                fs::read(root.join("history.db")).unwrap(),
                database,
                "密钥读取失败时不得改动数据库"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn shell_blacklist_includes_popup_menu_and_all_shell_classes() {
        for name in [
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
            "Progman",
            "WorkerW",
            "#32768",
        ] {
            assert!(is_shell_window_class(name));
        }
        assert!(!is_shell_window_class("Notepad"));
    }
}
