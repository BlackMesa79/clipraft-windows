//! 单次快照创建记录；失败时由拥有者显式清理，不在 Drop 中删除文件。
use anyhow::{bail, Context, Result};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

pub(crate) struct SnapshotFiles {
    root: PathBuf,
    files: Vec<PathBuf>,
    created: bool,
}
impl SnapshotFiles {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            files: Vec::new(),
            created: false,
        }
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn create(&mut self) -> Result<()> {
        fs::create_dir(&self.root).context("快照目标必须是尚不存在的目录")?;
        self.created = true;
        fs::create_dir_all(self.root.join("data/blobs"))?;
        // SQLCipher自行创建数据库及回滚日志，独占根目录创建后才取得这些路径的所有权。
        for name in [
            "history.db",
            "history.db-journal",
            "history.db-wal",
            "history.db-shm",
        ] {
            self.files.push(self.root.join(name));
        }
        Ok(())
    }
    pub(crate) fn create_blob(&mut self, hash: &str) -> Result<File> {
        let path = self.root.join("data/blobs").join(hash);
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        self.files.push(path);
        Ok(file)
    }
    pub(crate) fn cleanup(self) -> Result<()> {
        if !self.created {
            return Ok(());
        }
        for relative in ["", "data", "data/blobs"] {
            let path = self.root.join(relative);
            if path.try_exists()? && !super::snapshot::plain_metadata(&path)?.is_dir() {
                bail!("快照目录已被替换，停止清理");
            }
        }
        for path in self.files {
            if !path.try_exists()? {
                continue;
            }
            if !super::snapshot::plain_metadata(&path)?.is_file() {
                bail!("快照文件已被替换，停止清理");
            }
            fs::remove_file(path)?;
        }
        for relative in ["data/blobs", "data", ""] {
            let path = self.root.join(relative);
            if path.try_exists()? {
                fs::remove_dir(path).context("快照暂存有额外内容或无法清理，已保留")?;
            }
        }
        Ok(())
    }
}
