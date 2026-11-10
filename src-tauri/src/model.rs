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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ItemView {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    pub summary: String,
    pub formats: Vec<String>,
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    pub placeholder: Option<String>,
    pub files: Vec<String>,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub used_at: i64,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub group_id: Option<i64>,
    #[serde(default)]
    pub source_exe: Option<String>,
    #[serde(default)]
    pub source_title: Option<String>,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub expires_at: Option<i64>,
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
    /// 文件按路径集合与剪切语义去重；其他内容保持原有哈希身份。
    pub fn dedup_key(&self) -> String {
        self.file_dedup_key().unwrap_or_else(|| self.hash())
    }
    /// 迁移仅持有所需内联格式，失败时由调用者退回旧 id。
    pub(crate) fn file_dedup_key(&self) -> Option<String> {
        let mut paths = decode_hdrop(self.get("CF_HDROP")?)?;
        if paths.is_empty() {
            return None;
        }
        for path in &mut paths {
            *path = path
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_lowercase();
        }
        paths.sort();
        paths.dedup();
        let cut = self
            .get("Preferred DropEffect")
            .and_then(|bytes| bytes.get(..4))
            .is_some_and(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()) & 2 != 0);
        // HDROP 路径不能包含 NUL，以它分隔集合成员，避免路径拼接歧义。
        Some(digest(
            format!("files:{}:{}", u8::from(cut), paths.join("\0")).as_bytes(),
        ))
    }
    pub fn kind(&self) -> &'static str {
        if self.get("CF_HDROP").is_some() {
            "files"
        } else if image_priority().iter().any(|n| self.get(n).is_some()) {
            "image"
        } else if self.get("CF_UNICODETEXT").is_none()
            && (self.get("HTML Format").is_some() || self.get("Rich Text Format").is_some())
        {
            "rich"
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
    fn files(paths: &[&str], effect: Option<u32>) -> Clip {
        let mut clip = Clip {
            formats: vec![f(
                "CF_HDROP",
                &encode_hdrop(&paths.iter().map(|p| (*p).into()).collect::<Vec<_>>()),
            )],
        };
        if let Some(effect) = effect {
            clip.formats
                .push(f("Preferred DropEffect", &effect.to_le_bytes()));
        }
        clip
    }
    #[test]
    fn dedup_normalizes_path_case_separators_and_trailing_separators() {
        let a = files(&[r"D:\MyFiles\Desktop\IMAGE.JPG"], Some(5));
        let b = files(&["d:/myfiles/desktop/image.jpg/"], Some(1));
        assert_eq!(a.dedup_key(), b.dedup_key());
        assert_ne!(a.hash(), b.hash());
        assert!(is_hash(&a.dedup_key()));
    }
    #[test]
    fn dedup_sorts_and_deduplicates_file_sets_without_boundary_collisions() {
        let a = files(&[r"C:\a", r"D:\b", "c:/A/"], None);
        let b = files(&[r"d:\B", r"c:\A"], None);
        assert_eq!(a.dedup_key(), b.dedup_key());
        assert_ne!(a.dedup_key(), files(&[r"C:\aD:\b"], None).dedup_key());
        assert_ne!(a.dedup_key(), files(&[r"C:\a"], None).dedup_key());
    }
    #[test]
    fn dedup_ignores_extra_bitmap_and_html_formats_for_files() {
        let a = files(&[r"D:\Desktop\image.jpg"], Some(5));
        let mut b = files(&[r"D:\Desktop\image.jpg"], Some(1));
        b.formats.extend([
            f("CF_DIB", &[1; 100]),
            f("CF_DIBV5", &[2; 100]),
            f("HTML Format", b"<img>"),
        ]);
        assert_eq!(a.dedup_key(), b.dedup_key());
        assert_ne!(a.hash(), b.hash());
    }
    #[test]
    fn dedup_move_bit_distinguishes_cut_from_copy_and_link() {
        let key = files(&[r"C:\a"], None).dedup_key();
        for effect in [0, 1, 4, 5] {
            assert_eq!(key, files(&[r"C:\a"], Some(effect)).dedup_key());
        }
        let cut = files(&[r"C:\a"], Some(2)).dedup_key();
        assert_ne!(key, cut);
        for effect in [3, 6, 7] {
            assert_eq!(cut, files(&[r"C:\a"], Some(effect)).dedup_key());
        }
        let mut short = files(&[r"C:\a"], None);
        short.formats.push(f("Preferred DropEffect", &[2]));
        assert_eq!(key, short.dedup_key());
    }
    #[test]
    fn dedup_non_files_keep_the_content_hash() {
        for format in [
            f("CF_UNICODETEXT", &[65, 0, 0, 0]),
            f("PNG", b"png"),
            f("CF_DIB", b"dib"),
            f("CF_DIBV5", b"dibv5"),
        ] {
            let clip = Clip {
                formats: vec![format],
            };
            assert_eq!(clip.dedup_key(), clip.hash());
        }
    }
    #[test]
    fn dedup_invalid_or_empty_hdrop_falls_back_to_hash() {
        let mut truncated = encode_hdrop(&[r"C:\a".into()]);
        truncated.truncate(truncated.len() - 2);
        for bytes in [vec![], vec![0; 20], truncated, encode_hdrop(&[])] {
            let clip = Clip {
                formats: vec![f("CF_HDROP", &bytes), f("HTML Format", b"fallback")],
            };
            assert_eq!(clip.dedup_key(), clip.hash());
        }
    }
}
