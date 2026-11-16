//! 新库初始化后的源文件清理；失败只能待重试，绝不回滚新库定位。
use super::{backup, files, DirectoryControl};
use crate::owned_files;
use anyhow::{bail, Context, Result};
use std::fs;

impl DirectoryControl {
    pub(crate) fn mark_migration_initialized(&self) -> Result<()> {
        let _serial = self
            .plan
            .lock()
            .map_err(|_| anyhow::anyhow!("目录操作状态不可用"))?;
        let mut config = self.active_configuration()?;
        let Some(cleanup) = config.cleanup.as_mut() else {
            return Ok(());
        };
        if cleanup.target != self.root || cleanup.complete || cleanup.initialized {
            return Ok(());
        }
        cleanup.initialized = true;
        self.locations.save(&config)
    }
    pub(crate) fn retry_source_cleanup(&self) -> Result<()> {
        let _serial = self
            .plan
            .lock()
            .map_err(|_| anyhow::anyhow!("目录操作状态不可用"))?;
        let mut config = self.active_configuration()?;
        let Some(cleanup) = config.cleanup.as_mut() else {
            return Ok(());
        };
        if cleanup.complete {
            return Ok(());
        }
        if !cleanup.initialized || cleanup.target != self.root {
            bail!("新库尚未成功初始化，拒绝源清理");
        }
        let result = cleanup_source(cleanup);
        if let Err(error) = &result {
            cleanup.error = Some(format!("迁移成功，旧文件待清理：{error:#}"));
        }
        self.locations
            .save(&config)
            .context("新库仍在使用，但清理状态写入失败，可重试")?;
        result
    }
}
fn validate_cleanup(cleanup: &backup::Cleanup) -> Result<(std::path::PathBuf, Vec<std::fs::File>)> {
    let _original_source = owned_files::directory(&cleanup.source)?;
    let _original_backup = owned_files::directory(&cleanup.backup)?;
    let source = fs::canonicalize(&cleanup.source)?;
    let target = fs::canonicalize(&cleanup.target)?;
    let backup = fs::canonicalize(&cleanup.backup)?;
    if source == target
        || target.starts_with(&source)
        || source.starts_with(&target)
        || backup.starts_with(&source)
        || backup.starts_with(&target)
    {
        bail!("源清理路径边界无效");
    }
    let root_guard = owned_files::directory(&source)?;
    if owned_files::identity(&root_guard)? != cleanup.source_identity {
        bail!("源目录身份已变化，保留全部源文件");
    }
    let _source_lock = files::lock(&source)?;
    validate_backup(cleanup, &backup)?;
    // 源库重新启用可能有尚未checkpoint的变化，不能仅凭主库哈希判断未变。
    for name in ["history.db-wal", "history.db-shm", "history.db-journal"] {
        if source.join(name).try_exists()? {
            bail!("源库出现运行日志，保留源数据等待检查");
        }
    }
    // 任一源文件变化时整批保留，避免删除后来新增历史仍引用的旧blob。
    for entry in &cleanup.entries {
        let path = source.join(&entry.path);
        if !path.try_exists()? {
            continue;
        }
        if owned_files::record(&source, &entry.path)? != *entry {
            bail!("源文件身份或内容已变化，整批保留：{}", entry.path.display());
        }
    }
    Ok((
        source,
        vec![_original_source, _original_backup, root_guard, _source_lock],
    ))
}
fn validate_backup(cleanup: &backup::Cleanup, backup: &std::path::Path) -> Result<()> {
    let entries = cleanup
        .entries
        .iter()
        .map(|entry| files::Entry {
            path: entry.path.clone(),
            bytes: entry.bytes,
            hash: entry.hash.clone(),
        })
        .collect::<Vec<_>>();
    for entry in &entries {
        validate_path(&entry.path)?;
    }
    backup::verify(backup, &entries).context("备份不可用，保留源数据")
}

fn cleanup_source(cleanup: &mut backup::Cleanup) -> Result<()> {
    let (source, _guards) = validate_cleanup(cleanup)?;
    let mut remaining = 0;
    let mut first_error = None;
    for entry in &cleanup.entries {
        if let Err(error) = owned_files::remove(&source, entry) {
            remaining += 1;
            first_error.get_or_insert_with(|| format!("{error:#}"));
        }
    }
    cleanup.remaining = remaining;
    cleanup.complete = false;
    cleanup.error =
        first_error.map(|error| format!("迁移成功，{remaining}个旧文件待清理：{error}"));
    if let Some(error) = &cleanup.error {
        bail!("{error}");
    }
    prune_empty_parents(&source, &cleanup.entries)?;
    cleanup.complete = true;
    Ok(())
}
fn prune_empty_parents(root: &std::path::Path, entries: &[owned_files::Entry]) -> Result<()> {
    let mut parents = std::collections::BTreeSet::new();
    for entry in entries {
        let mut parent = entry.path.parent();
        while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
            parents.insert(path.to_path_buf());
            parent = path.parent();
        }
    }
    for path in parents.iter().rev() {
        let absolute = root.join(path);
        if !absolute.try_exists()? {
            continue;
        }
        let guard = owned_files::directory(&absolute)?;
        if fs::read_dir(&absolute)?.next().is_some() {
            continue;
        }
        drop(guard);
        match fs::remove_dir(&absolute) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn validate_path(path: &std::path::Path) -> Result<()> {
    owned_files::relative(path)?;
    if !files::cleanup_managed(path) {
        bail!("源清理清单包含非管理文件");
    }
    Ok(())
}

pub(crate) fn after_frontend_ready(app: &tauri::AppHandle) {
    use tauri::Manager;
    let Some(directory) = app.try_state::<std::sync::Arc<DirectoryControl>>() else {
        return;
    };
    let Some(service) = app.try_state::<crate::archive_service::Service>() else {
        return;
    };
    if !service.claim_cleanup_start() {
        return;
    }
    let directory = directory.inner().clone();
    let jobs = service.jobs.clone();
    let result = std::thread::Builder::new()
        .name("clipraft-migration-cleanup".into())
        .spawn(move || {
            let result = (|| {
                let _operation = jobs.begin_restart()?;
                directory.mark_migration_initialized()?;
                directory.retry_source_cleanup()
            })();
            if let Err(error) = result {
                crate::diag::write(format!("migration cleanup pending: {error:#}"));
            }
        });
    if let Err(error) = result {
        crate::diag::write(format!("migration cleanup worker unavailable: {error}"));
    }
}
