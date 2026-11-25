//! 快速面板的全量 ID 索引与有界安全视图；不读取 formats 或 blob。
use super::{item_view, touch_item, unix_millis, Storage};
use crate::{model::ItemView, panel_session::MAX_NEIGHBORHOOD_ITEMS};
use anyhow::{bail, Result};
use rusqlite::OptionalExtension;
use std::collections::HashSet;

const ID_QUERY_BATCH_SIZE: usize = 400;

impl Storage {
    pub fn panel_id_order(&self) -> Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare("SELECT id FROM items WHERE expires_at IS NULL OR expires_at>?1 ORDER BY pinned DESC,touched DESC,id ASC")?;
        let rows = statement.query_map([unix_millis()], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 调用方必须等待所有批次成功；任意失败不返回部分有效性集合。
    pub fn existing_panel_ids(&self, ids: &[&str]) -> Result<HashSet<String>> {
        let mut existing = HashSet::new();
        let now = unix_millis();
        for batch in ids.chunks(ID_QUERY_BATCH_SIZE) {
            let placeholders = vec!["?"; batch.len()].join(",");
            let mut statement = self.conn.prepare(&format!(
                "SELECT id FROM items WHERE id IN ({placeholders}) AND (expires_at IS NULL OR expires_at>?)"
            ))?;
            let mut parameters: Vec<rusqlite::types::Value> =
                batch.iter().map(|id| (*id).to_owned().into()).collect();
            parameters.push(now.into());
            let rows = statement.query_map(rusqlite::params_from_iter(parameters), |row| {
                row.get::<_, String>(0)
            })?;
            existing.extend(rows.collect::<rusqlite::Result<HashSet<_>>>()?);
        }
        Ok(existing)
    }

    /// 保持请求顺序，跳过已删除项；敏感打码复用统一 item_view 出口。
    pub fn panel_items(&self, ids: &[&str]) -> Result<Vec<ItemView>> {
        if ids.len() > MAX_NEIGHBORHOOD_ITEMS {
            bail!("快速面板邻域不能超过 {MAX_NEIGHBORHOOD_ITEMS} 条");
        }
        self.items_by_id(ids.iter().copied())
    }

    /// 主面板多选跨筛选保留，按 ID 刷新安全视图并剔除已删除或到期的记录。
    pub(crate) fn items_by_id<'a>(
        &self,
        ids: impl ExactSizeIterator<Item = &'a str>,
    ) -> Result<Vec<ItemView>> {
        let mut items = Vec::with_capacity(ids.len());
        let now = unix_millis();
        let mut statement = self.conn.prepare_cached("SELECT view,id,kind,created,used_at,size,pinned,favorite,group_id,source_exe,source_title,sensitive,expires_at,thumbnail,preview,is_link FROM items WHERE id=?1 AND (expires_at IS NULL OR expires_at>?2)")?;
        for id in ids {
            if let Some(item) = statement
                .query_row(rusqlite::params![id, now], item_view)
                .optional()?
            {
                items.push(item);
            }
        }
        Ok(items)
    }

    pub(crate) fn load_for_paste(&self, id: &str) -> Result<crate::model::Clip> {
        let available: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM items WHERE id=?1 AND (expires_at IS NULL OR expires_at>?2))",
            rusqlite::params![id, unix_millis()], |row| row.get(0),
        )?;
        if !available {
            bail!("记录已删除或到期，请重新选择");
        }
        self.load(id)
    }

    pub(crate) fn touch_many(&mut self, ids: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        let now = unix_millis();
        for id in ids {
            touch_item(&tx, id, now)?;
        }
        tx.commit()?;
        Ok(())
    }
}
