use super::*;
const NAME: &str = ".clipraft-replacement.json";
#[derive(Clone, Copy, Serialize, Deserialize)]
enum Phase {
    Preserve,
    Install,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    task: Replacement,
    old: Vec<files::Entry>,
    new: Vec<files::Entry>,
    phase: Phase,
}
pub(super) fn load_or_create(task: &Replacement) -> Result<Journal> {
    if task.backup.try_exists()? {
        let _guard = crate::owned_files::directory(&task.backup)?;
        let path = task.backup.join(NAME);
        files::plain_metadata(&path)?;
        let journal: Journal = serde_json::from_slice(&fs::read(path)?)?;
        journal.validate(task)?;
        return Ok(journal);
    }
    super::super::database::checkpoint(&task.root)?;
    verify(&task.root)?;
    let old = managed_inventory(&task.root)?;
    let new = managed_inventory(&task.restored)?;
    paths::capacity(&task.root, new.iter().map(|entry| entry.bytes).sum())?;
    fs::create_dir(&task.backup)?;
    let journal = Journal {
        task: task.clone(),
        old,
        new,
        phase: Phase::Preserve,
    };
    journal.save()?;
    Ok(journal)
}
fn managed_inventory(root: &Path) -> Result<Vec<files::Entry>> {
    Ok(files::inventory(root)?
        .into_iter()
        .filter(|entry| files::cleanup_managed(&entry.path))
        .collect())
}
impl Journal {
    fn validate(&self, task: &Replacement) -> Result<()> {
        if self.task.root != task.root
            || self.task.restored != task.restored
            || self.task.backup != task.backup
            || self.task.root_identity != task.root_identity
            || self.task.restored_identity != task.restored_identity
            || self.task.incoming != task.incoming
        {
            bail!("恢复事务日志不匹配");
        }
        for entry in self.old.iter().chain(&self.new) {
            crate::owned_files::relative(&entry.path)?;
            if !files::cleanup_managed(&entry.path) {
                bail!("恢复清单存在非管理路径");
            }
        }
        if managed_inventory(&task.restored)? != self.new {
            bail!("恢复暂存内容已变化，停止替换");
        }
        Ok(())
    }
    fn save(&self) -> Result<()> {
        config::atomic_write(&self.task.backup.join(NAME), &serde_json::to_vec(self)?)
    }
    pub(super) fn apply(&mut self, task: &Replacement) -> Result<()> {
        self.validate(task)?;
        let _backup = crate::owned_files::directory(&task.backup)?;
        if let Phase::Preserve = self.phase {
            for entry in &self.old {
                preserve(task, entry)?;
            }
            verify(&task.backup)?;
            self.phase = Phase::Install;
            self.save()?;
        }
        for entry in &self.new {
            install(task, entry)?;
        }
        Ok(())
    }
    pub(super) fn cleanup_restored(&self, task: &Replacement) -> Result<()> {
        // 已完成的导入副本不再成为正式用户目录，仅清理匹配清单的临时文件。
        for entry in &self.new {
            let actual = crate::owned_files::record(&task.restored, &entry.path)?;
            if actual.bytes != entry.bytes || actual.hash != entry.hash {
                bail!("恢复暂存文件变化，已保留");
            }
            crate::owned_files::remove(&task.restored, &actual)?;
        }
        Ok(())
    }
}
fn preserve(task: &Replacement, entry: &files::Entry) -> Result<()> {
    let source = task.root.join(&entry.path);
    let backup = task.backup.join(&entry.path);
    match (source.try_exists()?, backup.try_exists()?) {
        (true, false) => {
            if !files::matches(&task.root, entry)? {
                bail!("原库文件变化，停止替换");
            }
            files::ensure_parents(&task.backup, &entry.path)?;
            config::move_new(&source, &backup)
        }
        (false, true) => {
            if !files::matches(&task.backup, entry)? {
                bail!("旧库备份不完整，停止替换");
            }
            Ok(())
        }
        (true, true) | (false, false) => bail!("旧库与备份状态冲突，保留所有内容"),
    }
}
fn install(task: &Replacement, entry: &files::Entry) -> Result<()> {
    let destination = task.root.join(&entry.path);
    if destination.try_exists()? {
        if !files::matches(&task.root, entry)? {
            bail!("目标存在不同内容，停止替换");
        }
        return Ok(());
    }
    files::ensure_parents(&task.root, &entry.path)?;
    // 先复制到事务目录，再原子发布，避免中断留下半文件。
    let install_root = task.backup.join("install");
    if !install_root.try_exists()? {
        fs::create_dir(&install_root)?;
    }
    let _install_guard = crate::owned_files::directory(&install_root)?;
    let temporary = install_root.join(&entry.path);
    files::ensure_parents(&install_root, &entry.path)?;
    if temporary.try_exists()? && !files::matches(&install_root, entry)? {
        fs::remove_file(&temporary)?;
    }
    if !temporary.try_exists()? {
        files::copy_verified(&task.restored, &install_root, entry)?;
    }
    config::move_new(&temporary, &destination)
}
