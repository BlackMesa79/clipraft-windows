//! 迁移前冷备份及源清理日志；备份完成后才允许复制/提交目标。
use super::{
    config::{self, Migration},
    database,
    files::{self, Entry},
    paths,
};
use crate::owned_files::{self, Identity};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const JOURNAL: &str = ".clipraft-backup.json";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cleanup {
    pub source: PathBuf,
    pub target: PathBuf,
    pub backup: PathBuf,
    pub source_identity: Identity,
    pub entries: Vec<owned_files::Entry>,
    pub initialized: bool,
    pub complete: bool,
    pub remaining: usize,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupJournal {
    source: PathBuf,
    target: PathBuf,
    entries: Vec<Entry>,
    source_files: Vec<owned_files::Entry>,
}

pub(super) fn location(task: &Migration) -> Result<PathBuf> {
    Ok(task
        .target
        .parent()
        .context("目标目录缺少父目录")?
        .join(format!(".clipraft-backup-{}", task.id)))
}
pub(super) fn prepare(task: &Migration, entries: &[Entry]) -> Result<Cleanup> {
    let backup = location(task)?;
    if backup.starts_with(&task.source) || backup.starts_with(&task.target) {
        bail!("迁移备份不能位于源或目标目录内部");
    }
    let source_guard = owned_files::directory(&task.source)?;
    let source_identity = owned_files::identity(&source_guard)?;
    let journal = prepare_journal(task, &backup, entries)?;
    let _backup_guard = owned_files::directory(&backup)?;
    let bytes = entries.iter().try_fold(0u64, |total, entry| {
        total.checked_add(entry.bytes).context("备份大小溢出")
    })?;
    // 同卷还要为目标复制留空间；中断重试采用保守预算。
    paths::capacity(
        backup.parent().context("备份缺少父目录")?,
        bytes.checked_mul(2).context("迁移空间预算溢出")?,
    )?;
    for entry in entries {
        files::ensure_parents(&backup, &entry.path)?;
        if files::matches(&backup, entry)? {
            continue;
        }
        let partial = backup.join(&entry.path);
        if partial.try_exists()? {
            bail!("备份文件与清单不符，已保留；请选择新目标重新安排迁移");
        }
        files::copy_verified(&task.source, &backup, entry)?;
    }
    verify(&backup, entries)?;
    let source_files: Vec<_> = journal
        .source_files
        .into_iter()
        .filter(|entry| files::cleanup_managed(&entry.path))
        .collect();
    let remaining = source_files.len();
    Ok(Cleanup {
        source: task.source.clone(),
        target: task.target.clone(),
        backup,
        source_identity,
        entries: source_files,
        initialized: false,
        complete: false,
        remaining,
        error: None,
    })
}
fn prepare_journal(task: &Migration, backup: &Path, entries: &[Entry]) -> Result<BackupJournal> {
    if !backup.try_exists()? {
        fs::create_dir(backup)?;
    }
    let _guard = owned_files::directory(backup)?;
    let path = backup.join(JOURNAL);
    if path.try_exists()? {
        files::plain_metadata(&path)?;
        let journal: BackupJournal = serde_json::from_slice(&fs::read(path)?)?;
        if journal.source != task.source
            || journal.target != task.target
            || journal.entries != entries
        {
            bail!("备份日志与当前迁移不匹配，已保留备份和源数据");
        }
        validate_source_files(&task.source, &journal.source_files, entries)?;
        return Ok(journal);
    }
    if fs::read_dir(backup)?.next().is_some() {
        bail!("备份目录已被占用");
    }
    let source_files = entries
        .iter()
        .map(|entry| owned_files::record(&task.source, &entry.path))
        .collect::<Result<Vec<_>>>()?;
    validate_source_files(&task.source, &source_files, entries)?;
    let journal = BackupJournal {
        source: task.source.clone(),
        target: task.target.clone(),
        entries: entries.to_vec(),
        source_files,
    };
    config::atomic_write(&path, &serde_json::to_vec(&journal)?)?;
    Ok(journal)
}
fn validate_source_files(
    source: &Path,
    recorded: &[owned_files::Entry],
    entries: &[Entry],
) -> Result<()> {
    if recorded.len() != entries.len() {
        bail!("备份源清单条目数量不一致");
    }
    for (saved, entry) in recorded.iter().zip(entries) {
        if saved.path != entry.path
            || saved.bytes != entry.bytes
            || saved.hash != entry.hash
            || *saved != owned_files::record(source, &entry.path)?
        {
            bail!("迁移源文件身份或内容已变化，请保留备份重新安排迁移");
        }
    }
    Ok(())
}
pub(super) fn verify(root: &Path, entries: &[Entry]) -> Result<()> {
    for entry in entries {
        owned_files::relative(&entry.path)?;
        let actual = owned_files::record(root, &entry.path)?;
        if actual.bytes != entry.bytes || actual.hash != entry.hash {
            bail!("迁移备份校验失败：{}", entry.path.display());
        }
    }
    database::verify(root)?;
    let master = crate::platform::master_key(root)?;
    crate::storage::Storage::verify_snapshot(root, &master)
}
