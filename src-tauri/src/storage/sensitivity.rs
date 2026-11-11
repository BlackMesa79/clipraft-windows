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
    let mut view: serde_json::Value = serde_json::from_str(&json)?;
    view["summary"] = data.summary.clone().into();
    view["sensitive_hits"] = serde_json::to_value(&data.hits)?;
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
        let mut updates = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            if !self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM items WHERE id=?1)",
                [id],
                |r| r.get::<_, bool>(0),
            )? {
                continue;
            }
            let clip = self.load(id)?;
            let text = clip.text().unwrap_or_else(|| search_text(&clip));
            let mut hits = sensitive::detect(&text, true);
            if hits.is_empty() {
                hits.push(Hit {
                    rule: "manual".into(),
                    range: 0..text.len(),
                });
            }
            let detection = Detection::new(&text, hits, ttl);
            updates.push((id, clip, detection));
        }
        let tx = self.conn.transaction()?;
        let now = unix_millis();
        for (id, clip, data) in &updates {
            if value {
                write_sensitive(&tx, id, data, Some(now.saturating_add(data.ttl)))?;
            } else {
                let json: String =
                    tx.query_row("SELECT view FROM items WHERE id=?1", [id], |r| r.get(0))?;
                let mut view: serde_json::Value = serde_json::from_str(&json)?;
                let search = search_text(clip);
                let summary = if clip.kind() == "files" {
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
                };
                view["summary"] = summary.chars().take(180).collect::<String>().into();
                view.as_object_mut().unwrap().remove("sensitive_hits");
                tx.execute("UPDATE items SET sensitive=-1,expires_at=NULL,search_text=?2,search_pinyin=?3,view=?4 WHERE id=?1",params![id,search,search::pinyin_text(&search),serde_json::to_string(&view)?])?;
            }
        }
        tx.commit()?;
        Ok(updates.len())
    }
    /// 详情与显示原文共用读取入口；仅显式 reveal 跳过脱敏。
    pub(crate) fn display_detail(&self, id: &str, reveal: bool) -> Result<crate::ItemDetail> {
        let (state, json): (i64, String) =
            self.conn
                .query_row("SELECT sensitive,view FROM items WHERE id=?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        let clip = self.load(id)?;
        if state != 1 || reveal {
            return Ok(crate::detail(&clip));
        }
        let text = clip.text().unwrap_or_default();
        let view: serde_json::Value = serde_json::from_str(&json)?;
        let hits: Vec<Hit> = view
            .get("sensitive_hits")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_else(|| {
                vec![Hit {
                    rule: "manual".into(),
                    range: 0..text.len(),
                }]
            });
        let masked = sensitive::redact(&text, &hits);
        let mut chars = masked.chars();
        let text = chars.by_ref().take(20000).collect();
        Ok(crate::ItemDetail {
            text,
            truncated: chars.next().is_some(),
            files: Vec::new(),
        })
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
