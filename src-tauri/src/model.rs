use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FormatData {
    pub name: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Clip {
    pub formats: Vec<FormatData>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemView {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub formats: Vec<String>,
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    pub placeholder: Option<String>,
    pub files: Vec<String>,
}
impl Clip {
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.formats
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.bytes.as_slice())
    }
    pub fn text(&self) -> Option<String> {
        self.get("CF_UNICODETEXT").map(|b| {
            let words: Vec<u16> = b
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .take_while(|c| *c != 0)
                .collect();
            String::from_utf16_lossy(&words)
        })
    }
    pub fn hash(&self) -> String {
        // 加入名称、长度边界并排序，格式枚举顺序不影响内容身份。
        let mut formats: Vec<_> = self.formats.iter().collect();
        formats.sort_by(|a, b| a.name.cmp(&b.name));
        let mut h = Sha256::new();
        h.update(b"clipboard-m0-v1");
        for f in formats {
            h.update((f.name.len() as u64).to_le_bytes());
            h.update(f.name.as_bytes());
            h.update((f.bytes.len() as u64).to_le_bytes());
            h.update(&f.bytes);
        }
        format!("{:x}", h.finalize())
    }
    pub fn kind(&self) -> &'static str {
        if self.get("CF_HDROP").is_some() {
            "files"
        } else if image_priority().iter().any(|n| self.get(n).is_some()) {
            "image"
        } else {
            "text"
        }
    }
}
pub fn image_priority() -> [&'static str; 3] {
    ["PNG", "CF_DIBV5", "CF_DIB"]
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn is_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

// DROPFILES 的固定头长为 20 字节，路径使用双零结束的 UTF-16。
pub fn encode_hdrop(paths: &[String]) -> Vec<u8> {
    let mut out = vec![0u8; 20];
    out[0..4].copy_from_slice(&20u32.to_le_bytes());
    out[16..20].copy_from_slice(&1u32.to_le_bytes());
    for p in paths {
        for w in p.encode_utf16().chain(std::iter::once(0)) {
            out.extend(w.to_le_bytes());
        }
    }
    out.extend(0u16.to_le_bytes());
    out
}
pub fn decode_hdrop(bytes: &[u8]) -> Option<Vec<String>> {
    if bytes.len() < 22 {
        return None;
    }
    let offset = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
    if offset < 20 || offset >= bytes.len() || bytes[16..20] != 1u32.to_le_bytes() {
        return None;
    }
    let words: Vec<u16> = bytes[offset..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let mut paths = Vec::new();
    let mut start = 0;
    for (i, w) in words.iter().enumerate() {
        if *w == 0 {
            if start == i {
                return Some(paths);
            }
            paths.push(String::from_utf16_lossy(&words[start..i]));
            start = i + 1;
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    fn f(name: &str, bytes: &[u8]) -> FormatData {
        FormatData {
            name: name.into(),
            bytes: bytes.into(),
        }
    }
    #[test]
    fn hash_order_and_boundaries() {
        let a = Clip {
            formats: vec![f("a", b"bc"), f("b", b"d")],
        };
        let b = Clip {
            formats: vec![f("b", b"d"), f("a", b"bc")],
        };
        assert_eq!(a.hash(), b.hash());
        assert_ne!(
            a.hash(),
            Clip {
                formats: vec![f("ab", b"c"), f("b", b"d")]
            }
            .hash()
        );
    }
    #[test]
    fn files_roundtrip() {
        let paths = vec![r"C:\测试\a.txt".into(), r"C:\b.png".into()];
        assert_eq!(decode_hdrop(&encode_hdrop(&paths)), Some(paths));
        assert!(decode_hdrop(&[0; 20]).is_none());
    }
    #[test]
    fn priority_is_explicit() {
        assert_eq!(image_priority(), ["PNG", "CF_DIBV5", "CF_DIB"]);
    }
}
