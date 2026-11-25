use anyhow::{bail, Context, Result};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

pub const MAX_CONTENT_IGNORE_RULES: usize = 50;
pub const MAX_CONTENT_IGNORE_RULE_LENGTH: usize = 200;
const REGEX_SIZE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentIgnoreMode {
    Keyword,
    Regex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentIgnoreRule {
    pub mode: ContentIgnoreMode,
    pub value: String,
}

#[derive(Default)]
pub struct CompiledContentRules(Vec<CompiledContentRule>);

enum CompiledContentRule {
    Keyword(String),
    Regex(Regex),
}

fn compile_rule(index: usize, rule: &ContentIgnoreRule) -> Result<CompiledContentRule> {
    let value = rule.value.trim();
    if value.is_empty() {
        bail!("第 {} 条忽略内容规则不能为空", index + 1);
    }
    if value.chars().count() > MAX_CONTENT_IGNORE_RULE_LENGTH {
        bail!("第 {} 条忽略内容规则不能超过 200 个字符", index + 1);
    }
    match rule.mode {
        ContentIgnoreMode::Keyword => Ok(CompiledContentRule::Keyword(value.to_lowercase())),
        ContentIgnoreMode::Regex => RegexBuilder::new(value)
            .case_insensitive(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .build()
            .map(CompiledContentRule::Regex)
            .with_context(|| format!("第 {} 条正则表达式无效", index + 1)),
    }
}

fn compile_rules(rules: &[ContentIgnoreRule]) -> Result<Vec<CompiledContentRule>> {
    if rules.len() > MAX_CONTENT_IGNORE_RULES {
        bail!("忽略内容规则最多 50 条");
    }
    rules
        .iter()
        .enumerate()
        .map(|(index, rule)| compile_rule(index, rule))
        .collect()
}

pub fn validate_content_rules(rules: &[ContentIgnoreRule]) -> Result<()> {
    compile_content_rules(rules).map(|_| ())
}

pub fn compile_content_rules(rules: &[ContentIgnoreRule]) -> Result<CompiledContentRules> {
    compile_rules(rules).map(CompiledContentRules)
}

impl CompiledContentRules {
    pub fn matches(&self, text: Option<&str>) -> bool {
        let Some(text) = text else { return false };
        let text_lowercase = text.to_lowercase();
        self.0.iter().any(|rule| match rule {
            CompiledContentRule::Keyword(value) => text_lowercase.contains(value),
            CompiledContentRule::Regex(regex) => regex.is_match(text),
        })
    }
}

pub fn content_matches(text: Option<&str>, rules: &[ContentIgnoreRule]) -> Result<bool> {
    Ok(compile_content_rules(rules)?.matches(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(mode: ContentIgnoreMode, value: &str) -> ContentIgnoreRule {
        ContentIgnoreRule {
            mode,
            value: value.into(),
        }
    }

    #[test]
    fn keywords_ignore_case_and_regexes_match_case_insensitively() {
        let rules = [
            rule(ContentIgnoreMode::Keyword, "私密标记"),
            rule(ContentIgnoreMode::Regex, r"token-[0-9]{3}"),
        ];
        assert!(content_matches(Some("前缀私密标记后缀"), &rules).unwrap());
        assert!(content_matches(Some("TOKEN-123"), &rules).unwrap());
        assert!(!content_matches(Some("普通文本"), &rules).unwrap());
        assert!(!content_matches(None, &rules).unwrap());
    }

    #[test]
    fn invalid_and_oversized_rules_are_rejected() {
        assert!(validate_content_rules(&[rule(ContentIgnoreMode::Regex, "(")]).is_err());
        assert!(validate_content_rules(&[rule(ContentIgnoreMode::Keyword, " ")]).is_err());
        assert!(validate_content_rules(&[rule(
            ContentIgnoreMode::Keyword,
            &"x".repeat(MAX_CONTENT_IGNORE_RULE_LENGTH + 1)
        )])
        .is_err());
        let many = vec![rule(ContentIgnoreMode::Keyword, "x"); MAX_CONTENT_IGNORE_RULES + 1];
        assert!(validate_content_rules(&many).is_err());
    }

    #[test]
    fn long_text_matches_within_regex_limits() {
        let text = format!("{}needle", "x".repeat(1_000_000));
        let rules = [rule(ContentIgnoreMode::Regex, "needle$")];
        assert!(content_matches(Some(&text), &rules).unwrap());
    }
}
