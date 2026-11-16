//! 冻结快照的导出任务；调用方负责此前的存储锁与可迁移快照创建。
use super::{bundle::PreparedExport, control::Control};
use anyhow::{bail, Context, Result};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Outcome {
    pub cleanup_warning: Option<String>,
}

pub struct Request<'a> {
    pub snapshot: &'a Path,
    pub master: &'a [u8; 32],
    pub password: &'a [u8],
    pub destination: &'a Path,
}

/// 每个 Control 只运行一次；目标不得存在，不切换数据目录。
pub fn run(request: &Request<'_>, control: &Control) -> Result<Outcome> {
    run_with_capacity(request, control, crate::data_directory::capacity)
}

fn run_with_capacity(
    request: &Request<'_>,
    control: &Control,
    capacity: impl FnOnce(&Path, u64) -> Result<()>,
) -> Result<Outcome> {
    control.start()?;
    let result = export(request, control, capacity);
    if result.is_err() {
        control.fail()?;
    }
    result
}

/// 接续已经完成快照阶段的同一任务，不重复启动或提前结束状态。
pub(crate) fn run_prepared(request: &Request<'_>, control: &Control) -> Result<Outcome> {
    control.check()?;
    export(request, control, crate::data_directory::capacity)
}

fn export(
    request: &Request<'_>,
    control: &Control,
    capacity: impl FnOnce(&Path, u64) -> Result<()>,
) -> Result<Outcome> {
    let parent = destination_parent(request)?;
    let destination = parent.join(
        request
            .destination
            .file_name()
            .context("导出目标缺少文件名")?,
    );
    let request = Request {
        destination: &destination,
        ..*request
    };
    let plan = PreparedExport::new(request.snapshot, control)?;
    capacity(&parent, plan.bytes())?;
    control.writing(plan.bytes())?;
    let mut stage = Stage::create(&parent)?;
    let result = write_and_publish(&request, control, &plan, &mut stage);
    let cleaned = stage.cleanup();
    match (result, cleaned) {
        (Err(error), Err(cleanup)) => Err(error.context(format!("暂存清理失败：{cleanup:#}"))),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(cleanup)) => Ok(Outcome {
            cleanup_warning: Some(format!("归档已发布，暂存目录清理失败：{cleanup:#}")),
        }),
        (Ok(()), Ok(())) => Ok(Outcome {
            cleanup_warning: None,
        }),
    }
}

fn destination_parent(request: &Request<'_>) -> Result<PathBuf> {
    if !request.destination.is_absolute() || request.destination.file_name().is_none() {
        bail!("导出目标必须是绝对文件路径");
    }
    if request.destination.try_exists()? {
        bail!("导出目标已存在，不会覆盖已有文件");
    }
    let parent = fs::canonicalize(request.destination.parent().context("导出目标缺少父目录")?)?;
    if parent.starts_with(fs::canonicalize(request.snapshot)?) {
        bail!("导出目标不能位于快照目录内部");
    }
    Ok(parent)
}

fn write_and_publish(
    request: &Request<'_>,
    control: &Control,
    plan: &PreparedExport,
    stage: &mut Stage,
) -> Result<()> {
    let file = stage.output.as_mut().context("导出暂存文件已关闭")?;
    let mut output = ProgressWriter { file, control };
    plan.write(
        request.snapshot,
        request.master,
        &mut output,
        request.password,
    )?;
    file.sync_all()?;
    control.publish()?;
    stage.output.take();
    crate::data_directory::move_new(&stage.file(), request.destination)?;
    stage.published = true;
    control.finish()
}

struct ProgressWriter<'a> {
    file: &'a mut File,
    control: &'a Control,
}
impl Write for ProgressWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.control.check().map_err(io::Error::other)?;
        let count = self.file.write(bytes)?;
        self.control
            .advance(count as u64)
            .map_err(io::Error::other)?;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct Stage {
    root: PathBuf,
    output: Option<File>,
    directory: super::staging::Directory,
    published: bool,
}
impl Stage {
    fn create(parent: &Path) -> Result<Self> {
        Self::with_directory(super::staging::Directory::create(parent, "export")?)
    }
    #[cfg(test)]
    fn open(root: PathBuf) -> Result<Self> {
        Self::with_directory(super::staging::Directory::open(root)?)
    }
    fn with_directory(directory: super::staging::Directory) -> Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        let root = directory.path().to_path_buf();
        let output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(root.join("archive.part"));
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                return match directory.cleanup() {
                    Ok(()) => Err(error.into()),
                    Err(cleanup) => Err(anyhow::Error::from(error)
                        .context(format!("暂存初始化清理失败：{cleanup:#}"))),
                };
            }
        };
        Ok(Self {
            root,
            output: Some(output),
            directory,
            published: false,
        })
    }

    fn file(&self) -> PathBuf {
        self.root.join("archive.part")
    }
    fn cleanup(mut self) -> Result<()> {
        self.output.take();
        // 只清理本任务固定文件，不递归删除目录；出现其他内容时保留并报告。
        if !self.published && self.file().try_exists()? {
            fs::remove_file(self.file())?;
        }
        self.directory.cleanup()
    }
}

#[cfg(test)]
mod tests;
