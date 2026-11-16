//! 将密码归档恢复成独立新库；发布完成仍不改变当前启动定位。
use super::{bundle::PreparedRestore, control::Control, staging::Directory};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub struct Request<'a> {
    pub archive: &'a Path,
    pub password: &'a [u8],
    pub destination: &'a Path,
}
#[derive(Debug)]
pub struct Outcome {
    pub root: PathBuf,
    pub cleanup_warning: Option<String>,
}

pub fn run(request: &Request<'_>, control: &Control) -> Result<Outcome> {
    run_with_capacity(request, control, crate::data_directory::capacity)
}
fn run_with_capacity(
    request: &Request<'_>,
    control: &Control,
    capacity: impl FnOnce(&Path, u64) -> Result<()>,
) -> Result<Outcome> {
    control.start()?;
    let result = restore(request, control, capacity);
    if result.is_err() {
        control.fail()?;
    }
    result
}
fn restore(
    request: &Request<'_>,
    control: &Control,
    capacity: impl FnOnce(&Path, u64) -> Result<()>,
) -> Result<Outcome> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
    let destination = destination_path(request.destination)?;
    let parent = destination.parent().context("恢复目标缺少父目录")?;
    let mut input = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(request.archive)?;
    control.check()?;
    let mut plan = PreparedRestore::new(&mut input, request.password)?;
    control.check()?;
    capacity(parent, plan.required_bytes())?;
    control.writing(plan.bytes())?;
    let directory = Directory::create(parent, "restore")?;
    let target = directory.path().join("database");
    let result = extract_and_publish(&mut input, &mut plan, (&target, &destination), control);
    finish(result, &plan, directory, (&target, destination))
}
fn destination_path(target: &Path) -> Result<PathBuf> {
    if !target.is_absolute() || target.file_name().is_none() {
        bail!("恢复目标必须是绝对新目录路径");
    }
    if target.try_exists()? {
        bail!("恢复目标已存在，不会覆盖或合并已有数据");
    }
    let parent = fs::canonicalize(target.parent().context("恢复目标缺少父目录")?)?;
    Ok(parent.join(target.file_name().context("恢复目标缺少名称")?))
}
fn extract_and_publish(
    input: &mut impl Read,
    plan: &mut PreparedRestore,
    paths: (&Path, &Path),
    control: &Control,
) -> Result<()> {
    plan.extract(input, paths.0, control)?;
    control.publish()?;
    crate::data_directory::move_new(paths.0, paths.1)?;
    control.finish()
}
fn finish(
    result: Result<()>,
    plan: &PreparedRestore,
    directory: Directory,
    paths: (&Path, PathBuf),
) -> Result<Outcome> {
    let (target, destination) = paths;
    if let Err(error) = result {
        let cleanup = plan.cleanup(target).and_then(|()| directory.cleanup());
        return match cleanup {
            Ok(()) => Err(error),
            Err(cleanup) => Err(error.context(format!("恢复暂存清理失败：{cleanup:#}"))),
        };
    }
    let cleanup_warning = directory
        .cleanup()
        .err()
        .map(|error| format!("新库已发布，暂存目录清理失败：{error:#}"));
    Ok(Outcome {
        root: destination,
        cleanup_warning,
    })
}

#[cfg(test)]
mod tests;
