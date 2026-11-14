//! 规则按 PEM、已知前缀、JWT 顺序命中，范围使用原文 UTF-8 字节偏移。
use super::Hit;

enum Length {
    Minimum(usize),
    Exact(usize),
}

enum Charset {
    Base64Url,
    Alphanumeric,
    Underscore,
    Hyphen,
    UppercaseDigit,
}

impl Charset {
    fn contains(&self, byte: u8) -> bool {
        match self {
            Self::Base64Url => b64(byte),
            Self::Alphanumeric => byte.is_ascii_alphanumeric(),
            Self::Underscore => byte.is_ascii_alphanumeric() || byte == b'_',
            Self::Hyphen => byte.is_ascii_alphanumeric() || byte == b'-',
            Self::UppercaseDigit => byte.is_ascii_uppercase() || byte.is_ascii_digit(),
        }
    }
}

impl Length {
    fn accepts(&self, length: usize) -> bool {
        match self {
            Self::Minimum(minimum) => length >= *minimum,
            Self::Exact(exact) => length == *exact,
        }
    }
}

const PREFIX_RULES: [(&str, Length, Charset, &str); 16] = [
    (
        "sk-proj-",
        Length::Minimum(20),
        Charset::Base64Url,
        "openai_token",
    ),
    (
        "sk-ant-",
        Length::Minimum(20),
        Charset::Base64Url,
        "anthropic_token",
    ),
    ("sk-", Length::Minimum(20), Charset::Base64Url, "sk_token"),
    (
        "ghp_",
        Length::Exact(36),
        Charset::Alphanumeric,
        "github_token",
    ),
    (
        "gho_",
        Length::Exact(36),
        Charset::Alphanumeric,
        "github_token",
    ),
    (
        "ghu_",
        Length::Exact(36),
        Charset::Alphanumeric,
        "github_token",
    ),
    (
        "ghs_",
        Length::Exact(36),
        Charset::Alphanumeric,
        "github_token",
    ),
    (
        "ghr_",
        Length::Exact(36),
        Charset::Alphanumeric,
        "github_token",
    ),
    (
        "github_pat_",
        Length::Minimum(22),
        Charset::Underscore,
        "github_pat",
    ),
    ("xoxb-", Length::Minimum(10), Charset::Hyphen, "slack_token"),
    ("xoxp-", Length::Minimum(10), Charset::Hyphen, "slack_token"),
    ("xoxa-", Length::Minimum(10), Charset::Hyphen, "slack_token"),
    ("xoxe-", Length::Minimum(10), Charset::Hyphen, "slack_token"),
    (
        "AKIA",
        Length::Exact(16),
        Charset::UppercaseDigit,
        "aws_access_key",
    ),
    (
        "ASIA",
        Length::Exact(16),
        Charset::UppercaseDigit,
        "aws_access_key",
    ),
    (
        "AIza",
        Length::Exact(35),
        Charset::Base64Url,
        "google_api_key",
    ),
];

pub(super) fn can_start(scan: &str, start: usize) -> bool {
    scan.is_char_boundary(start) && !scan[..start].chars().next_back().is_some_and(word)
}

pub(super) fn at(text: &str, scan: &str, start: usize) -> Option<Hit> {
    pem(text, scan, start)
        .or_else(|| prefixed(text, scan, start))
        .or_else(|| jwt(text, scan, start))
}

fn pem(text: &str, scan: &str, start: usize) -> Option<Hit> {
    const BEGIN: &str = "-----BEGIN ";
    let rest = scan[start..].strip_prefix(BEGIN)?;
    let label = &rest[..rest.find("-----")?];
    let header_end = start + BEGIN.len() + label.len() + 5;
    if !(label == "PRIVATE KEY" || label.ends_with(" PRIVATE KEY"))
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b' ')
        || !boundary(text, start, header_end)
    {
        return None;
    }
    let footer = format!("-----END {label}-----");
    let end = scan[header_end..]
        .find(&footer)
        .map_or(scan.len(), |position| header_end + position + footer.len());
    Some(Hit {
        rule: "pem_private_key".into(),
        range: start..end,
    })
}

fn prefixed(text: &str, scan: &str, start: usize) -> Option<Hit> {
    // 只取首个已识别前缀；长前缀校验失败不能退回较短 sk- 规则。
    let (prefix, length, charset, rule) = PREFIX_RULES
        .iter()
        .find(|(prefix, ..)| scan[start..].starts_with(prefix))?;
    let suffix = start + prefix.len();
    let end = suffix
        + scan.as_bytes()[suffix..]
            .iter()
            .take_while(|byte| charset.contains(**byte))
            .count();
    if !length.accepts(end - suffix) || !boundary(text, start, end) {
        return None;
    }
    Some(Hit {
        rule: (*rule).into(),
        range: start..end,
    })
}

fn jwt(text: &str, scan: &str, start: usize) -> Option<Hit> {
    let bytes = scan.as_bytes();
    // JSON 对象允许前导 JSON 空白，对应 base64url 首字符 e/I/C/D。
    if !matches!(bytes[start], b'e' | b'I' | b'C' | b'D') {
        return None;
    }
    let end = jwt_end(bytes, start)?;
    if !boundary(text, start, end) || !valid_jwt(&scan[start..end]) {
        return None;
    }
    Some(Hit {
        rule: "jwt".into(),
        range: start..end,
    })
}

fn jwt_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut end = start;
    let mut dots = 0;
    while end < bytes.len() && (b64(bytes[end]) || bytes[end] == b'.') {
        dots += usize::from(bytes[end] == b'.');
        // 第三个点直接排除多段候选，保留失败候选的扫描成本边界。
        if dots == 3 {
            return None;
        }
        end += 1;
    }
    (dots == 2).then_some(end)
}

fn valid_jwt(candidate: &str) -> bool {
    let parts: Vec<_> = candidate.split('.').collect();
    let [header, claims, signature] = parts.as_slice() else {
        return false;
    };
    parts
        .iter()
        .all(|part| !part.is_empty() && part.len() % 4 != 1)
        && json_object(header).is_some_and(|header| header.contains_key("alg"))
        && json_object(claims).is_some()
        && decode64(signature).is_some()
}

pub(super) fn boundary(text: &str, start: usize, end: usize) -> bool {
    !text[..start].chars().next_back().is_some_and(word)
        && !text[end..].chars().next().is_some_and(word)
}

fn word(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

fn b64(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

pub(super) fn decode64(text: &str) -> Option<Vec<u8>> {
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::new();
    let mut accumulator = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
            accumulator &= (1 << bits) - 1;
        }
    }
    if accumulator != 0 {
        return None;
    }
    Some(out)
}

fn json_object(text: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let decoded = decode64(text)?;
    let start = decoded
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())?;
    if decoded[start] != b'{' {
        return None;
    }
    serde_json::from_slice(&decoded).ok()
}
