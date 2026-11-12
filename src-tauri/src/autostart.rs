use anyhow::{Context, Result};
use std::os::windows::ffi::OsStrExt;
use windows::{
    core::{w, PCWSTR},
    Win32::{Foundation::ERROR_FILE_NOT_FOUND, System::Registry::*},
};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("ClipboardM0");

pub fn command(path: &std::path::Path) -> Vec<u16> {
    std::iter::once(34)
        .chain(path.as_os_str().encode_wide())
        .chain([34, 0])
        .collect()
}

pub fn set(enabled: bool) -> Result<()> {
    let executable = std::env::current_exe().context("无法取得程序路径")?;
    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()
        .context("无法打开开机自启设置")?;
        let result = if enabled {
            let bytes: Vec<u8> = command(&executable)
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect();
            RegSetValueExW(key, NAME, None, REG_SZ, Some(&bytes)).ok()
        } else {
            let result = RegDeleteValueW(key, NAME);
            if result == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                result.ok()
            }
        };
        let _ = RegCloseKey(key);
        result.context("开机自启设置失败")
    }
}

#[cfg(test)]
mod tests {
    // AC-T8-1：只验证命令编码，不调用 set，不访问真实 Run 项。
    #[test]
    fn t8_run_command_quotes_metacharacters_and_utf16_surrogate_pair() {
        let path = r"C:\测试 & tools (x64)\😀 clipboard.exe";
        let command = super::command(std::path::Path::new(path));
        let expected: Vec<u16> = format!("\"{path}\"").encode_utf16().chain([0]).collect();
        assert_eq!(command, expected);
        assert_eq!(command.iter().filter(|&&word| word == 0).count(), 1);
    }
    #[test]
    fn quoted_unicode_executable() {
        let command = super::command(std::path::Path::new(r"C:\测试 软件\clip.exe"));
        assert_eq!(
            String::from_utf16(&command[..command.len() - 1]).unwrap(),
            "\"C:\\测试 软件\\clip.exe\""
        );
        assert_eq!(command.last(), Some(&0));
    }
}
