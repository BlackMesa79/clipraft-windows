use anyhow::Result;
use std::path::PathBuf;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::ERROR_CANCELLED,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        UI::{
            Shell::{
                CLSID_ProgressDialog, FileOpenDialog, IFileOpenDialog, IProgressDialog,
                FILEOPENDIALOGOPTIONS, FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
                FOS_NOCHANGEDIR, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, PROGDLG_NOCANCEL,
                SIGDN_FILESYSPATH,
            },
            WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONWARNING, MB_YESNO},
        },
    },
};

struct Com;
impl Com {
    fn start() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
pub(super) fn select_destination(owner: Option<isize>) -> Result<Option<PathBuf>> {
    select_folder(owner, "选择片筏数据目录（新目录或空目录）")
}
pub(super) fn select_existing() -> Result<Option<PathBuf>> {
    select_folder(None, "选择现有片筏数据目录（包含 history.db 与 key.bin）")
}
pub(crate) fn select_archive(owner: Option<isize>) -> Result<Option<PathBuf>> {
    select_path(owner, "选择片筏密码归档", FOS_FILEMUSTEXIST)
}
pub(crate) fn select_archive_folder(owner: Option<isize>) -> Result<Option<PathBuf>> {
    select_folder(owner, "选择归档或恢复新目录的父文件夹")
}
fn select_folder(owner: Option<isize>, title: &str) -> Result<Option<PathBuf>> {
    select_path(owner, title, FOS_PICKFOLDERS)
}
fn select_path(
    owner: Option<isize>,
    title: &str,
    mode: FILEOPENDIALOGOPTIONS,
) -> Result<Option<PathBuf>> {
    let _com = Com::start()?;
    unsafe {
        let picker: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        picker.SetOptions(
            mode | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR | FOS_DONTADDTORECENT,
        )?;
        let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
        picker.SetTitle(PCWSTR(title.as_ptr()))?;
        let result = picker.Show(owner.map(crate::platform::hwnd));
        if result.as_ref().is_err_and(|error| {
            error.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0)
        }) {
            return Ok(None);
        }
        result?;
        let name = picker.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let result = name.to_string();
        CoTaskMemFree(Some(name.0.cast()));
        Ok(Some(PathBuf::from(result?)))
    }
}
pub(super) fn offer_recovery(error: &str) -> bool {
    let message = format!("{error}\n\n尚未自动新建或覆盖数据。是否重新选择一份现有数据目录？\n选择旧迁移副本会恢复到旧快照，之后的新记录不会自动合并。选择取消则保留配置并退出。");
    let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide.as_ptr()),
            w!("片筏剪贴板 · 数据目录恢复"),
            MB_YESNO | MB_ICONWARNING,
        ) == IDYES
    }
}
pub(super) struct MigrationProgress {
    dialog: Option<IProgressDialog>,
    _com: Option<Com>,
}
impl MigrationProgress {
    pub fn new() -> Result<Self> {
        if std::env::var_os("CLIPBOARD_M0_DATA_DIR").is_some()
            || std::env::var_os("CLIPBOARD_M0_DIAG").is_some()
        {
            return Ok(Self {
                dialog: None,
                _com: None,
            });
        }
        let com = Com::start()?;
        let dialog: IProgressDialog =
            unsafe { CoCreateInstance(&CLSID_ProgressDialog, None, CLSCTX_INPROC_SERVER)? };
        unsafe {
            dialog.SetTitle(w!("片筏剪贴板 · 正在迁移数据"))?;
            dialog.StartProgressDialog(
                None,
                None::<&windows::core::IUnknown>,
                PROGDLG_NOCANCEL,
                None,
            )?;
        }
        Ok(Self {
            dialog: Some(dialog),
            _com: Some(com),
        })
    }
    pub fn update(&self, step: &str, done: u64, total: u64) {
        let Some(dialog) = &self.dialog else {
            return;
        };
        let line: Vec<u16> = step.encode_utf16().chain([0]).collect();
        let result = unsafe {
            dialog
                .SetLine(1, PCWSTR(line.as_ptr()), false, None)
                .and_then(|()| dialog.SetProgress64(done, total.max(1)))
        };
        if let Err(error) = result {
            crate::diag::write(format!("migration progress update failed: {error}"));
        }
    }
}
impl Drop for MigrationProgress {
    fn drop(&mut self) {
        if let Some(dialog) = self.dialog.take() {
            if let Err(error) = unsafe { dialog.StopProgressDialog() } {
                crate::diag::write(format!("migration progress stop failed: {error}"));
            }
        }
    }
}
