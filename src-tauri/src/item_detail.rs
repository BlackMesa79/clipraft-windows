use crate::model;

#[derive(Clone, serde::Serialize)]
pub(crate) struct FileDetail {
    pub(crate) path: String,
    pub(crate) exists: Option<bool>,
}
#[derive(Clone, serde::Serialize)]
pub(crate) struct ItemDetail {
    pub(crate) text: String,
    pub(crate) truncated: bool,
    pub(crate) files: Vec<FileDetail>,
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
        text,
        truncated,
        files,
    }
}
