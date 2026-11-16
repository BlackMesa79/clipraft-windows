use super::{crypto, SnapshotFiles, Storage};
use anyhow::{bail, Context, Result};
use rusqlite::{params, TransactionBehavior};
use std::{fs, path::Path};
use zeroize::Zeroizing;

const COPY_BUFFER_BYTES: usize = 1024 * 1024;

impl Storage {
    /// 恢复前只读核验，不执行迁移、到期清理或设置副作用。
    pub fn verify_snapshot(root: &Path, master: &[u8; 32]) -> Result<()> {
        Self::verify_snapshot_with_check(root, master, || Ok(()))
    }
    pub(crate) fn verify_snapshot_with_check(
        root: &Path,
        master: &[u8; 32],
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        check()?;
        let conn = rusqlite::Connection::open_with_flags(
            root.join("history.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let hex = Zeroizing::new(
            master
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        );
        conn.pragma_update(None, "key", format!("x'{}'", hex.as_str()))?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != super::migrations::VERSION {
            bail!("快照数据库版本不兼容");
        }
        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            bail!("快照数据库完整性验证失败");
        }
        let key = crypto::derive(master)?;
        let mut query = conn.prepare("SELECT blob FROM formats WHERE blob IS NOT NULL UNION SELECT thumbnail FROM items WHERE thumbnail IS NOT NULL UNION SELECT preview FROM items WHERE preview IS NOT NULL")?;
        let hashes = query.query_map([], |row| row.get::<_, String>(0))?;
        for hash in hashes {
            check()?;
            let hash = hash?;
            validate_hash(&hash)?;
            let path = root.join("data/blobs").join(&hash);
            if fs::metadata(&path)?.len()
                > crypto::MAX_PLAIN_BYTES as u64 + crypto::BLOB_OVERHEAD + 8
            {
                bail!("快照对象超过大小上限");
            }
            let encrypted = fs::read(path)?;
            let plain = Zeroizing::new(crypto::decrypt(&key, &hash, &encrypted)?);
            if crate::model::digest(&plain) != hash {
                bail!("快照对象内容哈希不一致");
            }
        }
        check()
    }
    /// 调用方须持续持有存储互斥锁；目标必须是尚不存在的独立目录。
    /// 失败时保留不完整目标供调用方按任务所有权处理，不改动源数据。
    pub fn create_snapshot(&mut self, target: &Path, master: &[u8; 32]) -> Result<()> {
        self.create_snapshot_checked(target, master, || Ok(()))
    }
    pub(crate) fn create_snapshot_checked(
        &mut self,
        target: &Path,
        master: &[u8; 32],
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        self.create_snapshot_owned(&mut SnapshotFiles::new(target), master, check)
    }
    pub(crate) fn create_snapshot_owned(
        &mut self,
        files: &mut SnapshotFiles,
        master: &[u8; 32],
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        check()?;
        if *crypto::derive(master)? != *self.key {
            bail!("快照密钥与当前存储不匹配");
        }
        files.create()?;
        let destination = files.root().join("history.db");
        let destination = destination.to_str().context("快照路径不是有效 Unicode")?;
        let key = Zeroizing::new(format!(
            "x'{}'",
            master
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ));
        self.conn.execute(
            "ATTACH DATABASE ?1 AS snapshot KEY ?2",
            params![destination, key.as_str()],
        )?;
        let result = self.copy_snapshot(files, &check);
        let detached = self.conn.execute_batch("DETACH DATABASE snapshot");
        if let Err(error) = detached {
            return Err(result
                .err()
                .unwrap_or_else(|| anyhow::anyhow!("快照复制已完成"))
                .context(format!("快照数据库关闭失败：{error}")));
        }
        result?;
        fs::OpenOptions::new()
            .write(true)
            .open(files.root().join("history.db"))?
            .sync_all()?;
        Ok(())
    }

    pub(crate) fn snapshot_bytes(&self, check: impl Fn() -> Result<()>) -> Result<u64> {
        let pages: u64 = self
            .conn
            .pragma_query_value(None, "page_count", |row| row.get(0))?;
        let size: String = self
            .conn
            .pragma_query_value(None, "cipher_page_size", |row| row.get(0))?;
        let size: u64 = size.parse().context("SQLCipher页大小无效")?;
        // 加密导出与白名单过滤可能产生回滚日志，按两份数据库页预算。
        let mut bytes = pages
            .checked_mul(size)
            .and_then(|value| value.checked_mul(2))
            .context("快照数据库大小溢出")?;
        let mut query = self.conn.prepare("SELECT blob FROM formats WHERE blob IS NOT NULL UNION SELECT thumbnail FROM items WHERE thumbnail IS NOT NULL UNION SELECT preview FROM items WHERE preview IS NOT NULL")?;
        let hashes = query.query_map([], |row| row.get::<_, String>(0))?;
        for hash in hashes {
            check()?;
            let hash = hash?;
            validate_hash(&hash)?;
            let metadata = object_metadata(&self.blobs.join(hash))?;
            bytes = bytes.checked_add(metadata.len()).context("快照大小溢出")?;
        }
        check()?;
        Ok(bytes)
    }

    fn copy_snapshot(
        &mut self,
        files: &mut SnapshotFiles,
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        // 同一事务固定数据库视图，存储互斥锁保护对象免于本进程清理。
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check()?;
        tx.query_row("SELECT sqlcipher_export('snapshot')", [], |_| Ok(()))?;
        check()?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        tx.pragma_update(Some("snapshot"), "user_version", version)?;
        let hashes = {
            let mut query = tx.prepare("SELECT blob FROM formats WHERE blob IS NOT NULL UNION SELECT thumbnail FROM items WHERE thumbnail IS NOT NULL UNION SELECT preview FROM items WHERE preview IS NOT NULL")?;
            let hashes = query.query_map([], |row| row.get::<_, String>(0))?;
            hashes.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for hash in hashes {
            check()?;
            copy_object(&self.blobs, files, &hash, check)?;
        }
        check()?;
        tx.commit()?;
        Ok(())
    }
}

fn copy_object(
    source: &Path,
    files: &mut SnapshotFiles,
    hash: &str,
    check: &impl Fn() -> Result<()>,
) -> Result<()> {
    validate_hash(hash)?;
    let path = source.join(hash);
    object_metadata(&path)?;
    let mut input = fs::File::open(path)?;
    let mut output = files.create_blob(hash)?;
    use std::io::{Read, Write};
    let mut buffer = vec![0; COPY_BUFFER_BYTES];
    loop {
        check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
    }
    output.sync_all()?;
    Ok(())
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("快照包含非法对象引用");
    }
    Ok(())
}

fn object_metadata(path: &Path) -> Result<fs::Metadata> {
    let metadata = plain_metadata(path)?;
    if !metadata.is_file() {
        bail!("快照对象不是普通文件");
    }
    Ok(metadata)
}

pub(super) fn plain_metadata(path: &Path) -> Result<fs::Metadata> {
    use std::os::windows::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_attributes() & 0x400 != 0 {
        bail!("快照路径包含重解析点");
    }
    Ok(metadata)
}
