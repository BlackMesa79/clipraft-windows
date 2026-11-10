use super::{crypto, display_json, search_text, unix_millis, Storage};
use crate::model::{self, Clip, FormatData, ItemView};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const VERSION: i64 = 3;
const V0: &str = "CREATE TABLE items(id TEXT PRIMARY KEY, view TEXT NOT NULL, touched INTEGER NOT NULL);
CREATE TABLE formats(item TEXT NOT NULL REFERENCES items(id), name TEXT NOT NULL, inline BLOB, blob TEXT, PRIMARY KEY(item,name));";
const V1: &str = "CREATE TABLE groups(id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort INTEGER NOT NULL, created INTEGER NOT NULL);
CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
ALTER TABLE items ADD COLUMN kind TEXT NOT NULL DEFAULT 'text';
ALTER TABLE items ADD COLUMN created INTEGER NOT NULL DEFAULT 0;
ALTER TABLE items ADD COLUMN used_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE items ADD COLUMN size INTEGER NOT NULL DEFAULT 0;
ALTER TABLE items ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE items ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE items ADD COLUMN group_id INTEGER REFERENCES groups(id);
ALTER TABLE items ADD COLUMN source_exe TEXT;
ALTER TABLE items ADD COLUMN source_title TEXT;
ALTER TABLE items ADD COLUMN sensitive INTEGER NOT NULL DEFAULT 0;
ALTER TABLE items ADD COLUMN expires_at INTEGER;
ALTER TABLE items ADD COLUMN search_text TEXT NOT NULL DEFAULT '';
ALTER TABLE items ADD COLUMN thumbnail TEXT;
ALTER TABLE items ADD COLUMN preview TEXT;
CREATE INDEX items_order ON items(pinned DESC,touched DESC,id ASC);
CREATE INDEX items_created ON items(created);
CREATE INDEX items_group ON items(group_id);
CREATE INDEX items_thumbnail ON items(thumbnail);
CREATE INDEX items_preview ON items(preview);";

/// checkpoint 返回成功并不代表完成；busy 或残留 WAL 都必须中止升级。
fn backup(conn: &Connection, root: &Path, version: i64) -> Result<PathBuf> {
    let (busy, frames, checkpointed): (i64, i64, i64) =
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    if busy != 0 || frames != 0 || checkpointed != 0 {
        bail!("数据库备份失败：数据库正被占用，请关闭其他实例后重试，原数据未改动");
    }
    let base = format!("history.db.bak-v{version}");
    let mut attempt = 0u64;
    loop {
        let name = if attempt == 0 {
            base.clone()
        } else {
            format!("{base}-{}-{attempt}", unix_millis())
        };
        let path = root.join(name);
        // create_new 避免覆盖已有备份，包括同一毫秒内的重试。
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let result = (|| -> io::Result<()> {
                    io::copy(&mut fs::File::open(root.join("history.db"))?, &mut file)?;
                    file.sync_all()
                })();
                result.with_context(|| {
                    format!(
                        "数据库备份失败，原数据未改动；未完成的备份位于 {}",
                        path.display()
                    )
                })?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => attempt += 1,
            Err(error) => return Err(error).context("数据库备份失败，原数据未改动"),
        }
    }
}

pub(super) fn upgrade(
    storage: &mut Storage,
    root: &Path,
    mut before_commit: impl FnMut(&Transaction<'_>) -> Result<()>,
) -> Result<()> {
    let mut version: i64 = storage
        .conn
        .pragma_query_value(None, "user_version", |r| r.get(0))?;
    if !(0..=VERSION).contains(&version) {
        bail!("数据库版本 {version} 不受支持（当前最高 {VERSION}），请使用匹配版本，原数据未改动");
    }
    if version == VERSION {
        return Ok(());
    }
    let has_items: bool = storage.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='items')",
        [],
        |r| r.get(0),
    )?;
    let backup = if has_items {
        // 独占模式跨事务持锁，封住 checkpoint 与文件复制之间的其他实例写入窗口。
        storage
            .conn
            .execute_batch("PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE; COMMIT;")
            .context("数据库升级未开始：数据库被占用，无法取得独占锁，原数据未改动")?;
        Some(backup(&storage.conn, root, version)?)
    } else {
        None
    };
    while version < VERSION {
        let tx = storage
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("数据库升级未开始：无法取得写锁，原数据未改动")?;
        let result = (|| -> Result<()> {
            match version {
                0 => {
                    if !has_items {
                        tx.execute_batch(V0)?;
                    }
                    migrate_v1(&tx, &storage.blobs, &storage.key)?;
                }
                1 => migrate_v2(&tx)?,
                2 => tx.execute_batch("CREATE INDEX formats_blob ON formats(blob);
                    CREATE INDEX items_touched ON items(touched DESC,id ASC);
                    CREATE INDEX items_expiry ON items(expires_at) WHERE expires_at IS NOT NULL;
                    CREATE INDEX items_retention ON items(used_at,touched) WHERE pinned=0 AND favorite=0 AND group_id IS NULL;")?,
                _ => bail!("缺少从版本 {version} 升级的迁移步骤"),
            }
            tx.pragma_update(None, "user_version", version + 1)?;
            before_commit(&tx)?;
            // 显式 COMMIT，使提交错误也走下面显式回滚和中文错误路径。
            tx.execute_batch("COMMIT")?;
            Ok(())
        })();
        if let Err(error) = result {
            let location = backup
                .as_ref()
                .map(|p| format!("备份位于 {}", p.display()))
                .unwrap_or_else(|| "这是新数据库，没有旧数据需要备份".into());
            match tx.rollback() {
                Ok(()) => bail!("数据库升级失败，已回滚，原数据未改动，{location}。原因：{error}"),
                Err(rollback) => bail!("数据库升级失败，回滚状态无法确认，请保留原库及备份，{location}。原因：{error}；回滚错误：{rollback}"),
            }
        }
        version += 1;
    }
    if has_items {
        storage.conn.execute_batch(
            "PRAGMA locking_mode=NORMAL; BEGIN; SELECT count(*) FROM sqlite_master; COMMIT;",
        )?;
    }
    Ok(())
}

fn migrate_v1(tx: &Transaction<'_>, blobs: &Path, key: &[u8; 32]) -> Result<()> {
    tx.execute_batch(V1)?;
    let now = unix_millis();
    let rows = {
        let mut items = tx.prepare("SELECT id,view FROM items")?;
        let rows = items.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut formats = tx.prepare("SELECT name,length(inline),blob FROM formats WHERE item=?1")?;
    for (id, json) in rows {
        let mut view: ItemView = serde_json::from_str(&json).context("旧记录展示数据无法解析")?;
        let mut size = 0i64;
        let mut clip = Clip::default();
        for row in formats.query_map([&id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })? {
            let (name, inline_length, blob) = row?;
            let blob_path = blob
                .as_deref()
                .filter(|b| model::is_hash(b))
                .map(|b| blobs.join(b));
            // 此函数仅由 v0 → v1 调用：v0 旧库只含 CM01，故可减固定开销。
            // CM02 不适用该估算；新记录保存时直接统计明文，后续迁移不回填 size。
            // 统计外部格式只读取文件元数据，不解密，也不计缩略图/预览。
            let length = inline_length.unwrap_or_else(|| {
                blob_path
                    .as_ref()
                    .and_then(|p| fs::metadata(p).ok())
                    .map(|m| {
                        m.len()
                            .saturating_sub(crypto::BLOB_OVERHEAD)
                            .min(i64::MAX as u64) as i64
                    })
                    .unwrap_or(0)
            });
            size = size.saturating_add(length);
            // 搜索提取仅加载所需文本/路径；图片和 RTF 二进制不读取。
            if name == "CF_UNICODETEXT" || name == "CF_HDROP" {
                let bytes = if inline_length.is_some() {
                    tx.query_row(
                        "SELECT inline FROM formats WHERE item=?1 AND name=?2",
                        params![id, name],
                        |r| r.get::<_, Vec<u8>>(0),
                    )
                    .ok()
                } else {
                    blob_path.and_then(|p| fs::read(p).ok()).and_then(|b| {
                        crypto::decrypt(key, blob.as_deref().unwrap_or_default(), &b).ok()
                    })
                };
                if let Some(bytes) = bytes {
                    clip.formats.push(FormatData { name, bytes });
                }
            }
        }
        // kind 延续旧记录分类；富文本标记可以仅从格式名判断，无需读取 blob。
        if view.kind == "text"
            && !view.formats.iter().any(|n| n == "CF_UNICODETEXT")
            && view
                .formats
                .iter()
                .any(|n| n == "HTML Format" || n == "Rich Text Format")
        {
            view.kind = "rich".into();
        }
        let search = if view.kind == "image" {
            String::new()
        } else {
            search_text(&clip)
        };
        tx.execute("UPDATE items SET view=?2,kind=?3,created=?4,used_at=?4,size=?5,search_text=?6,thumbnail=?7,preview=?8 WHERE id=?1",
            params![id, display_json(&view)?, view.kind, now, size, search, view.thumbnail, view.preview])?;
    }
    Ok(())
}

fn migrate_v2(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "ALTER TABLE items ADD COLUMN dedup_key TEXT;
        CREATE INDEX items_dedup_key ON items(dedup_key);",
    )?;
    let ids = {
        let mut items = tx.prepare("SELECT id FROM items")?;
        let rows = items.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    // 仅读取所需内联格式；外置、缺失或损坏时保留旧 id，不读取图片/blob。
    let mut formats = tx.prepare(
        "SELECT name,inline FROM formats WHERE item=?1 AND name IN ('CF_HDROP','Preferred DropEffect')",
    )?;
    for id in ids {
        let clip = formats
            .query_map([&id], |r| {
                Ok(FormatData {
                    name: r.get(0)?,
                    bytes: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .ok()
            .map(|formats| Clip { formats });
        let dedup_key = clip
            .and_then(|c| c.file_dedup_key())
            .unwrap_or_else(|| id.clone());
        tx.execute(
            "UPDATE items SET dedup_key=?2 WHERE id=?1",
            params![id, dedup_key],
        )?;
    }
    Ok(())
}
