use super::{config::Configuration, files};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};
use windows::{
    core::PCWSTR,
    Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumePathNameW},
};

const FREE_SPACE_RESERVE: u64 = 64 * 1024 * 1024;
#[derive(Clone)]
pub(crate) struct Locations {
    pub default_root: PathBuf,
    pub config: Option<PathBuf>,
    pub cache: PathBuf,
    pub legacy_config: Option<PathBuf>,
}
impl Locations {
    pub fn environment() -> Result<Self> {
        if let Some(root) = std::env::var_os("CLIPBOARD_M0_DATA_DIR") {
            let root = std::path::absolute(root)?;
            let config = std::env::var_os("CLIPRAFT_TEST_CONFIG_DIR").map(PathBuf::from);
            return Self::new(root, config);
        }
        let appdata = PathBuf::from(std::env::var_os("APPDATA").context("APPDATA 不可用")?);
        Self::installed(&appdata)
    }
    pub(super) fn installed(appdata: &Path) -> Result<Self> {
        let root = appdata.join("ClipboardM0");
        let mut locations = Self::new(root.clone(), Some(appdata.join("ClipRaft")))?;
        locations.cache = root.join("webview");
        locations.legacy_config = Some(root.join("startup.json"));
        Ok(locations)
    }
    pub fn new(default_root: PathBuf, config_base: Option<PathBuf>) -> Result<Self> {
        let cache = config_base
            .as_ref()
            .unwrap_or(&default_root)
            .join("webview");
        let config = config_base.map(|base| base.join("startup.json"));
        if !default_root.is_absolute() || config.as_ref().is_some_and(|path| !path.is_absolute()) {
            bail!("数据和启动配置目录必须是绝对路径");
        }
        Ok(Self {
            default_root,
            config,
            cache,
            legacy_config: None,
        })
    }
    pub fn read(&self) -> Result<Configuration> {
        let Some(path) = &self.config else {
            return Ok(Configuration::initial(self.default_root.clone()));
        };
        if path.try_exists()? {
            return Configuration::load(path);
        }
        if self.marker()?.try_exists()? {
            bail!("启动定位文件丢失，已停止自动创建空库。请通过恢复入口选择现有完整数据目录；不要清空任何目录。");
        }
        if let Some(legacy) = &self.legacy_config {
            if legacy.try_exists()? {
                return Configuration::load(legacy);
            }
        }
        Ok(Configuration::initial(self.default_root.clone()))
    }
    fn marker(&self) -> Result<PathBuf> {
        Ok(self
            .config
            .as_ref()
            .context("隔离环境未配置启动定位")?
            .parent()
            .context("启动定位文件没有父目录")?
            .join("initialized"))
    }
    pub fn save(&self, config: &Configuration) -> Result<()> {
        let path = self
            .config
            .as_deref()
            .context("当前隔离环境禁止更改数据目录")?;
        let marker = self.marker()?;
        if !marker.try_exists()? {
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker)?;
            file.sync_all()?;
        }
        files::plain_metadata(&marker)?;
        config.save(path)
    }
    pub(super) fn configured(&self) -> Result<bool> {
        let current = self
            .config
            .as_ref()
            .map(|path| path.try_exists())
            .transpose()?
            .unwrap_or(false);
        let legacy = self
            .legacy_config
            .as_ref()
            .map(|path| path.try_exists())
            .transpose()?
            .unwrap_or(false);
        Ok(current || legacy)
    }
}

pub(super) fn default_runtime_file(name: &str) -> bool {
    matches!(
        name,
        "startup.json" | ".clipraft-startup.lock" | ".clipraft-data.lock" | "webview" | "diag.log"
    )
}
pub(super) fn empty_target(target: &Path, locations: &Locations) -> Result<()> {
    for child in fs::read_dir(target)? {
        let child = child?;
        let name = child.file_name();
        if name == files::LOCK_NAME {
            files::plain_metadata(&child.path())?;
            continue;
        }
        // 默认目录还可能承载本程序的启动定位与缓存，不将它们当作用户历史。
        if target == fs::canonicalize(&locations.default_root)?
            && name.to_str().is_some_and(default_runtime_file)
        {
            continue;
        }
        bail!("目标目录非空，请选择新目录或空目录；请勿清空原数据目录；恢复默认位置会自动保留默认目录中的旧数据");
    }
    Ok(())
}
pub(super) fn target_path(source: &Path, target: &Path) -> Result<PathBuf> {
    if !target.is_absolute() {
        bail!("请选择绝对路径的数据目录");
    }
    let target = fs::canonicalize(target).context("目标目录不存在或无法访问")?;
    let source = fs::canonicalize(source)?;
    if target.starts_with(&source) || source.starts_with(&target) {
        bail!("源和目标不能相同，也不能互相包含");
    }
    if !files::plain_metadata(&target)?.is_dir() {
        bail!("目标不是目录");
    }
    fixed_volume(&target)?;
    Ok(target)
}
pub(crate) fn capacity(target: &Path, bytes: u64) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = target.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }?;
    let required = bytes
        .checked_add(FREE_SPACE_RESERVE.max(bytes / 20))
        .context("迁移大小溢出")?;
    if free < required {
        bail!("目标磁盘空间不足，迁移需要额外保留至少 64 MiB 空间");
    }
    Ok(())
}
fn fixed_volume(target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = target.as_os_str().encode_wide().chain([0]).collect();
    let mut volume = [0u16; 32768];
    unsafe { GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut volume) }?;
    // DRIVE_FIXED=3；首版只承诺本地固定磁盘。
    if unsafe { GetDriveTypeW(PCWSTR(volume.as_ptr())) } != 3 {
        bail!("首版数据迁移仅支持本地固定磁盘");
    }
    Ok(())
}
pub(super) fn writable(target: &Path) -> Result<()> {
    let path = target.join(format!(".clipraft-probe-{}", super::config::nonce()?));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .context("目标目录不可写")?;
    file.sync_all()?;
    drop(file);
    fs::remove_file(path).context("无法清理目录写入检查文件")
}
