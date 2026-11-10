use super::Storage;
use crate::{diag, model};
use anyhow::{bail, Result};
use rusqlite::params;
use serde::Serialize;
use std::{collections::BTreeSet, fs, io::ErrorKind, time::Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub max_items: u32,
    pub max_bytes: u64,
    pub max_days: u32,
}
impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_items: 1000,
            max_bytes: 1024 * 1024 * 1024,
            max_days: 30,
        }
    }
}
/// 纯校验：只有 max_days 的 0 表示不限；SQLite 的 size 求和使用有符号整数。
pub fn validate_retention_policy(policy: &RetentionPolicy) -> Result<()> {
    if policy.max_items == 0 {
        bail!("最大条数必须大于 0");
    }
    if policy.max_bytes == 0 || policy.max_bytes > i64::MAX as u64 {
        bail!("最大空间必须在 1 到 i64::MAX 字节之间");
    }
    Ok(())
}

#[derive(Debug, Default, Serialize)]
pub struct DeleteReport {
    pub deleted_items: usize,
    // 仅供内存采集上下文失效判断，不扩展前端响应或诊断日志中的记录信息。
    #[serde(skip)]
    pub(crate) deleted_ids: Vec<String>,
    pub reclaimed_blobs: usize,
    pub failed_blobs: Vec<String>,
    pub vacuumed: bool,
    pub maintenance_errors: Vec<String>,
}
#[derive(Debug, Default)]
pub struct ScanReport {
    pub reclaimed_blobs: usize,
    pub removed_temps: usize,
    pub failed_files: Vec<String>,
}

impl Storage {
    fn blob_referenced(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM formats WHERE blob=?1)
                 OR EXISTS(SELECT 1 FROM items WHERE thumbnail=?1)
                 OR EXISTS(SELECT 1 FROM items WHERE preview=?1)",
            [id],
            |r| r.get(0),
        )?)
    }

    pub fn delete(&mut self, ids: &[String]) -> Result<DeleteReport> {
        let mut report = DeleteReport::default();
        let mut candidates = BTreeSet::new();
        let tx = self.conn.transaction()?;
        {
            // 逐 id 参数绑定，不受 SQLite IN 参数个数限制；重复 id 只计一次。
            let mut blobs = tx.prepare(
                "SELECT blob FROM formats WHERE item=?1 AND blob IS NOT NULL
                UNION SELECT thumbnail FROM items WHERE id=?1 AND thumbnail IS NOT NULL
                UNION SELECT preview FROM items WHERE id=?1 AND preview IS NOT NULL",
            )?;
            let mut formats = tx.prepare("DELETE FROM formats WHERE item=?1")?;
            let mut items = tx.prepare("DELETE FROM items WHERE id=?1")?;
            for id in ids {
                for blob in blobs.query_map([id], |r| r.get::<_, String>(0))? {
                    candidates.insert(blob?);
                }
                formats.execute([id])?;
                if items.execute([id])? > 0 {
                    report.deleted_items += 1;
                    report.deleted_ids.push(id.clone());
                }
            }
        }
        tx.commit()?;
        // 提交后的任何维护错误不能伪装成删除失败；文件遗留由下次启动扫描重试。
        for blob in candidates {
            if !model::is_hash(&blob) {
                continue;
            }
            match self.blob_referenced(&blob) {
                Ok(false) => match fs::remove_file(self.blobs.join(&blob)) {
                    Ok(()) => report.reclaimed_blobs += 1,
                    Err(e) if e.kind() == ErrorKind::NotFound => (),
                    Err(_) => report.failed_blobs.push(blob),
                },
                Ok(true) => (),
                Err(e) => {
                    report.failed_blobs.push(blob);
                    report
                        .maintenance_errors
                        .push(format!("blob 引用检查失败：{e}"));
                }
            }
        }
        if report.deleted_items > 200 {
            match self.vacuum() {
                Ok(()) => report.vacuumed = true,
                Err(e) => report.maintenance_errors.push(format!("VACUUM 失败：{e}")),
            }
        }
        diag::write(format!(
            "delete items={} blobs={} failed_blobs={} vacuumed={} maintenance_errors={:?}",
            report.deleted_items,
            report.reclaimed_blobs,
            report.failed_blobs.len(),
            report.vacuumed,
            report.maintenance_errors
        ));
        Ok(report)
    }

    /// 只遍历直接子级普通文件；不跟随符号链接、不递归、不触碰其他命名。
    pub fn scan_orphan_blobs(&self) -> Result<ScanReport> {
        let mut report = ScanReport::default();
        for entry in fs::read_dir(&self.blobs)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let temp = name.ends_with(".tmp");
            if !temp && (!model::is_hash(name) || self.blob_referenced(name)?) {
                continue;
            }
            match fs::remove_file(entry.path()) {
                Ok(()) if temp => report.removed_temps += 1,
                Ok(()) => report.reclaimed_blobs += 1,
                Err(e) if e.kind() == ErrorKind::NotFound => (),
                Err(_) => report.failed_files.push(name.to_owned()),
            }
        }
        diag::write(format!(
            "orphan_scan blobs={} temps={} failed={}",
            report.reclaimed_blobs,
            report.removed_temps,
            report.failed_files.len()
        ));
        Ok(report)
    }

    pub fn cleanup(&mut self, policy: &RetentionPolicy, now: i64) -> Result<DeleteReport> {
        validate_retention_policy(policy)?;
        let cutoff = now.saturating_sub(i64::from(policy.max_days) * 86_400_000);
        let ids = {
            // newest 是全局最新一条（并列时 id 升序兜底），优先于到期与容量限制。
            // 先剔除必删项，再从新到旧累计保留容量，得到应从旧到新删除的前缀。
            let mut stmt = self.conn.prepare("WITH
                newest AS (SELECT id FROM items ORDER BY touched DESC,id ASC LIMIT 1),
                mandatory AS (
                    SELECT id FROM items WHERE id NOT IN newest AND (
                        expires_at <= ?1 OR (?2 > 0 AND used_at < ?3
                            AND pinned=0 AND favorite=0 AND group_id IS NULL))),
                ranked AS (
                    SELECT id,touched,
                        ROW_NUMBER() OVER (ORDER BY touched DESC,id ASC) AS item_count,
                        SUM(size) OVER (ORDER BY touched DESC,id ASC ROWS UNBOUNDED PRECEDING) AS bytes
                    FROM items WHERE pinned=0 AND favorite=0 AND group_id IS NULL
                        AND id NOT IN mandatory),
                victims AS (
                    SELECT id FROM mandatory UNION
                    SELECT id FROM ranked WHERE (item_count > ?4 OR bytes > ?5) AND id NOT IN newest)
                SELECT id FROM items WHERE id IN victims ORDER BY touched ASC,id DESC")?;
            let rows = stmt.query_map(
                params![
                    now,
                    policy.max_days,
                    cutoff,
                    policy.max_items,
                    policy.max_bytes as i64
                ],
                |r| r.get::<_, String>(0),
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        self.delete(&ids)
    }

    fn vacuum(&self) -> Result<()> {
        let started = Instant::now();
        let result = self.conn.execute_batch("VACUUM");
        diag::write(format!(
            "vacuum elapsed_ms={} ok={}",
            started.elapsed().as_millis(),
            result.is_ok()
        ));
        result.map_err(Into::into)
    }

    /// 只能在事务外执行；严格大于 25% 才整理。
    pub fn vacuum_on_startup(&self) -> Result<bool> {
        let pages: u64 = self
            .conn
            .pragma_query_value(None, "page_count", |r| r.get(0))?;
        let free: u64 = self
            .conn
            .pragma_query_value(None, "freelist_count", |r| r.get(0))?;
        if pages > 0 && free > pages / 4 {
            self.vacuum()?;
            return Ok(true);
        }
        Ok(false)
    }
}
