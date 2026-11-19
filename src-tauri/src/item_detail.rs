pub(crate) mod read;
use crate::model;

#[derive(Clone, serde::Serialize)]
pub(crate) struct FileDetail {
    pub(crate) path: String,
    pub(crate) exists: Option<bool>,
    pub(crate) is_directory: Option<bool>,
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
        .map(file_detail)
        .collect();
    ItemDetail {
        image: None,
        rich: crate::rich_preview::from_clip(clip),
        text,
        truncated,
        files,
    }
}

fn file_detail(path: String) -> FileDetail {
    let metadata = std::fs::metadata(&path);
    let exists = match &metadata {
        Ok(_) => Some(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    };
    FileDetail {
        is_directory: metadata.ok().map(|entry| entry.is_dir()),
        path,
        exists,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_distinguishes_directory_file_and_missing_target() {
        let root = std::env::temp_dir().join(format!(
            "clipraft-detail-{}-{}",
            std::process::id(),
            crate::storage::unix_millis()
        ));
        std::fs::create_dir(&root).unwrap();
        let file = root.join("entry.txt");
        std::fs::write(&file, "test").unwrap();
        let directory = file_detail(root.to_string_lossy().into_owned());
        let regular = file_detail(file.to_string_lossy().into_owned());
        let missing = file_detail(root.join("missing").to_string_lossy().into_owned());
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            (directory.exists, directory.is_directory),
            (Some(true), Some(true))
        );
        assert_eq!(
            (regular.exists, regular.is_directory),
            (Some(true), Some(false))
        );
        assert_eq!((missing.exists, missing.is_directory), (Some(false), None));
    }
}
