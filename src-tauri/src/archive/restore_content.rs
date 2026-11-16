//! 认证清单之后的受控恢复，记录本次成功创建的条目以供任务层清理。
use super::*;
use crate::archive::control::Control;
use std::path::PathBuf;

pub(crate) struct PreparedRestore {
    cipher: Cipher,
    master: Zeroizing<[u8; 32]>,
    entries: Vec<Entry>,
    owned: Vec<PathBuf>,
    created_root: bool,
}
impl PreparedRestore {
    pub(crate) fn new(input: &mut impl Read, password: &[u8]) -> Result<Self> {
        let mut magic = [0; 8];
        input.read_exact(&mut magic)?;
        if &magic != BUNDLE_MAGIC {
            bail!("不支持的快照容器版本");
        }
        let mut header = [0; HEADER_SIZE];
        input.read_exact(&mut header)?;
        let mut cipher = Cipher::new(header, password)?;
        let secret = cipher.read_block(input)?;
        if secret.len() != 40 || &secret[..8] != CONTENT_MAGIC {
            bail!("快照主密钥记录无效");
        }
        let mut master = Zeroizing::new([0; 32]);
        master.copy_from_slice(&secret[8..]);
        let entries: Vec<Entry> = serde_json::from_slice(&cipher.read_block(input)?)?;
        validate(&entries)?;
        Ok(Self {
            cipher,
            master,
            entries,
            owned: Vec::new(),
            created_root: false,
        })
    }
    pub(crate) fn bytes(&self) -> u64 {
        self.entries.iter().map(|entry| entry.bytes).sum()
    }
    pub(crate) fn required_bytes(&self) -> u64 {
        // 设置过滤可能产生回滚日志，额外预留一份数据库大小。
        self.bytes()
            + self
                .entries
                .iter()
                .filter(|entry| entry.name == "history.db")
                .map(|entry| entry.bytes)
                .sum::<u64>()
    }
    pub(crate) fn extract(
        &mut self,
        input: &mut impl Read,
        target: &Path,
        control: &Control,
    ) -> Result<()> {
        control.check()?;
        fs::create_dir(target).context("恢复目标必须是尚不存在的独立目录")?;
        self.created_root = true;
        fs::create_dir_all(target.join("data/blobs"))?;
        for index in 0..self.entries.len() {
            self.read_entry(index, input, target, control)?;
        }
        control.check()?;
        if !self.cipher.read_block(input)?.is_empty() {
            bail!("快照结束标记无效");
        }
        require_eof(input)?;
        control.verifying()?;
        self.verify_and_install(target, control)
    }
    fn read_entry(
        &mut self,
        index: usize,
        input: &mut impl Read,
        target: &Path,
        control: &Control,
    ) -> Result<()> {
        control.check()?;
        let entry = &self.entries[index];
        let path = target.join(&entry.name);
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        self.owned.push(path);
        let mut remaining = entry.bytes;
        let mut digest = Sha256::new();
        while remaining > 0 {
            control.check()?;
            let block = self.cipher.read_block(input)?;
            if block.len() as u64 != remaining.min(CHUNK_SIZE as u64) {
                bail!("快照条目长度不一致");
            }
            output.write_all(&block)?;
            digest.update(&block);
            remaining -= block.len() as u64;
            control.advance(block.len() as u64)?;
        }
        if format!("{:x}", digest.finalize()) != entry.hash {
            bail!("快照条目校验失败");
        }
        output.sync_all()?;
        Ok(())
    }
    fn verify_and_install(&mut self, target: &Path, control: &Control) -> Result<()> {
        crate::storage::Storage::verify_snapshot_with_check(target, &self.master, || {
            control.check()
        })?;
        for name in [
            "history.db-journal",
            "history.db-wal",
            "history.db-shm",
            "key.bin",
        ] {
            let path = target.join(name);
            if path.try_exists()? {
                bail!("恢复暂存区出现非本任务文件");
            }
            self.owned.push(path);
        }
        control.check()?;
        crate::storage::Storage::prepare_portable_snapshot(target, &self.master)?;
        control.check()?;
        crate::platform::install_master_key(target, &self.master)?;
        control.check()
    }
    pub(crate) fn cleanup(&self, target: &Path) -> Result<()> {
        if !self.created_root {
            return Ok(());
        }
        self.validate_cleanup_paths(target)?;
        for path in &self.owned {
            if !path.try_exists()? {
                continue;
            }
            if !crate::data_directory::plain_metadata(path)?.is_file() {
                bail!("恢复暂存条目已被替换，停止清理");
            }
            fs::remove_file(path)?;
        }
        for relative in ["data/blobs", "data", ""] {
            let path = target.join(relative);
            if path.try_exists()? {
                fs::remove_dir(path).context("恢复暂存区有额外内容或无法清理，已保留")?;
            }
        }
        Ok(())
    }
    fn validate_cleanup_paths(&self, target: &Path) -> Result<()> {
        for relative in ["", "data", "data/blobs"] {
            let path = target.join(relative);
            if path.try_exists()? && !crate::data_directory::plain_metadata(&path)?.is_dir() {
                bail!("恢复暂存目录已被替换，停止清理");
            }
        }
        Ok(())
    }
}
