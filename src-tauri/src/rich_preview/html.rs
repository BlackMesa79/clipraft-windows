use super::{Block, Preview, MAX_BLOCKS, MAX_IMAGES, MAX_TEXT_CHARS};
use dom_query::{Document, NodeRef};
const MAX_NODES: usize = 20_000;
const MAX_DEPTH: usize = 64;

pub(super) fn fragment(bytes: &[u8]) -> Option<&str> {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).ok()?;
    let header_end = text.find('<').unwrap_or(text.len());
    let header = &text[..header_end];
    let range = match (
        offset(header, "StartFragment:"),
        offset(header, "EndFragment:"),
    ) {
        (Some(start), Some(end)) => Some((start, end)),
        _ => None,
    };
    if let Some((start, end)) = range {
        if start <= end && start >= header_end {
            return text.get(start..end);
        }
        return None;
    }
    if let Some(start) = text.find("<!--StartFragment-->") {
        let start = start + "<!--StartFragment-->".len();
        let end = text[start..].find("<!--EndFragment-->")? + start;
        return text.get(start..end);
    }
    if header.contains("StartFragment:") || header.contains("EndFragment:") {
        return None;
    }
    if header.starts_with("Version:") {
        return text.get(header_end..);
    }
    Some(text)
}
fn offset(header: &str, key: &str) -> Option<usize> {
    header.lines().find_map(|line| {
        line.strip_prefix(key)
            .and_then(|value| value.trim().parse().ok())
    })
}
struct Builder {
    preview: Preview,
    text: String,
    characters: usize,
    images: usize,
}
impl Builder {
    fn append(&mut self, text: &str) {
        for character in text.chars() {
            if self.characters >= MAX_TEXT_CHARS {
                self.preview.truncated = true;
                break;
            }
            self.text.push(character);
            self.characters += 1;
        }
    }
    fn newline(&mut self) {
        if !self.text.ends_with('\n') {
            self.append("\n");
        }
    }
    fn flush(&mut self) {
        let text = self.text.trim().to_owned();
        self.text.clear();
        if !text.is_empty() {
            self.preview.blocks.push(Block::Text { text });
        }
    }
    fn image(&mut self, node: NodeRef<'_>) {
        self.flush();
        if self.images >= MAX_IMAGES {
            self.preview.truncated = true;
            return;
        }
        self.images += 1;
        let source = node.attr("src").map(|src| src.to_string());
        self.preview.blocks.push(Block::Image {
            src: None,
            message: source.is_none().then(|| "图片缺少资源地址".into()),
            pending: source,
        });
    }
}
fn excluded(node: NodeRef<'_>, name: &str) -> bool {
    node.attr("hidden").is_some()
        || matches!(
            name,
            "script"
                | "style"
                | "head"
                | "iframe"
                | "object"
                | "embed"
                | "svg"
                | "math"
                | "template"
                | "noscript"
        )
}
fn block_element(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "section"
            | "article"
            | "li"
            | "tr"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "pre"
            | "hr"
    )
}
pub(super) fn parse(fragment: &str) -> Preview {
    let document = Document::from(fragment);
    let mut builder = Builder {
        preview: Preview {
            blocks: Vec::new(),
            truncated: false,
            linked_images: false,
        },
        text: String::new(),
        characters: 0,
        images: 0,
    };
    let mut stack = vec![Visit {
        node: document.root(),
        depth: 0,
        closing: false,
    }];
    let mut visited = 0;
    while let Some(visit_state) = stack.pop() {
        if visited >= MAX_NODES || builder.preview.blocks.len() >= MAX_BLOCKS - 2 {
            builder.preview.truncated = true;
            break;
        }
        visited += 1;
        visit(&mut builder, visit_state, &mut stack);
    }
    builder.flush();
    builder.preview
}
struct Visit<'a> {
    node: NodeRef<'a>,
    depth: usize,
    closing: bool,
}
fn visit<'a>(builder: &mut Builder, state: Visit<'a>, stack: &mut Vec<Visit<'a>>) {
    let Visit {
        node,
        depth,
        closing,
    } = state;
    let name = node.node_name().unwrap_or_default();
    if closing {
        builder.newline();
        return;
    }
    if excluded(node, &name) {
        return;
    }
    if node.is_text() {
        builder.append(&node.text());
        return;
    }
    match name.as_ref() {
        "img" => {
            builder.image(node);
            return;
        }
        "br" => {
            builder.newline();
            return;
        }
        _ => {}
    }
    if depth >= MAX_DEPTH {
        builder.preview.truncated = true;
        return;
    }
    if block_element(&name) {
        builder.newline();
        stack.push(Visit {
            node,
            depth,
            closing: true,
        });
    }
    let children = node.children_it(true).take(MAX_NODES);
    for child in children {
        stack.push(Visit {
            node: child,
            depth: depth + 1,
            closing: false,
        });
    }
}
