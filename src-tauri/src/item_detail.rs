pub(crate) mod read;
use crate::model;

#[derive(Clone, serde::Serialize)]
pub(crate) struct FileDetail {
    pub(crate) path: String,
    pub(crate) exists: Option<bool>,
}
#[derive(Clone, serde::Serialize)]
pub(crate) struct ItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) image: Option<Vec<u8>>,
    pub(crate) text: String,
    pub(crate) truncated: bool,
    pub(crate) files: Vec<FileDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rich: Option<crate::rich_preview::Preview>,
}
pub(crate) fn from_clip(clip: &model::Clip) -> ItemDetail {
    let full = clip.text().unwrap_or_default();
    let mut chars = full.chars();
    let text = chars.by_ref().take(20000).collect();
    let truncated = chars.next().is_some();
    let files = clip
        .get("CF_HDROP")
        .and_then(model::decode_hdrop)
        .unwrap_or_default()
        .into_iter()
        .map(|path| FileDetail {
            exists: std::path::Path::new(&path).try_exists().ok(),
            path,
        })
        .collect();
    ItemDetail {
        image: None,
        rich: crate::rich_preview::from_clip(clip),
        text,
        truncated,
        files,
    }
}
