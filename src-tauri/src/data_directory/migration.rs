use super::{
    config::{Configuration, Migration},
    database,
    files::{self, Entry},
    paths::{self, Locations},
};
use anyhow::{bail, Context, Result};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

pub(super) type Progress<'a> = dyn FnMut(&str, u64, u64) + 'a;
const MANIFEST: &str = ".clipraft-manifest.json";
fn stage(task: &Migration) -> Result<PathBuf> {
    Ok(task
        .target
        .parent()
        .context("目标目录缺少父目录")?
        .join(format!(".clipraft-migration-{}", task.id)))
}
pub(super) fn execute(
    locations: &Locations,
    config: &Configuration,
    progress: &mut Progress<'_>,
) -> Result<(Configuration, File)> {
    let task = config.pending.as_ref().context("缺少迁移任务")?;
    let target = paths::target_path(&task.source, &task.target)?;
    if target != task.target {
        bail!("目标目录身份已变化，请重新选择目录");
    }
    let target_lock = files::lock(&target)?;
    progress("正在检查数据库与 WAL", 0, 0);
    database::checkpoint(&task.source)?;
    database::verify(&task.source)?;
    let entries = files::inventory(&task.source)?;
    progress("正在创建并验证迁移前备份", 0, 0);
    let cleanup = super::backup::prepare(task, &entries)?;
    progress("正在保留默认目录旧数据", 0, 0);
    super::restore::preserve(task, locations)?;
    let stage = prepare_stage(task, &entries)?;
    validate_destination(&target, &entries, locations)?;
    paths::capacity(&target, remaining_bytes(task, &stage, &entries)?)?;
    transfer(task, &stage, &entries, progress)?;
    progress("正在验证目标数据库", 0, 0);
    database::verify(&target)?;
    let mut committed = config.clone();
    committed.root = target;
    committed.previous = Some(task.source.clone());
    committed.pending = None;
    committed.cleanup = Some(cleanup);
    committed.displaced = task.displaced.clone();
    locations.save(&committed)?;
    // 提交后不再返回失败，防止用户误以为仍可退回旧快照。
    if let Err(error) = remove_empty_stage(&stage) {
        eprintln!("迁移已完成，暂存目录清理失败：{error}");
    }
    Ok((committed, target_lock))
}
fn prepare_stage(task: &Migration, entries: &[Entry]) -> Result<PathBuf> {
    let root = stage(task)?;
    if !root.try_exists()? {
        fs::create_dir(&root).context("无法在目标磁盘建立迁移暂存目录")?;
    }
    if !files::plain_metadata(&root)?.is_dir() {
        bail!("迁移暂存路径异常");
    }
    let path = root.join(MANIFEST);
    if path.try_exists()? {
        files::plain_metadata(&path)?;
        let previous: Vec<Entry> = serde_json::from_slice(&fs::read(path)?)?;
        if previous != entries {
            bail!("中断后源数据已变化，请通过恢复入口选择源目录，再换一个空目标目录迁移");
        }
    } else {
        if fs::read_dir(&root)?.next().is_some() {
            bail!("迁移暂存目录被其他文件占用");
        }
        super::config::atomic_write(&path, &serde_json::to_vec(entries)?)?;
    }
    check_known_tree(&root, Path::new(""), entries, Some(MANIFEST))?;
    Ok(root)
}
fn validate_destination(root: &Path, entries: &[Entry], locations: &Locations) -> Result<()> {
    for child in fs::read_dir(root)? {
        let child = child?;
        let name = child.file_name();
        if name == files::LOCK_NAME {
            files::plain_metadata(&child.path())?;
            continue;
        }
        if root == fs::canonicalize(&locations.default_root)?
            && name.to_str().is_some_and(paths::default_runtime_file)
        {
            continue;
        }
        check_known_tree(root, Path::new(&name), entries, None)?;
    }
    Ok(())
}
fn check_known_tree(
    root: &Path,
    relative: &Path,
    entries: &[Entry],
    ignored: Option<&str>,
) -> Result<()> {
    let path = root.join(relative);
    let metadata = files::plain_metadata(&path)?;
    if metadata.is_dir() {
        if !relative.as_os_str().is_empty()
            && !entries.iter().any(|entry| entry.path.starts_with(relative))
        {
            bail!("迁移目录出现清单外目录：{}", path.display());
        }
        for child in fs::read_dir(path)? {
            check_known_tree(root, &relative.join(child?.file_name()), entries, ignored)?;
        }
        return Ok(());
    }
    if relative.to_str() == ignored {
        return Ok(());
    }
    if !entries.iter().any(|entry| entry.path == relative) {
        bail!("迁移目录出现清单外文件：{}", path.display());
    }
    Ok(())
}
fn remaining_bytes(task: &Migration, stage: &Path, entries: &[Entry]) -> Result<u64> {
    let mut required = 0u64;
    for entry in entries {
        if files::matches(&task.target, entry)? || files::matches(stage, entry)? {
            continue;
        }
        let partial = stage.join(&entry.path);
        let reclaimable = if partial.try_exists()? {
            files::plain_metadata(&partial)?.len()
        } else {
            0
        };
        required = required
            .checked_add(entry.bytes.saturating_sub(reclaimable))
            .context("迁移大小溢出")?;
    }
    Ok(required)
}
fn transfer(
    task: &Migration,
    stage: &Path,
    entries: &[Entry],
    progress: &mut Progress<'_>,
) -> Result<()> {
    let total: u64 = entries.iter().map(|entry| entry.bytes).sum();
    let mut copied = 0;
    for entry in entries {
        progress("正在复制并校验加密数据", copied, total);
        transfer_entry(task, stage, entry)?;
        copied += entry.bytes;
    }
    progress("正在核对迁移结果", total, total);
    for entry in entries {
        if !files::matches(&task.target, entry)? {
            bail!("目标校验不一致：{}", entry.path.display());
        }
    }
    Ok(())
}
fn transfer_entry(task: &Migration, stage: &Path, entry: &Entry) -> Result<()> {
    let destination = task.target.join(&entry.path);
    if destination.try_exists()? {
        if !files::matches(&task.target, entry)? {
            bail!("目标已有文件与源不符，拒绝覆盖：{}", destination.display());
        }
        return Ok(());
    }
    files::ensure_parents(stage, &entry.path)?;
    let temporary = stage.join(&entry.path);
    if temporary.try_exists()? && !files::matches(stage, entry)? {
        fs::remove_file(&temporary)?;
    }
    if !temporary.try_exists()? {
        files::copy_verified(&task.source, stage, entry)?;
    }
    files::ensure_parents(&task.target, &entry.path)?;
    super::config::move_new(&temporary, &destination)
}
fn remove_empty_stage(root: &Path) -> Result<()> {
    for child in fs::read_dir(root)? {
        let child = child?;
        if files::plain_metadata(&child.path())?.is_dir() {
            remove_empty_stage(&child.path())?;
        }
    }
    let manifest = root.join(MANIFEST);
    if manifest.try_exists()? {
        fs::remove_file(manifest)?;
    }
    fs::remove_dir(root)?;
    Ok(())
}
