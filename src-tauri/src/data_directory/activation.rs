//! 切换到已恢复的新库；仅提交启动定位，保留当前库与所有原文件。
use super::{files, paths, DirectoryControl};
use anyhow::{bail, Context, Result};
use std::{fs::File, path::Path};

impl DirectoryControl {
    pub(crate) fn acknowledge_restored_startup(&self) -> Result<()> {
        if self.locations.config.is_none() {
            return Ok(());
        }
        let _plan = self
            .plan
            .lock()
            .map_err(|_| anyhow::anyhow!("目录操作状态不可用"))?;
        let mut config = self.active_configuration()?;
        if config.startup_capture == crate::capture_policy::StartupCapture::Existing {
            return Ok(());
        }
        config.startup_capture = crate::capture_policy::StartupCapture::Existing;
        self.locations.save(&config)
    }
    pub(crate) fn activate_restored(&self, target: &Path) -> Result<File> {
        if self.locations.config.is_none() {
            bail!("当前隔离环境未配置启动定位，不能切换数据库");
        }
        let mut planned = self
            .plan
            .lock()
            .map_err(|_| anyhow::anyhow!("迁移预检状态不可用"))?;
        let mut config = self.active_configuration()?;
        if config.cleanup.as_ref().is_some_and(|plan| !plan.complete) {
            bail!("迁移源清理尚未完成，请先处理待清理项目");
        }
        if config.pending.is_some() {
            bail!("已有待执行迁移，请先完成迁移");
        }
        let target = paths::target_path(&self.root, target)?;
        let guard = files::lock(&target)?;
        for name in ["history.db", "key.bin"] {
            if !files::plain_metadata(&target.join(name))?.is_file() {
                bail!("恢复目录缺少普通数据库或密钥文件");
            }
        }
        // 提交前重新验证，不能把后台曾经成功当作当前文件仍完整的证明。
        files::inventory(&target)?;
        let master = crate::platform::master_key(&target)?;
        crate::storage::Storage::verify_snapshot(&target, &master)
            .context("恢复库重新验证失败，当前库未切换")?;
        config.previous = Some(self.root.clone());
        config.root = target;
        config.startup_capture = crate::capture_policy::StartupCapture::ChangesOnly;
        self.locations
            .save(&config)
            .context("无法提交恢复库启动位置，原库仍保留")?;
        *planned = None;
        Ok(guard)
    }
}
