//! 带文件身份与内容指纹的清理清单；通过已验证句柄删除，避免路径替换竞态。
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Component, Path, PathBuf},
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
        BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
};

const READ_DELETE: u32 = 0x80000000 | 0x00010000;
const BUFFER: usize = 128 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    volume: u32,
    high: u32,
    low: u32,
    created_high: u32,
    created_low: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub path: PathBuf,
    pub bytes: u64,
    pub hash: String,
    pub identity: Identity,
}
fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }?;
    if info.dwFileAttributes & 0x400 != 0 {
        bail!("清理路径包含重解析点");
    }
    Ok(info)
}
pub(crate) fn identity(file: &File) -> Result<Identity> {
    let info = info(file)?;
    Ok(Identity {
        volume: info.dwVolumeSerialNumber,
        high: info.nFileIndexHigh,
        low: info.nFileIndexLow,
        created_high: info.ftCreationTime.dwHighDateTime,
        created_low: info.ftCreationTime.dwLowDateTime,
    })
}
pub(crate) fn directory(path: &Path) -> Result<File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    if info(&file)?.dwFileAttributes & 0x10 == 0 {
        bail!("清理根或父路径不是普通目录");
    }
    Ok(file)
}
pub(crate) fn relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("清理清单包含非法相对路径");
    }
    if path.to_string_lossy().contains(':') {
        bail!("清理清单不允许备用数据流");
    }
    Ok(())
}
fn parents(root: &Path, relative_path: &Path) -> Result<Vec<File>> {
    relative(relative_path)?;
    let mut guards = vec![directory(root)?];
    let mut current = root.to_path_buf();
    for part in relative_path
        .parent()
        .context("清理文件缺少父路径")?
        .components()
    {
        current.push(part);
        guards.push(directory(&current)?);
    }
    Ok(guards)
}
fn digest(file: &mut File) -> Result<(u64, String)> {
    if info(file)?.dwFileAttributes & 0x10 != 0 {
        bail!("清理条目不是普通文件");
    }
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0; BUFFER];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok((bytes, format!("{:x}", hash.finalize())));
        }
        bytes = bytes
            .checked_add(count as u64)
            .context("清理文件长度溢出")?;
        hash.update(&buffer[..count]);
    }
}
pub(crate) fn record(root: &Path, path: &Path) -> Result<Entry> {
    let _parents = parents(root, path)?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(root.join(path))?;
    let identity = identity(&file)?;
    let (bytes, hash) = digest(&mut file)?;
    Ok(Entry {
        path: path.to_path_buf(),
        bytes,
        hash,
        identity,
    })
}
/// 不存在表示此前已清理；同内容不同文件身份也必须保留。
pub(crate) fn remove(root: &Path, entry: &Entry) -> Result<()> {
    relative(&entry.path)?;
    let path = root.join(&entry.path);
    if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(());
    }
    let _parents = parents(root, &entry.path)?;
    let mut file = match fs::OpenOptions::new()
        .access_mode(READ_DELETE)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if identity(&file)? != entry.identity {
        bail!("文件身份已变化，已保留：{}", entry.path.display());
    }
    let (bytes, hash) = digest(&mut file)?;
    if bytes != entry.bytes || hash != entry.hash {
        bail!("文件内容已变化，已保留：{}", entry.path.display());
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            &disposition as *const _ as *const _,
            std::mem::size_of_val(&disposition) as u32,
        )
    }?;
    Ok(())
}
