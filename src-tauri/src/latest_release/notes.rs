use dom_query::{Document, NodeRef};

const UPDATE_HEADING: &str = "更新内容";
const MAX_SUMMARY_CHARS: usize = 4_000;

fn heading_level(name: &str) -> Option<u8> {
    let level = name.strip_prefix('h')?.parse::<u8>().ok()?;
    (1..=6).contains(&level).then_some(level)
}

fn heading(node: NodeRef<'_>) -> Option<(NodeRef<'_>, u8)> {
    let level = node.node_name().and_then(|name| heading_level(&name));
    if let Some(level) = level {
        return Some((node, level));
    }
    let classes = node.attr("class")?;
    if !classes
        .split_ascii_whitespace()
        .any(|name| name == "markdown-heading")
    {
        return None;
    }
    node.children_it(false).find_map(|child| {
        child
            .node_name()
            .and_then(|name| heading_level(&name))
            .map(|level| (child, level))
    })
}

fn append_text(node: NodeRef<'_>, prefix: &str, lines: &mut Vec<String>) {
    let text = node.text().split_whitespace().collect::<Vec<_>>().join(" ");
    if !text.is_empty() {
        lines.push(format!("{prefix}{text}"));
    }
}

fn append_block(node: NodeRef<'_>, lines: &mut Vec<String>) {
    if let Some((title, _)) = heading(node) {
        append_text(title, "", lines);
        return;
    }
    match node.node_name().as_deref() {
        Some("ul" | "ol") => node
            .children_it(false)
            .filter(|child| child.node_name().as_deref() == Some("li"))
            .for_each(|child| append_text(child, "- ", lines)),
        Some("p" | "blockquote") => append_text(node, "", lines),
        _ => {}
    }
}

fn section(body: NodeRef<'_>) -> Option<String> {
    let mut children = body.children_it(false);
    let level = children.find_map(|child| {
        let (title, level) = heading(child)?;
        (title.text().trim() == UPDATE_HEADING).then_some(level)
    })?;
    let mut lines = Vec::new();
    for node in children.take_while(|node| !heading(*node).is_some_and(|(_, next)| next <= level)) {
        append_block(node, &mut lines);
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

pub(super) fn read(html: &str) -> Option<String> {
    let document = Document::from(html);
    document
        .select(".markdown-body")
        .nodes()
        .iter()
        .find_map(|body| summary(*body))
}

pub(super) fn summary(body: NodeRef<'_>) -> Option<String> {
    let text = section(body)?;
    let mut characters = text.chars();
    let mut result: String = characters.by_ref().take(MAX_SUMMARY_CHARS).collect();
    if characters.next().is_some() {
        result.push('…');
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn extracts_update_content_and_stops_before_download() {
        let html = r#"<div class="markdown-body"><p>前言</p>
            <div class="markdown-heading"><h2>更新内容</h2><a>锚点</a></div>
            <ul><li>新增 <strong>设置</strong></li><li>优化 &amp; 修复</li></ul>
            <h3>细节</h3><p>保留子节</p>
            <h2>下载与运行</h2><p>下载命令</p></div>"#;
        assert_eq!(
            super::read(html).as_deref(),
            Some("- 新增 设置\n- 优化 & 修复\n细节\n保留子节"),
        );
    }

    #[test]
    fn omits_pages_without_an_update_section() {
        assert_eq!(
            super::read(r#"<div class="markdown-body"><h2>下载与运行</h2><p>示例</p></div>"#),
            None,
        );
    }
}
