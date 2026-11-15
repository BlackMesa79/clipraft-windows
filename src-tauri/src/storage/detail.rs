//! 普通详情与显式原文分开；两者保持同一记录读取和载荷加载顺序。
use super::Storage;
use crate::{
    item_detail::{self, ItemDetail},
    model::Clip,
    sensitive::{self, Hit},
};
use anyhow::Result;

struct StoredDetail {
    state: i64,
    view: String,
    clip: Clip,
}

impl Storage {
    pub(crate) fn display_detail(&self, id: &str) -> Result<ItemDetail> {
        let protected_image: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM items WHERE id=?1 AND sensitive=1 AND (kind='image' OR (kind='files' AND (preview IS NOT NULL OR thumbnail IS NOT NULL))))",
            [id],
            |row| row.get(0),
        )?;
        if protected_image {
            let image = self
                .detail_image(id)?
                .map(|bytes| crate::thumbnail::mosaic(&bytes))
                .transpose()?;
            return Ok(ItemDetail {
                rich: None,
                image,
                text: String::new(),
                truncated: false,
                files: Vec::new(),
            });
        }
        let stored = self.read_detail(id)?;
        if stored.state != 1 {
            return Ok(item_detail::from_clip(&stored.clip));
        }
        masked_detail(&stored.clip, &stored.view)
    }

    pub(crate) fn reveal_detail(&self, id: &str) -> Result<ItemDetail> {
        let stored = self.read_detail(id)?;
        let mut detail = item_detail::from_clip(&stored.clip);
        if matches!(stored.clip.kind(), "image" | "files") {
            detail.image = self.detail_image(id)?;
        }
        Ok(detail)
    }

    fn detail_image(&self, id: &str) -> Result<Option<Vec<u8>>> {
        // 只按记录 ID 读取登记预览，普通缩略图协议仍拒绝敏感图片。
        let preview: Option<String> = self.conn.query_row(
            "SELECT COALESCE(preview,thumbnail) FROM items WHERE id=?1",
            [id],
            |row| row.get(0),
        )?;
        preview.map(|hash| self.read_blob(&hash)).transpose()
    }

    fn read_detail(&self, id: &str) -> Result<StoredDetail> {
        let (state, view): (i64, String) = self.conn.query_row(
            "SELECT sensitive,view FROM items WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let clip = self.load(id)?;
        Ok(StoredDetail { state, view, clip })
    }
}

fn masked_detail(clip: &Clip, json: &str) -> Result<ItemDetail> {
    let text = clip.text().unwrap_or_default();
    let view: serde_json::Value = serde_json::from_str(json)?;
    let hits: Vec<Hit> = view
        .get("sensitive_hits")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(|| {
            vec![Hit {
                rule: "manual".into(),
                range: 0..text.len(),
            }]
        });
    let masked = sensitive::redact(&text, &hits);
    let mut chars = masked.chars();
    let text = chars.by_ref().take(20000).collect();
    Ok(ItemDetail {
        rich: None,
        image: None,
        text,
        truncated: chars.next().is_some(),
        files: Vec::new(),
    })
}
