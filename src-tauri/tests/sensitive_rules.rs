//! T7 规格补测：所有凭据均为无效合成数据，不联网、不访问桌面剪贴板。
use clipraft_windows::sensitive::{detect, expiry_delay, redact, SCAN_LIMIT};
use std::time::Duration;

const FAKE: &str = "sk-TESTONLY0000000000000000abcd";

// AC-1：变长规则在最短长度两侧的行为。
#[test]
fn variable_prefix_minimum_lengths() {
    for (prefix, minimum, rule) in [
        ("sk-", 20, "sk_token"),
        ("sk-proj-", 20, "openai_token"),
        ("sk-ant-", 20, "anthropic_token"),
        ("github_pat_", 22, "github_pat"),
        ("xoxb-", 10, "slack_token"),
        ("xoxp-", 10, "slack_token"),
        ("xoxa-", 10, "slack_token"),
        ("xoxe-", 10, "slack_token"),
    ] {
        for length in [minimum - 1, minimum, minimum + 1] {
            let sample = format!("{prefix}{}", "T".repeat(length));
            let hits = detect(&sample, false);
            assert_eq!(
                hits.len(),
                usize::from(length >= minimum),
                "{prefix}/{length}"
            );
            if let Some(hit) = hits.first() {
                assert_eq!(hit.rule, rule);
                assert_eq!(hit.range, 0..sample.len());
            }
        }
    }
}

#[test]
fn fixed_prefix_exact_lengths() {
    for (prefix, length) in [
        ("ghp_", 36),
        ("gho_", 36),
        ("ghu_", 36),
        ("ghs_", 36),
        ("ghr_", 36),
        ("AKIA", 16),
        ("ASIA", 16),
        ("AIza", 35),
    ] {
        for actual in [length - 1, length, length + 1] {
            assert_eq!(
                detect(&format!("{prefix}{}", "T".repeat(actual)), false).len(),
                usize::from(actual == length),
                "{prefix}/{actual}"
            );
        }
    }
}

#[test]
fn punctuation_and_emoji_boundaries_keep_exact_utf8_ranges() {
    for (left, right) in [
        ("", ""),
        ("（", "）"),
        ("“", "”"),
        ("😀", "🧪"),
        ("=\"", "\";"),
        ("\r\n", "\t"),
    ] {
        let text = format!("{left}{FAKE}{right}");
        let hits = detect(&text, false);
        assert_eq!(hits.len(), 1, "{left}/{right}");
        assert_eq!(hits[0].range, left.len()..left.len() + FAKE.len());
        assert!(!redact(&text, &hits).contains(FAKE));
    }
}

#[test]
fn ascii_word_boundaries_reject_embedded_samples() {
    for character in ['a', 'Z', '7', '_', '-'] {
        for text in [format!("{character}{FAKE}"), format!("{FAKE}{character}")] {
            // sk 的后缀是变长的；右侧 ASCII 字符属于同一个完整 token。
            let hits = detect(&text, false);
            if text.starts_with(FAKE) {
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].range, 0..text.len());
            } else {
                assert!(hits.is_empty());
            }
        }
    }
}

#[test]
fn scan_limit_includes_a_complete_token_ending_at_the_limit() {
    let start = SCAN_LIMIT - FAKE.len();
    let text = format!("{}{FAKE}", " ".repeat(start));
    let hits = detect(&text, false);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].range, start..SCAN_LIMIT);
    assert_eq!(detect(&format!("{text}。"), false), hits);
}

#[test]
fn scan_limit_does_not_accept_a_truncated_candidate() {
    for crossing in [1, 4, FAKE.len() - 1] {
        let text = format!("{}{FAKE}", " ".repeat(SCAN_LIMIT - FAKE.len() + crossing));
        assert!(detect(&text, false).is_empty(), "跨界 {crossing} 字节");
    }
    assert!(detect(&format!("{}{FAKE}", " ".repeat(SCAN_LIMIT)), false).is_empty());
}

#[test]
fn multibyte_scan_cutoffs_keep_all_ranges_on_character_boundaries() {
    for character in ["中", "😀", "é", "\u{301}"] {
        for offset in 0..character.len() {
            let text = format!(
                "{FAKE} {}{character} 尾文",
                " ".repeat(SCAN_LIMIT - FAKE.len() - 1 - offset)
            );
            let hits = detect(&text, false);
            assert_eq!(hits.len(), 1);
            for hit in &hits {
                assert!(text.is_char_boundary(hit.range.start));
                assert!(text.is_char_boundary(hit.range.end));
                assert!(hit.range.end <= SCAN_LIMIT);
            }
            assert!(!redact(&text, &hits).contains(FAKE));
        }
    }
}

#[test]
fn ordinary_documents_data_urls_markdown_and_logs_are_not_sensitive() {
    for sample in [
        "中文普通记录：测试完成，无密钥。",
        "const TEST_ONLY_COUNT = 42; // 普通代码",
        "https://example.invalid/docs?q=TEST_ONLY#intro",
        "550e8400-e29b-41d4-a716-446655440000",
        "cc61528a0123456789abcdef0123456789abcdef012",
        "data:image/png;base64,VEVTVE9OTFk=",
        r"D:\Projects\TEST_ONLY\文档\很长的目录名称\file-2026-11-12.txt",
        "# 测试文档\n- [普通链接](https://example.invalid/)\n```rust\nlet count = 42;\n```",
        "2026-11-12T10:20:30 INFO worker: saved id=cc61528 count=42",
    ] {
        for entropy in [false, true] {
            assert!(detect(sample, entropy).is_empty(), "{sample}/{entropy}");
        }
    }
}

#[test]
fn pem_dsa_and_encrypted_variants_preserve_only_headers_and_footers() {
    for label in ["DSA PRIVATE KEY", "EC PRIVATE KEY", "ENCRYPTED PRIVATE KEY"] {
        let sample =
            format!("-----BEGIN {label}-----\r\nTEST_ONLY_NOT_A_KEY\r\n-----END {label}-----");
        let hits = detect(&sample, false);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].range, 0..sample.len());
        let masked = redact(&sample, &hits);
        assert!(masked.starts_with(&format!("-----BEGIN {label}-----")));
        assert!(masked.ends_with(&format!("-----END {label}-----")));
        assert!(!masked.contains("TEST_ONLY_NOT_A_KEY"));
    }
}

#[test]
fn mixed_rules_have_ordered_nonoverlapping_ranges_and_preserve_context() {
    let sample = format!("说明：{FAKE}，xoxb-TESTONLY012345；结束");
    let hits = detect(&sample, true);
    assert_eq!(hits.len(), 2);
    assert!(hits.windows(2).all(|h| h[0].range.end <= h[1].range.start));
    let masked = redact(&sample, &hits);
    assert!(masked.starts_with("说明："));
    assert!(masked.ends_with("；结束"));
    assert!(!masked.contains(FAKE));
    assert!(!masked.contains("xoxb-TESTONLY012345"));
}

#[test]
fn expiry_delay_equal_now_and_extreme_timestamps_are_bounded() {
    assert_eq!(expiry_delay(1000, [1000, 1001]), Some(Duration::ZERO));
    assert_eq!(expiry_delay(i64::MAX, [i64::MIN]), Some(Duration::ZERO));
    assert_eq!(
        expiry_delay(i64::MIN, [i64::MAX]),
        Some(Duration::from_millis(i64::MAX as u64))
    );
}

// AC-1：非 ASCII 字符是边界，命中区间仅包含合成 token。
#[test]
fn chinese_adjacent_tokens_meet_requested_acceptance_rule() {
    for token in [FAKE.to_owned(), format!("ghp_TESTONLY{}", "0".repeat(28))] {
        for (left, right) in [
            ("密钥", ""),
            ("", "中文"),
            ("前", "后"),
            ("（", "）"),
            ("令牌：", "。"),
            ("全角Ａ", "１结束"),
            ("é", "é"),
        ] {
            let sample = format!("{left}{token}{right}");
            let hits = detect(&sample, false);
            assert_eq!(hits.len(), 1, "{sample}");
            assert_eq!(hits[0].range, left.len()..left.len() + token.len());
            assert_eq!(&sample[hits[0].range.clone()], token);
            let masked = redact(&sample, &hits);
            assert!(masked.starts_with(left));
            assert!(masked.ends_with(right));
            assert!(!masked.contains(&token));
        }
    }
}

// AC-1：合法 JSON 头允许前导空白及花括号后的空白，不限定 eyJ。
#[test]
fn jwt_json_header_with_whitespace_is_accepted() {
    for header in [
        r#"{"alg":"TEST_ONLY"}"#,
        r#"{ "alg":"TEST_ONLY" }"#,
        " \t\r\n{\"alg\":\"TEST_ONLY\"}",
        "\t{\"alg\":\"TEST_ONLY\"}",
        "\r{\"alg\":\"TEST_ONLY\"}",
        "\n{\"alg\":\"TEST_ONLY\"}",
    ] {
        let sample = format!("{}.{}.VEVTVE9OTFk", encode64(header), encode64(" \n{}"));
        let text = format!("令牌{sample}结束");
        let hits = detect(&text, false);
        assert_eq!(hits.len(), 1, "{header:?}");
        assert_eq!(hits[0].rule, "jwt");
        assert_eq!(hits[0].range, "令牌".len().."令牌".len() + sample.len());
    }
    // 保留 Tester 原始验收样例。
    assert_eq!(
        detect("eyAiYWxnIjoiSFMyNTYiIH0.e30.VEVTVE9OTFk", false).len(),
        1
    );
}

// 测试专用编码器，所有 JWT 载荷和签名均为合成内容。
fn encode64(text: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::new();
    let mut acc = 0usize;
    let mut bits = 0;
    for byte in text.bytes() {
        acc = (acc << 8) | usize::from(byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            encoded.push(ALPHABET[(acc >> bits) & 63] as char);
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        encoded.push(ALPHABET[acc << (6 - bits)] as char);
    }
    encoded
}

#[test]
fn ascii_words_containing_sk_are_not_tokens() {
    for prefix in ["ta", "ri", "TASK_", "test-"] {
        assert!(detect(&format!("{prefix}{FAKE}"), false).is_empty());
    }
}

#[test]
fn jwt_requires_json_objects_and_valid_nonempty_signature() {
    let header = encode64(r#"{"alg":"TEST_ONLY"}"#);
    for invalid in [
        "[]",
        "null",
        "true",
        "42",
        r#""TEST_ONLY""#,
        "TEST_ONLY",
        "{",
        "",
    ] {
        let encoded = encode64(invalid);
        assert!(detect(&format!("{encoded}.e30.VEVTVE9OTFk"), false).is_empty());
        assert!(detect(&format!("{header}.{encoded}.VEVTVE9OTFk"), false).is_empty());
    }
    for invalid_header in [
        r#"{}"#,
        r#"{"not_alg":"TEST_ONLY"}"#,
        "\u{b}{\"alg\":\"TEST_ONLY\"}",
    ] {
        assert!(detect(
            &format!("{}.e30.VEVTVE9OTFk", encode64(invalid_header)),
            false
        )
        .is_empty());
    }
    for signature in ["", "Z", "Zh", "VEVTVE9OTF$", "VEVTVE9OTFk.extra"] {
        assert!(detect(&format!("{header}.e30.{signature}"), false).is_empty());
    }
}

#[test]
fn non_jwt_candidates_do_not_hide_following_tokens() {
    for prefix in ["example.invalid.", "INVALID.", "CODE.", "DATA.", "e.e.e.e."] {
        let sample = format!("{prefix}{FAKE}");
        let hits = detect(&sample, false);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule, "sk_token");
        assert_eq!(hits[0].range, prefix.len()..sample.len());
    }
}

#[test]
fn invalid_long_prefix_never_falls_back_to_generic_sk() {
    for prefix in ["sk-proj-", "sk-ant-"] {
        for length in 14..20 {
            let invalid = format!("{prefix}{}", "T".repeat(length));
            let text = format!("{invalid} {FAKE}");
            let hits = detect(&text, false);
            assert_eq!(hits.len(), 1, "{prefix}/{length}");
            assert_eq!(hits[0].rule, "sk_token");
            assert_eq!(hits[0].range, invalid.len() + 1..text.len());
        }
    }
}

#[test]
fn pem_wins_over_embedded_tokens_and_entropy_only_adds_disjoint_hits() {
    let entropy = "aB3!cD4@eF5#gH6$jK7%Lm8&";
    let pem = format!("-----BEGIN PRIVATE KEY-----\n{FAKE}\n{entropy}\n-----END PRIVATE KEY-----");
    let text = format!("{entropy} {pem} {FAKE}");
    let hits = detect(&text, true);
    assert_eq!(
        hits.iter().map(|hit| hit.rule.as_str()).collect::<Vec<_>>(),
        ["high_entropy", "pem_private_key", "sk_token"]
    );
    assert_eq!(
        hits[1].range,
        entropy.len() + 1..entropy.len() + 1 + pem.len()
    );
    assert!(hits
        .windows(2)
        .all(|pair| pair[0].range.end <= pair[1].range.start));
}

#[test]
fn pem_footer_beyond_multibyte_scan_boundary_is_not_consumed() {
    let header = "-----BEGIN PRIVATE KEY-----\n";
    let text = format!(
        "{header}{}中\n-----END PRIVATE KEY-----",
        " ".repeat(SCAN_LIMIT - header.len() - 1)
    );
    let hits = detect(&text, true);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rule, "pem_private_key");
    assert_eq!(hits[0].range, 0..SCAN_LIMIT - 1);
    assert!(text.is_char_boundary(hits[0].range.end));
}
