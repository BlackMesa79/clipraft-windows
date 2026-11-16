//! 仅扫描调用方指定的直接父目录；非空或未知暂存不自动删除。
use super::staging::Directory;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_DIRECTORY_ENTRIES: usize = 4096;
const MAX_CANDIDATES: usize = 256;
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Active,
    Empty,
    Retained,
    Unrecognized { reason: String },
}
#[derive(Debug, Serialize)]
pub struct Candidate {
    pub root: PathBuf,
    pub status: Status,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub candidates: Vec<Candidate>,
    pub truncated: bool,
}

pub fn scan(parent: &Path) -> Result<Report> {
    if !parent.is_absolute() {
        bail!("暂存扫描需要绝对父目录");
    }
    let parent = fs::canonicalize(parent)?;
    let mut report = Report {
        candidates: Vec::new(),
        truncated: false,
    };
    for (index, entry) in fs::read_dir(parent)?.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES || report.candidates.len() >= MAX_CANDIDATES {
            report.truncated = true;
            break;
        }
        let entry = entry?;
        if !candidate_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        report.candidates.push(Candidate {
            root: entry.path(),
            status: inspect(&entry.path()),
        });
    }
    report
        .candidates
        .sort_by(|left, right| left.root.cmp(&right.root));
    Ok(report)
}
fn candidate_name(name: &str) -> bool {
    [
        ".clipraft-snapshot-",
        ".clipraft-export-",
        ".clipraft-restore-",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}
fn inspect(root: &Path) -> Status {
    match Directory::claim(root) {
        Ok(None) => Status::Active,
        Ok(Some(directory)) => match directory.is_empty() {
            Ok(true) => Status::Empty,
            Ok(false) => Status::Retained,
            Err(error) => Status::Unrecognized {
                reason: format!("{error:#}"),
            },
        },
        Err(error) => Status::Unrecognized {
            reason: format!("{error:#}"),
        },
    }
}

/// 必须重新取得所有权锁，不能依赖此前扫描结果；只移除标记和空目录。
pub fn cleanup_empty(root: &Path) -> Result<()> {
    if !root.is_absolute() {
        bail!("暂存清理需要绝对目录");
    }
    let directory = Directory::claim(root)?.context("暂存任务仍在运行或正被其他清理任务使用")?;
    if !directory.is_empty()? {
        bail!("暂存仍含内容，已保留，不能按空目录清理");
    }
    directory.cleanup()
}

#[cfg(test)]
mod tests;
