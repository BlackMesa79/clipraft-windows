use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) const LOCK_NAME: &str = ".clipraft-data.lock";
const COPY_BUFFER: usize = 128 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub path: PathBuf,
    pub bytes: u64,
    pub hash: String,
}
pub(super) fn plain_metadata(path: &Path) -> Result<fs::Metadata> {
    use std::os::windows::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)?;
    // Junction 与符号链接均不能将迁移写入引向清单之外。
    if metadata.file_attributes() & 0x400 != 0 {
        bail!("迁移路径包含重解析点：{}", path.display());
    }
    Ok(metadata)
}
fn lock_file(root: &Path) -> Result<File> {
    let path = root.join(LOCK_NAME);
    if path.exists() {
        plain_metadata(&path)?;
    }
    Ok(fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?)
}
pub(super) fn lock(root: &Path) -> Result<File> {
    let file = lock_file(root)?;
    file.try_lock().map_err(|error| {
        anyhow::anyhow!("数据目录正被其他实例使用（{}）：{error}", root.display())
    })?;
    Ok(file)
}
pub(super) fn startup_lock(root: &Path) -> Result<File> {
    let file = lock_file(root)?;
    wait_for_lock(&file)
        .with_context(|| format!("数据目录正被其他实例使用：{}", root.display()))?;
    Ok(file)
}
pub(super) fn wait_for_lock(file: &File) -> Result<()> {
    const RESTART_LOCK_ATTEMPTS: usize = 40;
    const RESTART_LOCK_DELAY: std::time::Duration = std::time::Duration::from_millis(50);
    // Tauri 的新进程可能先于旧进程完全退出，仅启动时短暂等待旧句柄释放。
    for _ in 0..RESTART_LOCK_ATTEMPTS {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => std::thread::sleep(RESTART_LOCK_DELAY),
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
    }
    bail!("目录锁仍被占用，请先退出正在使用它的实例")
}
pub(super) fn fingerprint(path: &Path) -> Result<(u64, String)> {
    if !plain_metadata(path)?.is_file() {
        bail!("迁移清单中存在非普通文件：{}", path.display());
    }
    let mut source = File::open(path)?;
    let mut digest = Sha256::new();
    let mut count = 0;
    let mut buffer = vec![0u8; COPY_BUFFER];
    loop {
        let size = source.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        count += size as u64;
        digest.update(&buffer[..size]);
    }
    Ok((count, format!("{:x}", digest.finalize())))
}
pub(super) fn matches(root: &Path, entry: &Entry) -> Result<bool> {
    let path = root.join(&entry.path);
    if !path.try_exists()? {
        return Ok(false);
    }
    let (bytes, hash) = fingerprint(&path)?;
    Ok(bytes == entry.bytes && hash == entry.hash)
}
pub(super) fn managed(name: &str) -> bool {
    matches!(name, "history.db" | "key.bin" | "data") || name.starts_with("history.db.bak-v")
}
pub(super) fn inventory(root: &Path) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for child in fs::read_dir(root)? {
        let child = child?;
        if managed(&child.file_name().to_string_lossy()) {
            collect(root, &child.path(), &mut entries)?;
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}
fn collect(root: &Path, path: &Path, entries: &mut Vec<Entry>) -> Result<()> {
    let metadata = plain_metadata(path)?;
    if metadata.is_dir() {
        for child in fs::read_dir(path)? {
            collect(root, &child?.path(), entries)?;
        }
        return Ok(());
    }
    let (bytes, hash) = fingerprint(path)?;
    entries.push(Entry {
        path: path.strip_prefix(root)?.to_path_buf(),
        bytes,
        hash,
    });
    Ok(())
}
pub(super) fn measure(root: &Path) -> Result<u64> {
    let mut bytes = 0u64;
    for child in fs::read_dir(root)? {
        let child = child?;
        if managed(&child.file_name().to_string_lossy()) {
            bytes = bytes
                .checked_add(tree_size(&child.path())?)
                .context("迁移数据大小溢出")?;
        }
    }
    Ok(bytes)
}
fn tree_size(path: &Path) -> Result<u64> {
    let metadata = plain_metadata(path)?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0u64;
    for child in fs::read_dir(path)? {
        total = total
            .checked_add(tree_size(&child?.path())?)
            .context("迁移数据大小溢出")?;
    }
    Ok(total)
}
pub(super) fn ensure_parents(root: &Path, relative: &Path) -> Result<()> {
    let mut parent = root.to_path_buf();
    let parts = relative.parent().context("迁移文件缺少相对父路径")?;
    for part in parts.components() {
        parent.push(part);
        if !parent.try_exists()? {
            fs::create_dir(&parent)?;
        }
        if !plain_metadata(&parent)?.is_dir() {
            bail!("目标文件父路径不是普通目录");
        }
    }
    Ok(())
}
pub(super) fn copy_verified(source: &Path, target: &Path, entry: &Entry) -> Result<()> {
    let mut input = File::open(source.join(&entry.path))?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target.join(&entry.path))?;
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        digest.update(&buffer[..count]);
        bytes += count as u64;
    }
    output.sync_all()?;
    if bytes != entry.bytes || format!("{:x}", digest.finalize()) != entry.hash {
        bail!("复制期间源文件发生变化：{}", entry.path.display());
    }
    Ok(())
}
