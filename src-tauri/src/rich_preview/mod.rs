//! 只提取文字/图片顺序，不向 WebView 传递 HTML、脚本或源文件路径。
mod html;
mod images;
#[cfg(test)]
mod tests;
use crate::model::Clip;
use serde::Serialize;

const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 20_000;
const MAX_IMAGES: usize = 12;
const MAX_BLOCKS: usize = 128;
#[derive(Clone, Serialize)]
pub(crate) struct Preview {
    pub blocks: Vec<Block>,
    pub truncated: bool,
    pub linked_images: bool,
}
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Block {
    Text {
        text: String,
    },
    Image {
        src: Option<String>,
        message: Option<String>,
        #[serde(skip)]
        pending: Option<String>,
    },
}
pub(crate) fn from_clip(clip: &Clip) -> Option<Preview> {
    if !matches!(clip.kind(), "text" | "rich") {
        return None;
    }
    let bytes = clip.get("HTML Format")?;
    if bytes.len() > MAX_HTML_BYTES {
        return Some(unavailable("图文内容过大，当前只显示纯文本", clip));
    }
    let Some(fragment) = html::fragment(bytes) else {
        return Some(unavailable("图文片段无法解析，当前只显示纯文本", clip));
    };
    let preview = html::parse(fragment);
    preview
        .blocks
        .iter()
        .any(|block| matches!(block, Block::Image { .. }))
        .then_some(preview)
}
fn unavailable(message: &str, clip: &Clip) -> Preview {
    Preview {
        blocks: vec![
            Block::Text {
                text: clip
                    .text()
                    .unwrap_or_default()
                    .chars()
                    .take(MAX_TEXT_CHARS)
                    .collect(),
            },
            Block::Image {
                src: None,
                message: Some(message.into()),
                pending: None,
            },
        ],
        truncated: true,
        linked_images: false,
    }
}
impl Preview {
    pub fn resolve_images(&mut self) {
        let mut budget = images::Budget::default();
        for block in &mut self.blocks {
            resolve_block(block, &mut budget, &mut self.linked_images);
        }
    }
}
fn resolve_block(block: &mut Block, budget: &mut images::Budget, linked: &mut bool) {
    let Block::Image {
        src,
        message,
        pending,
    } = block
    else {
        return;
    };
    let Some(source) = pending.take() else {
        return;
    };
    *linked |= source.starts_with("file:");
    match images::load(&source, budget) {
        Ok(image) => *src = Some(image),
        Err(reason) => *message = Some(reason.into()),
    }
}
