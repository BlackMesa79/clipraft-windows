//! 文件记录显式操作：后端按记录ID/索引取路径，磁盘及Shell调用不占存储锁。
use crate::{runtime::State, storage::Storage};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::WebviewWindow;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::{
            Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, ShellExecuteW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
};

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Open,
    Reveal,
}
fn target(storage: &Storage, id: &str, index: usize) -> Result<PathBuf, String> {
    let item = storage
        .preview_item(id)
        .map_err(|_| "文件记录不存在".to_string())?;
    if item.kind != "files" {
        return Err("仅文件记录支持此操作".into());
    }
    if item.sensitive {
        return Err("请先取消此文件记录的敏感标记".into());
    }
    let path = item.files.get(index).ok_or("文件索引无效")?;
    if path.contains('\0') || !Path::new(path).is_absolute() {
        return Err("文件路径无效".into());
    }
    Ok(PathBuf::from(path))
}
fn check_target(path: &Path) -> Result<(), String> {
    std::fs::metadata(path)
        .map(|_| ())
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => "源文件已不存在".into(),
            std::io::ErrorKind::PermissionDenied => "没有权限访问源文件".into(),
            _ => "无法访问源文件".into(),
        })
}
fn shell_action(path: &Path, action: Action) -> Result<(), String> {
    // 专用短生命线程使用STA，避免后台线程池的COM模式影响Shell扩展。
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|_| "系统文件接口初始化失败")?;
    }
    let _com = Com;
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        match action {
            Action::Open => {
                let code = ShellExecuteW(
                    None,
                    w!("open"),
                    PCWSTR(wide.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                )
                .0 as isize;
                if code > 32 {
                    return Ok(());
                }
                Err(match code {
                    2 | 3 => "源文件已不存在",
                    5 => "系统拒绝打开此文件",
                    31 => "此文件类型没有关联的默认应用",
                    _ => "系统无法打开此文件",
                }
                .into())
            }
            Action::Reveal => {
                let pidl = ILCreateFromPathW(PCWSTR(wide.as_ptr()));
                if pidl.is_null() {
                    return Err("无法解析文件的资源管理器位置".into());
                }
                // cidl=0时，完整PIDL表示需在父文件夹中选中的单个项目。
                let result = SHOpenFolderAndSelectItems(pidl, None, 0);
                ILFree(Some(pidl));
                result.map_err(|_| "资源管理器无法定位此文件".into())
            }
        }
    }
}
#[tauri::command]
pub async fn file_action(
    id: String,
    index: usize,
    action: Action,
    window: WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("仅主窗口可操作文件记录".into());
    }
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = {
            let storage = shared.storage.lock().map_err(|_| "存储不可用")?;
            target(&storage, &id, index)?
        };
        check_target(&path)?;
        std::thread::spawn(move || shell_action(&path, action))
            .join()
            .map_err(|_| "系统文件操作失败".to_string())?
    })
    .await
    .map_err(|_| "文件操作任务失败".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{encode_hdrop, Clip, FormatData};
    #[test]
    fn only_saved_file_paths_are_resolved_and_invalid_or_sensitive_requests_are_rejected() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!("file-actions-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = vec![
            root.join("中文 空格,甲.txt").to_string_lossy().into_owned(),
            root.join("图像.png").to_string_lossy().into_owned(),
        ];
        std::fs::write(&paths[0], b"synthetic").unwrap();
        let mut storage = Storage::open(&root, &[97; 32]).unwrap();
        let clip = Clip {
            formats: vec![FormatData {
                name: "CF_HDROP".into(),
                bytes: encode_hdrop(&paths),
            }],
        };
        storage.save(&clip).unwrap();
        let id = storage.list().unwrap()[0].id.clone();
        assert_eq!(target(&storage, &id, 0).unwrap(), PathBuf::from(&paths[0]));
        assert_eq!(target(&storage, &id, 1).unwrap(), PathBuf::from(&paths[1]));
        assert!(target(&storage, &id, 2).is_err());
        assert!(target(&storage, "absent", 0).is_err());
        assert!(check_target(Path::new(&paths[0])).is_ok());
        assert_eq!(
            check_target(Path::new(&paths[1])).unwrap_err(),
            "源文件已不存在"
        );
        storage
            .set_sensitive(std::slice::from_ref(&id), true, 10)
            .unwrap();
        assert!(target(&storage, &id, 0).unwrap_err().contains("敏感"));
        let text = Clip {
            formats: vec![FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: "plain"
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            }],
        };
        storage.save(&text).unwrap();
        assert!(target(&storage, &text.hash(), 0).is_err());
        drop(storage);
        std::fs::remove_dir_all(root).unwrap();
    }
}
