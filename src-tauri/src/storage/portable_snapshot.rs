//! 跨账户快照的设置边界；只修改独立快照，不应用运行时设置。
use super::Storage;
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use zeroize::Zeroizing;

// 新设置默认不迁移，必须在此显式评审；自启动始终由目标机器重新选择。
const PORTABLE_SETTINGS: &[&str] = &[
    super::background::REGULAR_KEY,
    super::background::COMPACT_KEY,
    "skin",
    "theme",
    "auto_check_updates",
    "backdrop_strength",
    "layout",
    "compact",
    "hide_on_blur",
    "select_first_on_show",
    "content_ignore_rules",
    "merge_paste_separator",
    "main_position",
    "pause_on_start",
    "max_items",
    "max_bytes",
    "max_days",
    "ignored_apps",
    "sensitive_detection",
    "sensitive_high_entropy",
    "sensitive_ttl_minutes",
    "main_hotkey",
    "quick_hotkey",
    "quick_visible_count",
];

impl Storage {
    /// 与原始备份分开：源设置不变，仅在独立加密副本中应用白名单。
    pub fn create_portable_snapshot(&mut self, target: &Path, master: &[u8; 32]) -> Result<()> {
        self.create_snapshot(target, master)?;
        Self::prepare_portable_snapshot(target, master)
    }

    /// 只能用于未发布的任务目录；不打开 Storage，避免启动清理与设置副作用。
    pub(crate) fn prepare_portable_snapshot(root: &Path, master: &[u8; 32]) -> Result<()> {
        let mut conn = Connection::open_with_flags(
            root.join("history.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        let hex = Zeroizing::new(
            master
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        );
        conn.pragma_update(None, "key", format!("x'{}'", hex.as_str()))?;
        let tx = conn.transaction()?;
        let keys = {
            let mut query = tx.prepare("SELECT key FROM settings")?;
            let rows = query.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for key in keys {
            if !PORTABLE_SETTINGS.contains(&key.as_str()) {
                tx.execute("DELETE FROM settings WHERE key=?1", [&key])?;
            }
        }
        tx.commit()?;
        conn.close().map_err(|(_, error)| error)?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(root.join("history.db"))?
            .sync_all()?;
        Ok(())
    }
}
