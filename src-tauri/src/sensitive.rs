//! 有界的本地敏感规则；不校验凭据有效性，不记录载荷。
use serde::{Deserialize, Serialize};
use std::{ops::Range, time::Duration};
pub const SCAN_LIMIT: usize = 64 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hit {
    pub rule: String,
    pub range: Range<usize>,
}
fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}
fn boundary(text: &str, start: usize, end: usize) -> bool {
    !text[..start].chars().next_back().is_some_and(word)
        && !text[end..].chars().next().is_some_and(word)
}
fn b64(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')
}
fn decode64(s: &str) -> Option<Vec<u8>> {
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for b in s.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if acc != 0 {
        return None;
    }
    Some(out)
}
fn json_object(s: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let decoded = decode64(s)?;
    let start = decoded.iter().position(|b| !b.is_ascii_whitespace())?;
    if decoded[start] != b'{' {
        return None;
    }
    serde_json::from_slice(&decoded).ok()
}
fn entropy_candidate(s: &str) -> bool {
    if s.len() < 20
        || !s.is_ascii()
        || s.starts_with("AKIA")
        || s.starts_with("ASIA")
        || s.contains(['/', '\\', ':', '.'])
        || s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
    {
        return false;
    }
    let mut classes = [false; 4];
    let mut counts = [0usize; 128];
    for b in s.bytes() {
        classes[if b.is_ascii_uppercase() {
            0
        } else if b.is_ascii_lowercase() {
            1
        } else if b.is_ascii_digit() {
            2
        } else {
            3
        }] = true;
        counts[b as usize] += 1;
    }
    if classes.into_iter().filter(|b| *b).count() < 3 {
        return false;
    }
    let entropy: f64 = counts
        .into_iter()
        .filter(|n| *n > 0)
        .map(|n| {
            let p = n as f64 / s.len() as f64;
            -p * p.log2()
        })
        .sum();
    entropy > 4.0
}
/// 范围是原始 UTF-8 字节偏移，永远落在字符边界上。
pub fn detect(text: &str, high_entropy: bool) -> Vec<Hit> {
    let mut limit = text.len().min(SCAN_LIMIT);
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    let scan = &text[..limit];
    let bytes = scan.as_bytes();
    let mut hits = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !scan.is_char_boundary(i) {
            i += 1;
            continue;
        }
        if scan[..i].chars().next_back().is_some_and(word) {
            i += 1;
            continue;
        }
        let rest = &scan[i..];
        if rest.starts_with("-----BEGIN ") {
            if let Some(n) = rest
                .strip_prefix("-----BEGIN ")
                .and_then(|s| s.find("-----").map(|n| n + 11))
            {
                let header_end = i + n + 5;
                let label = &scan[i + 11..i + n];
                if (label == "PRIVATE KEY" || label.ends_with(" PRIVATE KEY"))
                    && label.bytes().all(|b| b.is_ascii_uppercase() || b == b' ')
                    && boundary(text, i, header_end)
                {
                    let footer = format!("-----END {label}-----");
                    let end = scan[header_end..]
                        .find(&footer)
                        .map_or(limit, |p| header_end + p + footer.len());
                    hits.push(Hit {
                        rule: "pem_private_key".into(),
                        range: i..end,
                    });
                    i = end;
                    continue;
                }
            }
        }
        let mut matched = None;
        for (prefix, min, exact, charset, rule) in [
            ("sk-proj-", 20, false, 0, "openai_token"),
            ("sk-ant-", 20, false, 0, "anthropic_token"),
            ("sk-", 20, false, 0, "sk_token"),
            ("ghp_", 36, true, 1, "github_token"),
            ("gho_", 36, true, 1, "github_token"),
            ("ghu_", 36, true, 1, "github_token"),
            ("ghs_", 36, true, 1, "github_token"),
            ("ghr_", 36, true, 1, "github_token"),
            ("github_pat_", 22, false, 2, "github_pat"),
            ("xoxb-", 10, false, 3, "slack_token"),
            ("xoxp-", 10, false, 3, "slack_token"),
            ("xoxa-", 10, false, 3, "slack_token"),
            ("xoxe-", 10, false, 3, "slack_token"),
            ("AKIA", 16, true, 4, "aws_access_key"),
            ("ASIA", 16, true, 4, "aws_access_key"),
            ("AIza", 35, true, 0, "google_api_key"),
        ] {
            if !rest.starts_with(prefix) {
                continue;
            }
            let start = i + prefix.len();
            let mut end = start;
            while end < limit
                && match charset {
                    1 => bytes[end].is_ascii_alphanumeric(),
                    2 => bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_',
                    3 => bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-',
                    4 => bytes[end].is_ascii_uppercase() || bytes[end].is_ascii_digit(),
                    _ => b64(bytes[end]),
                }
            {
                end += 1;
            }
            if end - start >= min && (!exact || end - start == min) && boundary(text, i, end) {
                matched = Some((end, rule));
                break;
            }
            // 已识别长前缀但长度不够时不退回较短 sk- 规则。
            break;
        }
        // JSON 对象只能以 { 或 JSON 空白开始，对应 base64url 首字符 e/I/C/D。
        if matched.is_none() && matches!(bytes[i], b'e' | b'I' | b'C' | b'D') {
            let mut end = i;
            let mut dots = 0;
            while end < limit && (b64(bytes[end]) || bytes[end] == b'.') {
                if bytes[end] == b'.' {
                    dots += 1;
                    if dots == 3 {
                        break;
                    }
                }
                end += 1;
            }
            // 第三个点直接排除多段候选，并限制失败候选的扫描成本。
            if dots == 2 && boundary(text, i, end) {
                let parts: Vec<_> = scan[i..end].split('.').collect();
                if parts.iter().all(|p| !p.is_empty() && p.len() % 4 != 1)
                    && json_object(parts[0]).is_some_and(|h| h.contains_key("alg"))
                    && json_object(parts[1]).is_some()
                    && decode64(parts[2]).is_some()
                {
                    matched = Some((end, "jwt"));
                }
            }
        }
        if let Some((end, rule)) = matched {
            hits.push(Hit {
                rule: rule.into(),
                range: i..end,
            });
            i = end;
        } else {
            i += 1;
        }
    }
    if high_entropy {
        let mut start = 0;
        for part in scan.split_inclusive(char::is_whitespace) {
            let s = part.trim_end_matches(char::is_whitespace);
            let end = start + s.len();
            if entropy_candidate(s)
                && boundary(text, start, end)
                && !hits
                    .iter()
                    .any(|h| h.range.start < end && start < h.range.end)
            {
                hits.push(Hit {
                    rule: "high_entropy".into(),
                    range: start..end,
                });
            }
            start += part.len();
        }
        hits.sort_by_key(|h| h.range.start);
    }
    hits
}
pub fn redact(text: &str, hits: &[Hit]) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    for hit in hits {
        let r = &hit.range;
        if r.start < cursor
            || r.end > text.len()
            || !text.is_char_boundary(r.start)
            || !text.is_char_boundary(r.end)
        {
            continue;
        }
        out.push_str(&text[cursor..r.start]);
        let secret = &text[r.clone()];
        if hit.rule == "unscanned_tail" {
            out.push_str("••••");
        } else if hit.rule == "pem_private_key" {
            let header = secret
                .find("-----")
                .and_then(|_| secret[5..].find("-----").map(|p| p + 10))
                .unwrap_or(0);
            out.push_str(&secret[..header]);
            out.push_str("\n••••\n");
            if let Some(end) = secret.rfind("-----END ") {
                out.push_str(&secret[end..]);
            }
        } else {
            let long = secret.chars().take(9).count() > 8;
            if long {
                out.extend(secret.chars().take(4));
            }
            out.push_str("••••••••");
            if long {
                let tail: Vec<_> = secret.chars().rev().take(2).collect();
                out.extend(tail.iter().rev());
            }
        }
        cursor = r.end;
    }
    out.push_str(&text[cursor..]);
    out
}
pub fn expiry_delay(now: i64, deadlines: impl IntoIterator<Item = i64>) -> Option<Duration> {
    deadlines
        .into_iter()
        .min()
        .map(|deadline| Duration::from_millis(deadline.saturating_sub(now).max(0) as u64))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn check(prefix: &str, suffix: &str, rule: &str) {
        let token = format!("{prefix}{suffix}");
        let text = format!("前文：({token}) 后文");
        let hits = detect(&text, false);
        assert_eq!(hits.len(), 1, "{prefix}");
        assert_eq!(hits[0].rule, rule);
        assert_eq!(&text[hits[0].range.clone()], token);
        assert!(detect(&format!("z{token}"), false).is_empty());
        assert!(detect(&format!("{prefix}short"), false).is_empty());
        assert!(!redact(&text, &hits).contains(&token));
        assert!(redact(&text, &hits).starts_with("前文：("));
    }
    #[test]
    fn sk() {
        check("sk-", "aB0123456789_cD-efGHI", "sk_token");
    }
    #[test]
    fn sk_proj() {
        check("sk-proj-", "aB0123456789_cD-efGHI", "openai_token");
        assert!(detect("sk-proj-123456789012345", false).is_empty());
    }
    #[test]
    fn sk_ant() {
        check("sk-ant-", "aB0123456789_cD-efGHI", "anthropic_token");
    }
    #[test]
    fn ghp() {
        check("ghp_", &"A1".repeat(18), "github_token");
    }
    #[test]
    fn gho() {
        check("gho_", &"A1".repeat(18), "github_token");
    }
    #[test]
    fn ghu() {
        check("ghu_", &"A1".repeat(18), "github_token");
    }
    #[test]
    fn ghs() {
        check("ghs_", &"A1".repeat(18), "github_token");
    }
    #[test]
    fn ghr() {
        check("ghr_", &"A1".repeat(18), "github_token");
    }
    #[test]
    fn github_pat() {
        check("github_pat_", "AB_cd0123456789EF_ghIJK", "github_pat");
    }
    #[test]
    fn xoxb() {
        check("xoxb-", "ABcd012345-678", "slack_token");
    }
    #[test]
    fn xoxp() {
        check("xoxp-", "ABcd012345-678", "slack_token");
    }
    #[test]
    fn xoxa() {
        check("xoxa-", "ABcd012345-678", "slack_token");
    }
    #[test]
    fn xoxe() {
        check("xoxe-", "ABcd012345-678", "slack_token");
    }
    #[test]
    fn akia() {
        check("AKIA", "ABCD0123456789EF", "aws_access_key");
        assert!(detect("AKIAabcd0123456789ef", false).is_empty());
    }
    #[test]
    fn asia() {
        check("ASIA", "ABCD0123456789EF", "aws_access_key");
    }
    #[test]
    fn google() {
        check("AIza", &"A".repeat(35), "google_api_key");
    }
    #[test]
    fn exact_lengths_and_right_boundaries() {
        for (p, n) in [("ghp_", 36), ("AKIA", 16), ("AIza", 35)] {
            for extra in ["A", "_", "-a"] {
                assert!(detect(&format!("{p}{}{extra}", "A".repeat(n)), false).is_empty());
            }
            let token = format!("{p}{}", "T".repeat(n));
            let hits = detect(&format!("{token}中文"), false);
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].range, 0..token.len());
        }
    }
    #[test]
    fn jwt_json_alg_and_base64() {
        let token = "eyJhbGciOiJIUzI1NiJ9.e30.c2ln";
        let h = detect(token, false);
        assert_eq!(h[0].rule, "jwt");
        assert_eq!(h[0].range, 0..token.len());
        for bad in [
            "eyJ.e30.c2ln",
            "eyJ4IjoxfQ.e30.c2ln",
            "eyJhbGciOiJIUzI1NiJ9..c2ln",
            "eyJhbGciOiJIUzI1NiJ9.e30.c2ln.extra",
            "eyJhbGciOiJIUzI1NiJ9.e30.Z",
        ] {
            assert!(detect(bad, false).is_empty(), "{bad}");
        }
    }
    #[test]
    fn base64_rejects_bad_padding_bits() {
        assert_eq!(decode64("e30"), Some(b"{}".to_vec()));
        for bad in ["Z", "Zh", "Zg=", "Z$"] {
            assert!(decode64(bad).is_none());
        }
    }
    #[test]
    fn pem_variants() {
        for label in [
            "PRIVATE KEY",
            "RSA PRIVATE KEY",
            "EC PRIVATE KEY",
            "OPENSSH PRIVATE KEY",
            "ENCRYPTED PRIVATE KEY",
        ] {
            let text = format!(
                "前文\n-----BEGIN {label}-----\r\nSYNTHETIC_BODY\n-----END {label}-----\n后文"
            );
            let h = detect(&text, false);
            assert_eq!(h.len(), 1);
            let masked = redact(&text, &h);
            assert!(!masked.contains("SYNTHETIC_BODY"));
            assert!(masked.contains(&format!("-----END {label}-----")));
            assert!(masked.ends_with("后文"));
        }
        assert!(detect(
            "-----BEGIN PUBLIC KEY-----\nbody\n-----END PUBLIC KEY-----",
            false
        )
        .is_empty());
    }
    #[test]
    fn pem_missing_footer_masks_remainder() {
        let s = "-----BEGIN PRIVATE KEY-----\nSYNTHETIC_BODY";
        let h = detect(s, false);
        assert_eq!(h[0].range.end, s.len());
        assert!(!redact(s, &h).contains("SYNTHETIC_BODY"));
    }
    #[test]
    fn entropy_opt_in() {
        let s = "aB3!cD4@eF5#gH6$jK7%Lm8&";
        assert!(detect(s, false).is_empty());
        assert_eq!(detect(s, true)[0].rule, "high_entropy");
        assert!(detect("AAAAAAAAAAAAaaaa1111!!", true).is_empty());
    }
    #[test]
    fn ordinary_content_not_sensitive() {
        for s in [
            "Hello world this is ordinary English",
            "这是普通中文文字和标点。",
            "const value = 42; function hello() { return value; }",
            "https://Example.com/ABcd1234EFgh5678?x=Yz!",
            r"C:\Users\Example\ABcd123456EFgh789.txt",
            "550e8400-e29b-41d4-a716-446655440000",
            "cc61528",
            "a1234567890bcdef1234567890abcdef1234567890",
            "sk-short",
            "AKIAabcd123456789012",
        ] {
            assert!(detect(s, true).is_empty(), "{s}");
        }
    }
    #[test]
    fn multiple_unicode_ranges_redact_exactly() {
        let a = format!("sk-{}", "a".repeat(20));
        let b = format!("ghp_{}", "B".repeat(36));
        let s = format!("中文 😀 {a}，普通 {b} 结束");
        let h = detect(&s, false);
        assert_eq!(h.len(), 2);
        assert_eq!(
            redact(&s, &h),
            "中文 😀 sk-a••••••••aa，普通 ghp_••••••••BB 结束"
        );
    }
    #[test]
    fn scan_limit_and_utf8() {
        let token = format!("sk-{}", "a".repeat(20));
        assert!(detect(&format!("{} {token}", "x".repeat(SCAN_LIMIT)), false).is_empty());
        let s = format!("{token} {}😀{token}", " ".repeat(SCAN_LIMIT - 27));
        assert_eq!(detect(&s, false).len(), 1);
        let crossing = format!("{} {token}", " ".repeat(SCAN_LIMIT - 10));
        assert!(detect(&crossing, false).is_empty());
    }
    #[test]
    fn short_manual_and_invalid_ranges() {
        assert_eq!(
            redact(
                "密码",
                &[Hit {
                    rule: "manual".into(),
                    range: 0..6
                }]
            ),
            "••••••••"
        );
        assert_eq!(
            redact(
                "中文",
                &[Hit {
                    rule: "manual".into(),
                    range: 1..2
                }]
            ),
            "中文"
        );
    }
    #[test]
    fn deadline_none() {
        assert_eq!(expiry_delay(100, []), None);
    }
    #[test]
    fn deadline_expired() {
        assert_eq!(expiry_delay(100, [90, 110]), Some(Duration::ZERO));
    }
    #[test]
    fn deadline_earliest() {
        assert_eq!(
            expiry_delay(100, [500, 200, 300]),
            Some(Duration::from_millis(100))
        );
    }
}
