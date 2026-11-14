//! 有界的本地敏感规则；不校验凭据有效性，不记录载荷。
mod rules;
#[cfg(test)]
use rules::decode64;
use serde::{Deserialize, Serialize};
use std::{ops::Range, time::Duration};
pub const SCAN_LIMIT: usize = 64 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hit {
    pub rule: String,
    pub range: Range<usize>,
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
        classes[character_class(b)] = true;
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
fn character_class(byte: u8) -> usize {
    match byte {
        b'A'..=b'Z' => 0,
        b'a'..=b'z' => 1,
        b'0'..=b'9' => 2,
        _ => 3,
    }
}

/// 范围是原始 UTF-8 字节偏移，永远落在字符边界上。
pub fn detect(text: &str, high_entropy: bool) -> Vec<Hit> {
    let mut limit = text.len().min(SCAN_LIMIT);
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    let scan = &text[..limit];
    let mut hits = scan_rules(text, scan);
    if high_entropy {
        append_entropy(text, scan, &mut hits);
        hits.sort_by_key(|hit| hit.range.start);
    }
    hits
}

fn scan_rules(text: &str, scan: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut cursor = 0;
    while cursor < scan.len() {
        if !rules::can_start(scan, cursor) {
            cursor += 1;
            continue;
        }
        match rules::at(text, scan, cursor) {
            Some(hit) => {
                cursor = hit.range.end;
                hits.push(hit);
            }
            None => cursor += 1,
        }
    }
    hits
}

fn append_entropy(text: &str, scan: &str, hits: &mut Vec<Hit>) {
    let mut start = 0;
    for part in scan.split_inclusive(char::is_whitespace) {
        let candidate = part.trim_end_matches(char::is_whitespace);
        let end = start + candidate.len();
        if entropy_candidate(candidate)
            && rules::boundary(text, start, end)
            && !hits
                .iter()
                .any(|hit| hit.range.start < end && start < hit.range.end)
        {
            hits.push(Hit {
                rule: "high_entropy".into(),
                range: start..end,
            });
        }
        start += part.len();
    }
}

pub fn redact(text: &str, hits: &[Hit]) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    for hit in hits {
        let r = &hit.range;
        if r.start < cursor
            || r.start > r.end
            || r.end > text.len()
            || !text.is_char_boundary(r.start)
            || !text.is_char_boundary(r.end)
        {
            continue;
        }
        out.push_str(&text[cursor..r.start]);
        let secret = &text[r.clone()];
        match hit.rule.as_str() {
            "unscanned_tail" => out.push_str("••••"),
            "pem_private_key" => redact_pem(&mut out, secret),
            _ => redact_token(&mut out, secret),
        }
        cursor = r.end;
    }
    out.push_str(&text[cursor..]);
    out
}
fn redact_pem(out: &mut String, secret: &str) {
    let header = secret
        .find("-----")
        .and_then(|_| secret.get(5..)?.find("-----").map(|position| position + 10))
        .unwrap_or(0);
    out.push_str(&secret[..header]);
    out.push_str("\n••••\n");
    if let Some(end) = secret.rfind("-----END ") {
        out.push_str(&secret[end..]);
    }
}

fn redact_token(out: &mut String, secret: &str) {
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

pub fn expiry_delay(now: i64, deadlines: impl IntoIterator<Item = i64>) -> Option<Duration> {
    deadlines
        .into_iter()
        .min()
        .map(|deadline| Duration::from_millis(deadline.saturating_sub(now).max(0) as u64))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reversed_hit_range_is_skipped_like_other_invalid_ranges() {
        let text = "prefix synthetic suffix";
        let start = 12;
        let end = 3;
        let hit = Hit {
            rule: "manual".into(),
            range: start..end,
        };
        assert_eq!(redact(text, &[hit]), text);
    }

    #[test]
    fn malformed_pem_metadata_does_not_slice_inside_utf8() {
        let text = "甲乙-----END PRIVATE KEY-----";
        let hit = Hit {
            rule: "pem_private_key".into(),
            range: 0..text.len(),
        };
        assert_eq!(redact(text, &[hit]), "\n••••\n-----END PRIVATE KEY-----");
    }

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
