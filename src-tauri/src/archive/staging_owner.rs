//! 持久化暂存所有权标记；活动判断依赖操作系统锁，不依赖PID或时间。
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

pub(super) const NAME: &str = ".clipraft-owner.json";
const VERSION: u32 = 1;
const MAX_MARKER_BYTES: u64 = 8192;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    root: PathBuf,
    kind: Kind,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Snapshot,
    Export,
    Restore,
}
impl Kind {
    pub(super) fn parse(value: &str) -> Result<Self> {
        match value {
            "snapshot" => Ok(Self::Snapshot),
            "export" => Ok(Self::Export),
            "restore" => Ok(Self::Restore),
            _ => bail!("未知归档暂存类型"),
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Self::Snapshot => ".clipraft-snapshot-",
            Self::Export => ".clipraft-export-",
            Self::Restore => ".clipraft-restore-",
        }
    }
}
pub(super) struct Owner {
    _file: File,
}
impl Owner {
    pub(super) fn create(root: &Path, kind: Kind) -> Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        let path = root.join(NAME);
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .open(&path)?;
        let result = (|| {
            file.try_lock().context("无法锁定归档暂存标记")?;
            let record = Record {
                version: VERSION,
                root: fs::canonicalize(root)?,
                kind,
            };
            let bytes = serde_json::to_vec(&record)?;
            if bytes.len() as u64 > MAX_MARKER_BYTES {
                bail!("暂存所有权标记过长");
            }
            file.write_all(&bytes)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            drop(file);
            fs::remove_file(path).context("所有权标记创建失败且无法清理")?;
            return Err(error);
        }
        Ok(Self { _file: file })
    }
    pub(super) fn claim(root: &Path) -> Result<Option<Self>> {
        use std::os::windows::fs::OpenOptionsExt;
        let path = root.join(NAME);
        let metadata = crate::data_directory::plain_metadata(&path)?;
        if !metadata.is_file() || metadata.len() > MAX_MARKER_BYTES {
            bail!("暂存标记类型或大小无效");
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_MARKER_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MARKER_BYTES {
            bail!("暂存标记超过大小上限");
        }
        let record: Record = serde_json::from_slice(&bytes).context("无法识别暂存所有权标记")?;
        validate(root, &record)?;
        Ok(Some(Self { _file: file }))
    }
}
fn validate(root: &Path, record: &Record) -> Result<()> {
    if record.version != VERSION || record.root != fs::canonicalize(root)? {
        bail!("暂存标记版本或位置不匹配");
    }
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .context("暂存目录名无效")?;
    let nonce = name
        .strip_prefix(record.kind.prefix())
        .context("暂存类型与目录名不匹配")?;
    if nonce.len() != 32
        || !nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("暂存目录标识无效");
    }
    Ok(())
}
