use super::*;
use crate::{
    sensitive::{self, Hit},
    settings::Settings,
};

pub(super) struct Detection {
    hits: Vec<Hit>,
    pub(super) summary: String,
    ttl: i64,
}
impl Detection {
    pub(super) fn for_clip(clip: &Clip, settings: &Settings) -> Option<Self> {
        if !settings.sensitive_detection || !matches!(clip.kind(), "text" | "rich") {
            return None;
        }
        let text = clip.text()?;
        let hits = sensitive::detect(&text, settings.sensitive_high_entropy);
        if hits.is_empty() {
            return None;
        }
        Some(Self::new(&text, hits, settings.sensitive_ttl_minutes))
    }
    fn new(text: &str, mut hits: Vec<Hit>, ttl: u32) -> Self {
        // 已标记记录的未扫描后缀不发送到前端；不把扫描上限当作输出旁路。
        if text.len() > sensitive::SCAN_LIMIT {
            let mut end = sensitive::SCAN_LIMIT;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            if hits.last().is_none_or(|h| h.range.end <= end) {
                hits.push(Hit {
                    rule: "unscanned_tail".into(),
                    range: end..text.len(),
                });
            }
        }
        Self {
            summary: sensitive::redact(text, &hits).chars().take(180).collect(),
            hits,
            ttl: i64::from(ttl) * 60_000,
        }
    }
}
fn write_sensitive(
    conn: &Connection,
    id: &str,
    data: &Detection,
    expiry: Option<i64>,
) -> Result<()> {
    let json: String = conn.query_row("SELECT view FROM items WHERE id=?1", [id], |r| r.get(0))?;
    let mut view: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&json).context("已存记录视图不是有效 JSON 对象")?;
    view.insert("summary".into(), data.summary.clone().into());
    view.insert("sensitive_hits".into(), serde_json::to_value(&data.hits)?);
    conn.execute("UPDATE items SET sensitive=1,expires_at=?2,search_text='',search_pinyin='',view=?3 WHERE id=?1",params![id,expiry,serde_json::to_string(&view)?])?;
    for hit in &data.hits {
        crate::diag::write(format!("sensitive id={id} rule={}", hit.rule));
    }
    Ok(())
}
pub(super) fn apply_detection(
    conn: &Connection,
    id: &str,
    data: &Detection,
    now: i64,
) -> Result<()> {
    let (state, expiry, pinned, favorite, group): (i64, Option<i64>, bool, bool, Option<i64>) =
        conn.query_row(
            "SELECT sensitive,expires_at,pinned,favorite,group_id FROM items WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
    if state == -1 {
        return Ok(());
    }
    // 已经保留过的敏感记录不因取消保护后再次复制而恢复倒计时。
    let next = if pinned || favorite || group.is_some() || (state == 1 && expiry.is_none()) {
        None
    } else {
        Some(now.saturating_add(data.ttl))
    };
    write_sensitive(conn, id, data, next)
}
impl Storage {
    pub fn set_sensitive(&mut self, ids: &[String], value: bool, ttl: u32) -> Result<usize> {
        if !(1..=1440).contains(&ttl) {
            bail!("敏感内容保留分钟数必须为 1–1440");
        }
        let updates = self.prepare_sensitivity(ids, ttl)?;
        let tx = self.conn.transaction()?;
        let now = unix_millis();
        if value {
            mark_sensitive_batch(&tx, &updates, now)?;
        } else {
            unmark_sensitive_batch(&tx, &updates)?;
        }
        tx.commit()?;
        Ok(updates.len())
    }

    fn prepare_sensitivity<'a>(
        &self,
        ids: &'a [String],
        ttl: u32,
    ) -> Result<Vec<PreparedSensitivity<'a>>> {
        let mut updates = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            if !self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM items WHERE id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )? {
                continue;
            }
            updates.push(PreparedSensitivity::new(id, self.load(id)?, ttl));
        }
        Ok(updates)
    }
    pub fn next_expiry(&self) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT MIN(expires_at) FROM items", [], |r| r.get(0))?)
    }
    pub fn delete_expired(&mut self, now: i64) -> Result<DeleteReport> {
        let ids = self
            .conn
            .prepare("SELECT id FROM items WHERE expires_at<=?1 ORDER BY expires_at,id")?
            .query_map([now], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        self.delete(&ids)
    }
}

struct PreparedSensitivity<'a> {
    id: &'a str,
    clip: Clip,
    detection: Detection,
}

impl<'a> PreparedSensitivity<'a> {
    fn new(id: &'a str, clip: Clip, ttl: u32) -> Self {
        let text = clip.text().unwrap_or_else(|| search_text(&clip));
        let mut hits = sensitive::detect(&text, true);
        if hits.is_empty() {
            hits.push(Hit {
                rule: "manual".into(),
                range: 0..text.len(),
            });
        }
        Self {
            id,
            detection: Detection::new(&text, hits, ttl),
            clip,
        }
    }
}

fn mark_sensitive_batch(
    conn: &Connection,
    updates: &[PreparedSensitivity<'_>],
    now: i64,
) -> Result<()> {
    for update in updates {
        let detection = &update.detection;
        write_sensitive(
            conn,
            update.id,
            detection,
            Some(now.saturating_add(detection.ttl)),
        )?;
    }
    Ok(())
}

fn unmark_sensitive_batch(conn: &Connection, updates: &[PreparedSensitivity<'_>]) -> Result<()> {
    for update in updates {
        restore_sensitive(conn, update.id, &update.clip)?;
    }
    Ok(())
}

fn restore_sensitive(conn: &Connection, id: &str, clip: &Clip) -> Result<()> {
    let json: String =
        conn.query_row("SELECT view FROM items WHERE id=?1", [id], |row| row.get(0))?;
    let mut view: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&json).context("已存记录视图不是有效 JSON 对象")?;
    let search = search_text(clip);
    view.insert(
        "summary".into(),
        restored_summary(clip)
            .chars()
            .take(180)
            .collect::<String>()
            .into(),
    );
    view.remove("sensitive_hits");
    conn.execute("UPDATE items SET sensitive=-1,expires_at=NULL,search_text=?2,search_pinyin=?3,view=?4 WHERE id=?1",params![id,search,search::pinyin_text(&search),serde_json::to_string(&view)?])?;
    Ok(())
}

fn restored_summary(clip: &Clip) -> String {
    if clip.kind() == "files" {
        clip.get("CF_HDROP")
            .and_then(model::decode_hdrop)
            .unwrap_or_default()
            .iter()
            .map(|p| p.rsplit(['\\', '/']).next().unwrap_or(p))
            .collect::<Vec<_>>()
            .join("、")
    } else {
        clip.text().unwrap_or_else(|| {
            if clip.kind() == "image" {
                "图片".into()
            } else {
                "富文本".into()
            }
        })
    }
}
