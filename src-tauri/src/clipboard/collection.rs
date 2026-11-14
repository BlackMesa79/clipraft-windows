//! 格式读取结果与总预算合并；预算在规范化前扣除，格式顺序由 SUPPORTED 决定。
use super::{normalize, CaptureLimit, SUPPORTED};
use crate::{
    capture_policy::CaptureError,
    model::{Clip, FormatData},
};
use anyhow::Result;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum FormatChoice {
    Keep,
    Redundant,
}

pub(super) struct FormatBudget {
    pub total: usize,
    pub limit: usize,
    pub has_v5: bool,
}

impl FormatBudget {
    pub fn select(&self, name: &str, size: usize) -> Result<FormatChoice, CaptureLimit> {
        if name == "CF_DIB" && self.has_v5 {
            return Ok(FormatChoice::Redundant);
        }
        if size > self.limit || self.total.saturating_add(size) > self.limit {
            return Err(CaptureLimit {
                actual: self.total.saturating_add(size),
                limit: self.limit,
                image: matches!(name, "CF_DIBV5" | "CF_DIB" | "PNG"),
            });
        }
        Ok(FormatChoice::Keep)
    }
}

struct Collection {
    clip: Clip,
    budget: FormatBudget,
    invalid: bool,
    exceeded: Option<CaptureLimit>,
}

pub(super) fn collect_formats_with_budget(
    mut read: impl FnMut(&str, usize) -> Result<Option<Vec<u8>>>,
    limit: usize,
) -> Result<Option<Clip>> {
    let mut collection = Collection {
        clip: Clip::default(),
        budget: FormatBudget {
            total: 0,
            limit,
            has_v5: false,
        },
        invalid: false,
        exceeded: None,
    };
    for (name, _) in SUPPORTED {
        if matches!(
            collection.budget.select(name, 0),
            Ok(FormatChoice::Redundant)
        ) {
            crate::diag::write(format!("capture skip format={name} reason=redundant_dib"));
            continue;
        }
        let remaining = limit.saturating_sub(collection.budget.total);
        collection.merge(name, read(name, remaining));
    }
    collection.finish()
}

impl Collection {
    fn merge(&mut self, name: &str, result: Result<Option<Vec<u8>>>) {
        match result {
            Ok(Some(bytes)) => self.keep(name, bytes),
            Ok(None) => (),
            Err(error) => self.read_failure(name, error),
        }
    }

    fn read_failure(&mut self, name: &str, error: anyhow::Error) {
        let mut skipped = match error.downcast::<CaptureLimit>() {
            Ok(skipped) => skipped,
            Err(_) => {
                self.invalid |= name != "Preferred DropEffect";
                // 只记录固定格式名，不输出内容、路径或底层错误文本。
                eprintln!("剪贴板格式读取失败，已跳过：{name}");
                return;
            }
        };
        let FormatBudget { total, limit, .. } = self.budget;
        crate::diag::write(format!(
            "capture skip format={name} bytes={} total={total} limit={limit} reason=read_limit",
            skipped.actual
        ));
        skipped.actual = total.saturating_add(skipped.actual);
        skipped.limit = limit;
        self.record_exceeded(skipped);
    }

    fn keep(&mut self, name: &str, bytes: Vec<u8>) {
        if let Err(skipped) = self.budget.select(name, bytes.len()) {
            let FormatBudget { total, limit, .. } = self.budget;
            crate::diag::write(format!("capture skip format={name} bytes={} total={total} limit={limit} reason=capture_limit", bytes.len()));
            self.record_exceeded(skipped);
            return;
        }
        if bytes.is_empty() || (name == "Preferred DropEffect" && bytes.len() < 4) {
            self.invalid |= name != "Preferred DropEffect";
            eprintln!("剪贴板格式长度无效，已跳过：{name}");
            return;
        }
        self.budget.total += bytes.len();
        self.budget.has_v5 |= name == "CF_DIBV5";
        self.clip.formats.push(FormatData {
            name: name.into(),
            bytes: normalize(name, bytes),
        });
    }

    fn record_exceeded(&mut self, skipped: CaptureLimit) {
        if self
            .exceeded
            .as_ref()
            .is_none_or(|old| old.actual < skipped.actual)
        {
            self.exceeded = Some(skipped);
        }
    }

    fn finish(mut self) -> Result<Option<Clip>> {
        self.complete_drop_effect();
        if self
            .clip
            .formats
            .iter()
            .all(|format| format.name == "Preferred DropEffect")
        {
            return self.empty_outcome();
        }
        remove_redundant_dib(&mut self.clip.formats);
        Ok(Some(self.clip))
    }

    fn complete_drop_effect(&mut self) {
        if self.clip.get("CF_HDROP").is_some()
            && self.clip.get("Preferred DropEffect").is_none()
            && self.budget.total.saturating_add(4) <= self.budget.limit
        {
            self.clip.formats.push(FormatData {
                name: "Preferred DropEffect".into(),
                bytes: 1u32.to_le_bytes().to_vec(),
            });
        }
    }

    fn empty_outcome(self) -> Result<Option<Clip>> {
        if let Some(exceeded) = self.exceeded {
            return Err(exceeded.into());
        }
        if self.invalid {
            return Err(CaptureError::Invalid.into());
        }
        Ok(None)
    }
}

/// 采集规范化发生在哈希之前；不改变旧记录的写回格式集合。
pub(super) fn remove_redundant_dib(formats: &mut Vec<FormatData>) {
    if formats.iter().any(|format| format.name == "CF_DIBV5") {
        formats.retain(|format| format.name != "CF_DIB");
    }
}
