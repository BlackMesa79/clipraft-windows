use crate::{
    model::{Clip, FormatData},
    settings::MergePasteSeparator as Separator,
    storage::crypto::MAX_PLAIN_BYTES,
};
use anyhow::{anyhow, bail, Result};

impl Separator {
    fn text(self) -> &'static str {
        match self {
            Self::Newline => "\r\n",
            Self::BlankLine => "\r\n\r\n",
            Self::Space => " ",
            Self::Tab => "\t",
            Self::Comma => ",",
        }
    }
}

pub(super) fn load(
    ids: &[String],
    separator: Separator,
    mut load_clip: impl FnMut(&str) -> Result<Clip>,
) -> Result<Clip> {
    let mut merged = TextAccumulator::new(separator);
    for id in ids {
        merged.append(&load_clip(id)?, MAX_PLAIN_BYTES)?;
    }
    Ok(merged.finish())
}

struct TextAccumulator {
    bytes: Vec<u8>,
    separator: Vec<u8>,
    has_text: bool,
}

impl TextAccumulator {
    fn new(separator: Separator) -> Self {
        Self {
            bytes: Vec::new(),
            separator: separator
                .text()
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
            has_text: false,
        }
    }

    fn append(&mut self, clip: &Clip, byte_limit: usize) -> Result<()> {
        let text = clip
            .get("CF_UNICODETEXT")
            .filter(|_| clip.kind() == "text")
            .ok_or_else(|| anyhow!("选中的记录中包含不支持合并的非文本内容"))?;
        let separator = if self.has_text {
            self.separator.as_slice()
        } else {
            &[]
        };
        let text = text_payload(text);
        if self
            .bytes
            .len()
            .saturating_add(separator.len())
            .saturating_add(text.len())
            .saturating_add(2)
            > byte_limit
        {
            bail!("合并内容超出大小限制");
        }
        self.bytes.extend_from_slice(separator);
        self.bytes.extend_from_slice(text);
        self.has_text = true;
        Ok(())
    }

    fn finish(mut self) -> Clip {
        self.bytes.extend_from_slice(&[0, 0]);
        Clip {
            formats: vec![FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: self.bytes,
            }],
        }
    }
}

fn text_payload(bytes: &[u8]) -> &[u8] {
    let even_length = bytes.len() & !1;
    let length = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .position(|word| *word == [0, 0])
        .unwrap_or(even_length / 2);
    &bytes[..length * 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(bytes: &[u8]) -> Clip {
        Clip {
            formats: vec![FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: bytes.to_vec(),
            }],
        }
    }

    #[test]
    fn combines_unicode_text_with_selected_separator() {
        let ids = ["中".into(), "A".into()];
        for (separator, expected) in [
            (Separator::Newline, "中\r\nA"),
            (Separator::BlankLine, "中\r\n\r\nA"),
            (Separator::Space, "中 A"),
            (Separator::Tab, "中\tA"),
            (Separator::Comma, "中,A"),
        ] {
            let combined = load(&ids, separator, |id| match id {
                "中" => Ok(clip(&[0x2d, 0x4e, 0, 0])),
                "A" => Ok(clip(&[65, 0, 0, 0])),
                _ => unreachable!(),
            })
            .unwrap();
            assert_eq!(combined.text().as_deref(), Some(expected));
        }
    }

    #[test]
    fn rejects_records_without_unicode_text() {
        let mut merged = TextAccumulator::new(Separator::Space);
        let mut image = clip(&[65, 0, 0, 0]);
        image.formats.push(FormatData {
            name: "CF_DIB".into(),
            bytes: vec![1],
        });
        let result = merged.append(&image, MAX_PLAIN_BYTES);
        assert_eq!(
            result.unwrap_err().to_string(),
            "选中的记录中包含不支持合并的非文本内容"
        );
    }

    #[test]
    fn size_limit_counts_empty_records_separator_and_terminator() {
        let mut merged = TextAccumulator::new(Separator::Comma);
        merged.append(&clip(&[0, 0]), 6).unwrap();
        merged.append(&clip(&[65, 0, 0, 0]), 6).unwrap();
        assert!(merged.append(&clip(&[0, 0]), 6).is_err());
        assert_eq!(merged.finish().text().as_deref(), Some(",A"));
    }
}
