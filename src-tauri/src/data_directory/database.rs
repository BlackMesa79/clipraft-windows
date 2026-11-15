use anyhow::{bail, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use zeroize::Zeroizing;

fn connect(root: &Path, flags: OpenFlags) -> Result<Connection> {
    if !root.join("history.db").is_file() || !root.join("key.bin").is_file() {
        bail!("数据目录缺少 history.db 或 key.bin");
    }
    super::files::plain_metadata(&root.join("key.bin"))?;
    super::files::plain_metadata(&root.join("history.db"))?;
    for name in ["history.db-wal", "history.db-shm"] {
        if root.join(name).try_exists()? {
            super::files::plain_metadata(&root.join(name))?;
        }
    }
    let key = crate::platform::master_key(root)?;
    let hex = Zeroizing::new(
        key.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    );
    let conn = open_connection(root, flags)?;
    conn.pragma_update(None, "key", format!("x'{}'", hex.as_str()))?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })?;
    Ok(conn)
}

fn open_connection(root: &Path, flags: OpenFlags) -> Result<Connection> {
    if flags.contains(OpenFlags::SQLITE_OPEN_READ_WRITE) {
        return Ok(Connection::open_with_flags(root.join("history.db"), flags)?);
    }
    // 冷快照已经完成 checkpoint；immutable 防止验证自身在目标生成 WAL/SHM，干扰中断恢复。
    let mut uri = tauri::Url::from_file_path(root.join("history.db"))
        .map_err(|()| anyhow::anyhow!("无法编码数据库路径"))?;
    uri.query_pairs_mut().append_pair("immutable", "1");
    Ok(Connection::open_with_flags(
        uri.as_str(),
        flags | OpenFlags::SQLITE_OPEN_URI,
    )?)
}
pub(super) fn checkpoint(root: &Path) -> Result<()> {
    let conn = connect(root, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let (busy, frames, checkpointed): (i64, i64, i64) =
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if busy != 0 || frames != checkpointed {
        bail!("数据库仍有连接正在使用，无法安全迁移 WAL");
    }
    conn.close().map_err(|(_, error)| error)?;
    Ok(())
}
pub(super) fn verify(root: &Path) -> Result<()> {
    let conn = connect(root, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let checks = conn
        .prepare("PRAGMA integrity_check")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if checks != ["ok"] {
        bail!("数据库完整性验证失败，请保留源数据");
    }
    let mut refs = conn.prepare("SELECT blob FROM formats WHERE blob IS NOT NULL UNION SELECT thumbnail FROM items WHERE thumbnail IS NOT NULL UNION SELECT preview FROM items WHERE preview IS NOT NULL")?;
    let hashes = refs.query_map([], |row| row.get::<_, String>(0))?;
    for hash in hashes {
        let hash = hash?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("数据库包含非法的对象引用");
        }
        if !super::files::plain_metadata(&root.join("data/blobs").join(&hash))?.is_file() {
            bail!("数据目录缺少被引用的图片或格式对象");
        }
    }
    Ok(())
}
