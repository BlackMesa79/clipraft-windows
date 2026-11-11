use pinyin::ToPinyin;
use rusqlite::types::Value;
use std::collections::HashSet;

pub(super) const QUERY_WORD_LIMIT: usize = 32;
pub(super) const LINK_TEXT_LIMIT: usize = 8 * 1024;

pub(super) const PINYIN_HAN_LIMIT: usize = 2000;

/// 保留非汉字，使 hello你好 也能通过 hellonihao / hellonh 检索。
pub(super) fn pinyin_text(text: &str) -> String {
    let mut full = String::new();
    let mut initials = String::new();
    let mut count = 0;
    for ch in text.chars() {
        if let Some(py) = ch.to_pinyin() {
            if count == PINYIN_HAN_LIMIT {
                break;
            }
            count += 1;
            full.push_str(py.plain());
            if let Some(first) = py.plain().chars().next() {
                initials.push(first);
            }
        } else {
            full.push(ch);
            initials.push(ch);
        }
    }
    if count == 0 {
        String::new()
    } else {
        format!("{full}\n{initials}")
    }
}

pub(super) fn is_link(kind: &str, text: &str) -> bool {
    // 先检查完整文本的字节数，避免截断后误判，也限制 URL 解析开销。
    if text.len() > LINK_TEXT_LIMIT {
        return false;
    }
    let text = text.trim();
    kind == "text"
        && !text.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        && (text
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
            || text
                .get(..8)
                .is_some_and(|s| s.eq_ignore_ascii_case("https://")))
        && tauri::Url::parse(text).is_ok_and(|url| url.host_str().is_some())
}

pub(super) fn append_query(sql: &mut String, values: &mut Vec<Value>, query: Option<&str>) {
    let mut phrases = Vec::new();
    let mut seen = HashSet::new();
    for word in query.unwrap_or_default().split_whitespace() {
        if !seen.insert(word.to_lowercase()) {
            continue;
        }
        if seen.len() > QUERY_WORD_LIMIT {
            crate::diag::write(format!(
                "search query_truncated=true word_limit={QUERY_WORD_LIMIT}"
            ));
            break;
        }
        if word.chars().count() >= 3 {
            // 用户输入仅作为 FTS 字符串字面量；绝不解释运算符、列名或前缀语法。
            phrases.push(format!("\"{}\"", word.replace('"', "\"\"")));
        } else {
            let escaped = word
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            // 去重后总词数最多 32，LIKE 的 AND 深度远低于 SQLite 默认的 1000 层上限。
            sql.push_str(
                " AND (search_text LIKE ? ESCAPE '\\' OR search_pinyin LIKE ? ESCAPE '\\')",
            );
            let pattern = format!("%{escaped}%");
            values.push(pattern.clone().into());
            values.push(pattern.into());
        }
    }
    if !phrases.is_empty() {
        sql.push_str(" AND id IN (SELECT item_id FROM items_fts_map WHERE search_id IN (SELECT rowid FROM items_fts WHERE items_fts MATCH ?))");
        values.push(phrases.join(" AND ").into());
    }
}
