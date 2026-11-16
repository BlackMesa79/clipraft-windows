//! 归档任务独占的暂存根目录，不递归清理未知内容。
use super::staging_owner::{Kind, Owner, NAME};
use anyhow::{bail, Context, Result};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

pub(crate) struct Directory {
    root: PathBuf,
    guard: File,
    owner: Option<Owner>,
}
impl Directory {
    pub(crate) fn create(parent: &Path, kind: &str) -> Result<Self> {
        let owner_kind = Kind::parse(kind)?;
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| anyhow::anyhow!("系统随机数不可用"))?;
        let name: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let root = parent.join(format!(".clipraft-{kind}-{name}"));
        fs::create_dir(&root).context("无法独占创建归档暂存目录")?;
        match Self::owned(root.clone(), owner_kind) {
            Ok(directory) => Ok(directory),
            Err(error) => {
                fs::remove_dir(&root)
                    .with_context(|| format!("暂存初始化失败且目录无法清理：{error:#}"))?;
                Err(error)
            }
        }
    }
    pub(crate) fn open(root: PathBuf) -> Result<Self> {
        if !crate::data_directory::plain_metadata(&root)?.is_dir() {
            bail!("归档暂存根不是普通目录");
        }
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        // 不共享删除，阻止任务期间暂存根被重命名或替换。
        let guard = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(&root)?;
        Ok(Self {
            root,
            guard,
            owner: None,
        })
    }
    fn owned(root: PathBuf, kind: Kind) -> Result<Self> {
        let mut directory = Self::open(root)?;
        directory.owner = Some(Owner::create(&directory.root, kind)?);
        Ok(directory)
    }
    pub(crate) fn claim(root: &Path) -> Result<Option<Self>> {
        let mut directory = Self::open(root.to_path_buf())?;
        let Some(owner) = Owner::claim(root)? else {
            return Ok(None);
        };
        directory.owner = Some(owner);
        Ok(Some(directory))
    }
    pub(crate) fn is_empty(&self) -> Result<bool> {
        for entry in fs::read_dir(&self.root)? {
            if entry?.file_name() != NAME {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.root
    }
    pub(crate) fn cleanup(self) -> Result<()> {
        if self.owner.is_some() {
            if !self.is_empty()? {
                bail!("暂存目录仍有内容，所有权标记及内容已保留");
            }
            drop(self.owner);
            fs::remove_file(self.root.join(NAME)).context("无法删除暂存所有权标记")?;
        }
        drop(self.guard);
        fs::remove_dir(self.root).context("无法清理本任务暂存目录，额外内容已保留")
    }
}
