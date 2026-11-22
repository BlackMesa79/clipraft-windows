//! 启动定位、目录独占和重启后迁移；不在运行时替换 Storage。
mod activation;
mod backup;
pub(crate) mod cleanup;
pub(crate) mod commands;
mod config;
mod database;
mod dialog;
pub(crate) use dialog::{select_archive, select_archive_folder, select_background};
mod files;
mod migration;
mod paths;
mod replacement;
mod restore;
#[cfg(test)]
mod tests;
use anyhow::{bail, Context, Result};
pub(crate) use config::move_new;
use config::Configuration;
pub(crate) use files::plain_metadata;
pub(crate) use paths::capacity;
use paths::Locations;
use std::{
    fs::{self, File},
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub(crate) struct Session {
    pub startup_capture: crate::capture_policy::StartupCapture,
    pub root: PathBuf,
    pub cache: PathBuf,
    pub control: Arc<DirectoryControl>,
    _profile_lock: Option<File>,
    _data_lock: File,
    _legacy_lock: Option<File>,
}
pub(crate) struct DirectoryControl {
    locations: Locations,
    root: PathBuf,
    plan: Mutex<Option<commands::Plan>>,
}
impl Session {
    pub(crate) fn persist_location(&self) -> Result<()> {
        if self.control.locations.config.is_none() {
            return Ok(());
        }
        let mut config = self.control.locations.read()?;
        config.root = self.root.clone();
        self.control.locations.save(&config)
    }
    pub fn open() -> Result<Self> {
        let locations = Locations::environment()?;
        Self::open_at(locations)
    }
    fn open_at(locations: Locations) -> Result<Self> {
        fs::create_dir_all(&locations.default_root)?;
        let profile_lock = profile_lock(&locations)?;
        let legacy_lock = match &locations.legacy_config {
            Some(path) => Some(lock_profile_file(path)?),
            None => None,
        };
        let loaded = load_data(&locations);
        let (config, data_lock) = match loaded {
            Ok(result) => result,
            Err(error) => recover(&locations, &error)?,
        };
        let root = fs::canonicalize(config.root)?;
        Ok(Self {
            startup_capture: config.startup_capture,
            root: root.clone(),
            cache: locations.cache.clone(),
            control: Arc::new(DirectoryControl {
                locations,
                root,
                plan: Mutex::new(None),
            }),
            _profile_lock: profile_lock,
            _data_lock: data_lock,
            _legacy_lock: legacy_lock,
        })
    }
}
fn profile_lock(locations: &Locations) -> Result<Option<File>> {
    let Some(config) = &locations.config else {
        return Ok(None);
    };
    Ok(Some(lock_profile_file(config)?))
}
fn lock_profile_file(config: &std::path::Path) -> Result<File> {
    let parent = config.parent().context("启动配置缺少父目录")?;
    fs::create_dir_all(parent)?;
    let path = parent.join(".clipraft-startup.lock");
    if path.exists() {
        files::plain_metadata(&path)?;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    files::wait_for_lock(&file).context("当前用户配置已由另一实例使用")?;
    Ok(file)
}
fn load_data(locations: &Locations) -> Result<(Configuration, File)> {
    let config = locations.read()?;
    let root = fs::canonicalize(&config.root).with_context(|| {
        format!(
            "无法访问数据目录 {}；不会自动新建空库",
            config.root.display()
        )
    })?;
    if config.replacement.is_none()
        && (locations.configured()? || locations.cache.try_exists()?)
        && (!root.join("history.db").is_file() || !root.join("key.bin").is_file())
    {
        bail!(
            "已配置的数据目录缺少数据库或密钥，拒绝新建空库：{}",
            root.display()
        );
    }
    let source_lock = files::startup_lock(&root)?;
    if config.replacement.is_some() {
        let committed = replacement::execute(locations, &config)?;
        return Ok((committed, source_lock));
    }
    if config.pending.is_none() {
        return Ok((config, source_lock));
    }
    let progress = dialog::MigrationProgress::new()?;
    migration::execute(locations, &config, &mut |step, done, total| {
        progress.update(step, done, total)
    })
}
fn recover(locations: &Locations, error: &anyhow::Error) -> Result<(Configuration, File)> {
    if std::env::var_os("CLIPBOARD_M0_DATA_DIR").is_some()
        || std::env::var_os("CLIPBOARD_M0_DIAG").is_some()
    {
        bail!("{error:#}");
    }
    let config_path = locations
        .config
        .as_ref()
        .context("隔离环境不支持恢复正式目录")?;
    if !dialog::offer_recovery(&format!("{error:#}\n启动定位：{}", config_path.display())) {
        bail!("{error:#}");
    }
    let selected = dialog::select_existing()?.context("已取消数据目录恢复，原数据与配置未更改")?;
    let root = fs::canonicalize(selected)?;
    let guard = files::lock(&root)?;
    database::checkpoint(&root).context("无法整理所选目录的 WAL，原配置未更改")?;
    database::verify(&root).context("所选目录不是可解密的完整数据目录，原启动配置未更改")?;
    let config = Configuration::initial(root);
    locations.save(&config)?;
    Ok((config, guard))
}
