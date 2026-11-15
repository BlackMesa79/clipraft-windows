use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use windows::{
    core::PCWSTR,
    Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH},
};

pub(super) const CONFIG_VERSION: u32 = 1;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configuration {
    pub version: u32,
    pub root: PathBuf,
    pub pending: Option<Migration>,
    #[serde(default)]
    pub previous: Option<PathBuf>,
    #[serde(default)]
    pub displaced: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Migration {
    pub source: PathBuf,
    pub target: PathBuf,
    pub id: String,
    #[serde(default)]
    pub displaced: Option<PathBuf>,
}
impl Configuration {
    pub fn initial(root: PathBuf) -> Self {
        Self {
            version: CONFIG_VERSION,
            root,
            pending: None,
            previous: None,
            displaced: None,
        }
    }
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).context("无法读取启动定位文件；不会改用默认空库")?;
        let config: Self = serde_json::from_slice(&bytes)
            .context("启动定位文件损坏，请保留原文件后恢复数据目录")?;
        if config.version != CONFIG_VERSION || !config.root.is_absolute() {
            bail!("启动定位文件的版本或数据路径无效");
        }
        if let Some(task) = &config.pending {
            if task.source != config.root
                || !task.target.is_absolute()
                || task.id.len() != 32
                || !task.id.bytes().all(|b| b.is_ascii_hexdigit())
            {
                bail!("待迁移任务无效，请保留原数据与启动定位文件");
            }
        }
        Ok(config)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }
}

pub(super) fn nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("系统随机数不可用"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("配置路径缺少父目录")?;
    let temporary = parent.join(format!(".clipraft-write-{}", nonce()?));
    let result = write_and_replace(&temporary, path, bytes);
    if result.is_err() && temporary.exists() {
        fs::remove_file(&temporary).context("写入失败，且无法清理本次配置临时文件")?;
    }
    result
}
fn write_and_replace(temporary: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    let from: Vec<u16> = temporary.as_os_str().encode_wide().chain([0]).collect();
    let to: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .context("无法原子更新启动定位文件")
}

pub(super) fn move_new(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let from: Vec<u16> = from.as_os_str().encode_wide().chain([0]).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .context("无法提交迁移文件，目标可能已被占用；不会覆盖已有文件")
}
