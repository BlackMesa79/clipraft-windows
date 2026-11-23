use anyhow::{ensure, Result};
use dom_query::{Document, NodeRef, Selection};
use serde::Serialize;

const HISTORY_PATH: &str = "/BlackMesa79/clipraft-windows/releases";
const MAX_HISTORY_PAGES: usize = 3;
const NEXT_PAGE: &str =
    ".pagination a[rel='next'], .pagination a.next_page, a[aria-label='Next Page']";

#[derive(Serialize)]
pub(crate) struct ReleaseHistory {
    releases: Vec<ReleaseNotes>,
    complete: bool,
}

#[derive(Serialize)]
struct ReleaseNotes {
    tag: String,
    notes: String,
}

fn release_notes(node: NodeRef<'_>) -> Option<ReleaseNotes> {
    let anchor = node.attr("data-release-anchor")?;
    let tag = anchor
        .strip_prefix("release-")
        .filter(|tag| super::valid_tag(tag))?;
    let release = Selection::from(node);
    if !release
        .select(".Label--warning, .Label--danger")
        .nodes()
        .is_empty()
    {
        return None;
    }
    let notes = release
        .select("[data-test-selector='body-content'].markdown-body, .markdown-body")
        .nodes()
        .iter()
        .find_map(|body| super::notes::summary(*body))
        .unwrap_or_default();
    Some(ReleaseNotes {
        tag: tag.into(),
        notes,
    })
}

fn read_page(html: &str) -> Result<(Vec<ReleaseNotes>, bool)> {
    let document = Document::from(html);
    let cards = document.select("section[data-release-anchor]");
    ensure!(!cards.nodes().is_empty(), "无法识别 GitHub 公开发布列表");
    let releases = cards
        .nodes()
        .iter()
        .filter_map(|node| release_notes(*node))
        .collect();
    let has_next = !document.select(NEXT_PAGE).nodes().is_empty();
    Ok((releases, has_next))
}

pub(super) fn query() -> Result<ReleaseHistory> {
    let mut history = ReleaseHistory {
        releases: Vec::new(),
        complete: false,
    };
    for page in 1..=MAX_HISTORY_PAGES {
        let path = format!("{HISTORY_PATH}?page={page}");
        let (releases, has_next) =
            match super::read_release_html(&path).and_then(|html| read_page(&html)) {
                Ok(page_history) => page_history,
                Err(error) if page == 1 => return Err(error),
                Err(error) => {
                    crate::diag::write(format!("release history page {page} failed: {error:#}"));
                    break;
                }
            };
        history.releases.extend(releases);
        if !has_next {
            history.complete = true;
            break;
        }
    }
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_separate_sections_and_skips_prereleases() {
        let html = r#"
            <section data-release-anchor="release-v0.8.0">
                <div class="markdown-body"><h2>更新内容</h2><ul><li>背景</li></ul>
                <h2>下载与运行</h2><p>下载说明</p></div>
            </section>
            <section data-release-anchor="release-v0.7.0">
                <div class="markdown-body"><h2>更新内容</h2><ul><li>主题</li></ul></div>
            </section>
            <section data-release-anchor="release-v0.6.2">
                <span class="Label Label--warning">Pre-release</span>
                <div class="markdown-body"><h2>更新内容</h2><p>测试</p></div>
            </section>
            <div class="pagination"><a rel="next" href="?page=2">Next</a></div>"#;
        let (releases, has_next) = read_page(html).unwrap();
        assert!(has_next);
        let entries: Vec<_> = releases
            .iter()
            .map(|release| (release.tag.as_str(), release.notes.as_str()))
            .collect();
        assert_eq!(entries, [("v0.8.0", "- 背景"), ("v0.7.0", "- 主题")]);
    }

    #[test]
    #[ignore = "独立联网探测 GitHub 的公开发布列表"]
    fn live_release_history_includes_intermediate_stable_notes() {
        let history = query().unwrap();
        for tag in ["v0.8.0", "v0.7.0"] {
            let release = history
                .releases
                .iter()
                .find(|release| release.tag == tag)
                .unwrap();
            assert!(!release.notes.is_empty());
            println!(
                "release {tag}; summary lines: {}",
                release.notes.lines().count()
            );
        }
        assert!(!history
            .releases
            .iter()
            .any(|release| release.tag == "v0.6.2"));
    }
}
