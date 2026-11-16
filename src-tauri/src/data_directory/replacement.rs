//! 固定目录恢复事务：停写重启后保留旧文件、安装新文件，位置始终不变。
use super::{
    config::{self, Configuration},
    files,
    paths::{self, Locations},
    DirectoryControl,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

#[path = "replacement_journal.rs"]
mod journal;
const PREFIX: &str = ".clipraft-replaced-";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Replacement {
    pub root: PathBuf,
    pub restored: PathBuf,
    pub backup: PathBuf,
    pub root_identity: crate::owned_files::Identity,
    pub restored_identity: crate::owned_files::Identity,
    pub incoming: Vec<files::Entry>,
}
impl Replacement {
    pub(super) fn validate(&self) -> Result<()> {
        if !self.root.is_absolute() || !self.restored.is_absolute() || !self.backup.is_absolute() {
            bail!("恢复事务需要绝对路径");
        }
        if self.root.starts_with(&self.restored)
            || self.restored.starts_with(&self.root)
            || self.backup.parent() != self.root.parent()
        {
            bail!("恢复事务源/目标/备份边界无效");
        }
        let nonce = self
            .backup
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(PREFIX))
            .context("恢复备份名称无效")?;
        if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("恢复备份标识无效");
        }
        Ok(())
    }
}
impl DirectoryControl {
    pub(crate) fn schedule_replacement(&self, restored: &Path) -> Result<File> {
        if self.locations.config.is_none() {
            bail!("当前隔离环境未配置启动定位，不能替换数据库");
        }
        let mut planned = self
            .plan
            .lock()
            .map_err(|_| anyhow::anyhow!("目录操作状态不可用"))?;
        let mut config = self.active_configuration()?;
        if config.pending.is_some() || config.cleanup.as_ref().is_some_and(|plan| !plan.complete) {
            bail!("已有迁移或源清理待完成，不能替换数据");
        }
        let restored = paths::target_path(&self.root, restored)?;
        let guard = files::lock(&restored)?;
        verify(&restored)?;
        let root_identity =
            crate::owned_files::identity(&crate::owned_files::directory(&self.root)?)?;
        let restored_identity =
            crate::owned_files::identity(&crate::owned_files::directory(&restored)?)?;
        let incoming = files::inventory(&restored)?;
        let task = Replacement {
            root_identity,
            restored_identity,
            incoming,
            root: self.root.clone(),
            restored,
            backup: self
                .root
                .parent()
                .context("当前目录缺少父目录")?
                .join(format!("{PREFIX}{}", config::nonce()?)),
        };
        task.validate()?;
        if task.backup.try_exists()? {
            bail!("恢复备份位置已存在");
        }
        config.replacement = Some(task);
        self.locations.save(&config)?;
        *planned = None;
        Ok(guard)
    }
}
fn verify(root: &Path) -> Result<()> {
    for name in ["history.db", "key.bin"] {
        if !files::plain_metadata(&root.join(name))?.is_file() {
            bail!("恢复数据库或密钥不是普通文件");
        }
    }
    files::inventory(root)?;
    let master = crate::platform::master_key(root)?;
    crate::storage::Storage::verify_snapshot(root, &master)
}
pub(super) fn execute(locations: &Locations, config: &Configuration) -> Result<Configuration> {
    let task = config.replacement.as_ref().context("缺少恢复替换任务")?;
    task.validate()?;
    let owner_path = task.restored.parent().context("恢复暂存缺少父目录")?;
    let owner = crate::archive::staging::Directory::claim(owner_path)?
        .context("恢复暂存正由其他任务使用")?;
    let _root = crate::owned_files::directory(&task.root)?;
    let _restored = crate::owned_files::directory(&task.restored)?;
    if crate::owned_files::identity(&_root)? != task.root_identity
        || crate::owned_files::identity(&_restored)? != task.restored_identity
    {
        bail!("恢复事务目录身份变化，保留全部数据");
    }
    let _restored_lock = files::lock(&task.restored)?;
    verify(&task.restored)?;
    if files::inventory(&task.restored)? != task.incoming {
        bail!("确认后恢复内容已变化，停止替换");
    }
    let mut journal = journal::load_or_create(task)?;
    journal.apply(task)?;
    verify(&task.root)?;
    let mut committed = config.clone();
    committed.replacement = None;
    committed.previous = Some(task.backup.clone());
    committed.startup_capture = crate::capture_policy::StartupCapture::ChangesOnly;
    locations.save(&committed)?;
    // 新库已开始作为权威，暂存清理失败只能提示，不能回滚启动位置。
    let cleaned = journal.cleanup_restored(task);
    drop(_restored_lock);
    drop(_restored);
    let cleaned = cleaned.and_then(|()| cleanup_directories(task, owner));
    if let Err(error) = cleaned {
        eprintln!("恢复已完成，临时数据待清理：{error:#}");
    }
    Ok(committed)
}

fn cleanup_directories(
    task: &Replacement,
    owner: crate::archive::staging::Directory,
) -> Result<()> {
    let lock = task.restored.join(files::LOCK_NAME);
    if lock.try_exists()? {
        let record = crate::owned_files::record(&task.restored, Path::new(files::LOCK_NAME))?;
        crate::owned_files::remove(&task.restored, &record)?;
    }
    for relative in ["data/blobs", "data", ""] {
        let path = task.restored.join(relative);
        if path.try_exists()? {
            fs::remove_dir(path)?;
        }
    }
    owner.cleanup()
}
#[cfg(test)]
#[path = "replacement_tests.rs"]
mod tests;
