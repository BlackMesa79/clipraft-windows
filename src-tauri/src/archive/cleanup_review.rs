//! 遗留暂存显式预检与确认清理；仅接受任务布局，未知内容继续保留。
use super::staging::Directory;
use crate::owned_files::{self, Entry, Identity};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 100_000;
#[derive(Clone, Serialize)]
pub(crate) struct Preview {
    pub ticket: String,
    pub root: PathBuf,
    pub files: usize,
    pub bytes: u64,
}
pub(crate) struct Review {
    pub preview: Preview,
    directory: Directory,
    files: Vec<Entry>,
    directories: Vec<(PathBuf, Identity)>,
}
impl Review {
    pub(crate) fn prepare(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            bail!("暂存清理需要绝对路径");
        }
        let directory = Directory::claim(root)?.context("暂存仍由活动任务使用")?;
        let root = root.to_path_buf();
        let mut review = Self {
            preview: Preview {
                ticket: nonce()?,
                root,
                files: 0,
                bytes: 0,
            },
            directory,
            files: Vec::new(),
            directories: Vec::new(),
        };
        review.collect(Path::new(""))?;
        review.preview.files = review.files.len();
        review.preview.bytes = review.files.iter().try_fold(0u64, |total, entry| {
            total.checked_add(entry.bytes).context("暂存大小溢出")
        })?;
        Ok(review)
    }
    fn collect(&mut self, relative: &Path) -> Result<()> {
        let path = self.directory.path().join(relative);
        let guard = owned_files::directory(&path)?;
        self.directories
            .push((relative.to_path_buf(), owned_files::identity(&guard)?));
        for child in fs::read_dir(path)? {
            let child = child?;
            if relative.as_os_str().is_empty() && child.file_name() == super::staging_owner::NAME {
                continue;
            }
            let name = relative.join(child.file_name());
            let metadata = crate::data_directory::plain_metadata(&child.path())?;
            if metadata.is_dir() {
                self.collect_directory(&name)?;
                continue;
            }
            if !known_file(&name) {
                bail!("暂存包含未知文件，未安排清理：{}", name.display());
            }
            if self.files.len() >= MAX_FILES {
                bail!("暂存清理条目超过上限，请保留并检查");
            }
            self.files
                .push(owned_files::record(self.directory.path(), &name)?);
        }
        Ok(())
    }
    fn collect_directory(&mut self, relative: &Path) -> Result<()> {
        if ![
            Path::new("database"),
            Path::new("database/data"),
            Path::new("database/data/blobs"),
        ]
        .contains(&relative)
        {
            bail!("暂存包含未知目录，未安排清理");
        }
        self.collect(relative)
    }
    pub(crate) fn confirm(self, ticket: &str) -> Result<()> {
        if ticket != self.preview.ticket {
            bail!("暂存清理确认已失效");
        }
        let guards = self
            .directories
            .iter()
            .map(|(relative, expected)| {
                let guard = owned_files::directory(&self.directory.path().join(relative))?;
                if owned_files::identity(&guard)? != *expected {
                    bail!("暂存目录身份变化，未执行清理");
                }
                Ok(guard)
            })
            .collect::<Result<Vec<_>>>()?;
        // 删除前先检查所有既有文件，发现变化时不开始删除。
        for entry in &self.files {
            if owned_files::record(self.directory.path(), &entry.path)? != *entry {
                bail!("暂存文件已变化，请重新预检");
            }
        }
        for entry in &self.files {
            owned_files::remove(self.directory.path(), entry)?;
        }
        drop(guards);
        for (relative, _) in self.directories.iter().rev() {
            if !relative.as_os_str().is_empty() {
                fs::remove_dir(self.directory.path().join(relative))
                    .context("暂存出现额外内容，已保留")?;
            }
        }
        self.directory.cleanup()
    }
}
fn known_file(path: &Path) -> bool {
    if path == Path::new("archive.part") {
        return true;
    }
    let Ok(path) = path.strip_prefix("database") else {
        return false;
    };
    if [
        "history.db",
        "key.bin",
        "history.db-journal",
        "history.db-wal",
        "history.db-shm",
    ]
    .iter()
    .any(|name| path == Path::new(name))
    {
        return true;
    }
    let Ok(hash) = path.strip_prefix("data/blobs") else {
        return false;
    };
    hash.to_str().is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
fn nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("系统随机数不可用"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
#[cfg(test)]
mod tests;
