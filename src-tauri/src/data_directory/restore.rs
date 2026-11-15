//! 返回默认目录时先保留目标旧数据，不要求用户清空配置或缓存。
use super::{
    config::{self, Migration},
    database,
    files::{self, Entry},
    paths::{self, Locations},
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
const ARCHIVE_PREFIX: &str = "ClipRaft-preserved-";
const JOURNAL: &str = "restore-journal.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    target: PathBuf,
    names: Vec<String>,
    entries: Vec<Entry>,
    complete: bool,
}
fn managed_names(target: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(target)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("默认目录存在无法识别的文件名，未移动任何文件"))?;
        if paths::default_runtime_file(&name) {
            continue;
        }
        if !files::managed(&name) && !matches!(name.as_str(), "history.db-wal" | "history.db-shm") {
            bail!("默认目录包含非应用管理文件：{name}，请保留原目录并选择其他空目录");
        }
        files::plain_metadata(&entry.path())?;
        names.push(name);
    }
    names.sort();
    Ok(names)
}
pub(super) fn preview(target: &Path, locations: &Locations) -> Result<Option<PathBuf>> {
    if target != fs::canonicalize(&locations.default_root)? {
        paths::empty_target(target, locations)?;
        return Ok(None);
    }
    if managed_names(target)?.is_empty() {
        return Ok(None);
    }
    files::measure(target)?;
    let parent = target.parent().context("默认目录缺少父目录")?;
    paths::writable(parent)?;
    Ok(Some(
        parent.join(format!("{ARCHIVE_PREFIX}{}", config::nonce()?)),
    ))
}
pub(super) fn confirm(target: &Path, archive: Option<&Path>, locations: &Locations) -> Result<()> {
    let Some(archive) = archive else {
        return paths::empty_target(target, locations);
    };
    validate_archive(target, archive, locations)?;
    if archive.try_exists()? {
        bail!("预定保留目录已存在，请重新选择恢复默认位置");
    }
    managed_names(target)?;
    files::measure(target)?;
    Ok(())
}
fn validate_archive(target: &Path, archive: &Path, locations: &Locations) -> Result<()> {
    if target != fs::canonicalize(&locations.default_root)? || archive.parent() != target.parent() {
        bail!("旧数据保留路径不属于默认目录的同级位置");
    }
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .context("旧数据保留路径无效")?;
    let id = name
        .strip_prefix(ARCHIVE_PREFIX)
        .context("旧数据保留目录名称无效")?;
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("旧数据保留目录标识无效");
    }
    Ok(())
}
pub(super) fn preserve(task: &Migration, locations: &Locations) -> Result<()> {
    let Some(archive) = &task.displaced else {
        return Ok(());
    };
    validate_archive(&task.target, archive, locations)?;
    let mut journal = journal(task, archive)?;
    if journal.complete {
        return Ok(());
    }
    move_entries(task, archive, &journal)?;
    if files::inventory(archive)? != journal.entries {
        bail!("默认目录旧数据保留校验失败，未切换启动定位");
    }
    journal.complete = true;
    config::atomic_write(
        &archive.join(JOURNAL),
        &serde_json::to_vec_pretty(&journal)?,
    )
}
fn journal(task: &Migration, archive: &Path) -> Result<Journal> {
    if !archive.try_exists()? {
        fs::create_dir(archive)?;
    }
    if !files::plain_metadata(archive)?.is_dir() {
        bail!("旧数据保留位置不是普通目录");
    }
    let path = archive.join(JOURNAL);
    if path.try_exists()? {
        files::plain_metadata(&path)?;
        let journal: Journal = serde_json::from_slice(&fs::read(&path)?)?;
        validate_journal(&journal, task)?;
        return Ok(journal);
    }
    if fs::read_dir(archive)?.next().is_some() {
        bail!("旧数据保留目录被其他文件占用，拒绝覆盖");
    }
    // checkpoint 只在全量旧数据尚在目标时执行；中断恢复时旧文件可能已分批移入保留目录。
    if task.target.join("history.db").is_file() && task.target.join("key.bin").is_file() {
        database::checkpoint(&task.target)?;
    }
    let names = managed_names(&task.target)?;
    let entries = files::inventory(&task.target)?;
    let journal = Journal {
        target: task.target.clone(),
        names,
        entries,
        complete: false,
    };
    config::atomic_write(&path, &serde_json::to_vec_pretty(&journal)?)?;
    Ok(journal)
}
fn validate_journal(journal: &Journal, task: &Migration) -> Result<()> {
    if journal.target != task.target {
        bail!("默认目录保留日志不匹配");
    }
    for name in &journal.names {
        let path = Path::new(name);
        if path.components().count() != 1
            || !(files::managed(name)
                || matches!(name.as_str(), "history.db-wal" | "history.db-shm"))
        {
            bail!("默认目录保留日志包含非法文件名");
        }
    }
    Ok(())
}
fn move_entries(task: &Migration, archive: &Path, journal: &Journal) -> Result<()> {
    for name in &journal.names {
        let from = task.target.join(name);
        let to = archive.join(name);
        match (from.try_exists()?, to.try_exists()?) {
            (true, false) => {
                files::plain_metadata(&from)?;
                config::move_new(&from, &to)?;
            }
            (false, true) => {
                files::plain_metadata(&to)?;
            }
            (true, true) => bail!("旧数据源和保留目录同时有 {name}，拒绝覆盖"),
            (false, false) => bail!("旧数据文件 {name} 缺失，停止迁移"),
        }
    }
    Ok(())
}
