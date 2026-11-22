use super::Storage;
use anyhow::{bail, Context, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

// 保留首批资源键，让已有背景直接成为普通布局的背景。
pub(super) const REGULAR_KEY: &str = "main_background_asset";
pub(super) const COMPACT_KEY: &str = "compact_background_asset";
const DEFAULT_SCRIM: u8 = 78;
const SCRIM_RANGE: std::ops::RangeInclusive<u8> = 65..=95;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BackgroundLayout {
    Regular,
    Compact,
}

impl BackgroundLayout {
    fn key(self) -> &'static str {
        match self {
            Self::Regular => REGULAR_KEY,
            Self::Compact => COMPACT_KEY,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Fit {
    #[default]
    Cover,
    Contain,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Position {
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackgroundOptions {
    fit: Fit,
    position: Position,
    scrim: u8,
}

impl Default for BackgroundOptions {
    fn default() -> Self {
        Self {
            fit: Fit::default(),
            position: Position::default(),
            scrim: DEFAULT_SCRIM,
        }
    }
}

impl BackgroundOptions {
    fn validate(self) -> Result<()> {
        if !SCRIM_RANGE.contains(&self.scrim) {
            bail!("背景遮罩强度须在 65% 至 95% 之间");
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MainBackground {
    pub(crate) options: BackgroundOptions,
    pub(crate) jpeg: String,
}

impl Storage {
    pub(crate) fn load_main_background(
        &self,
        layout: BackgroundLayout,
    ) -> Result<Option<MainBackground>> {
        let encoded: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [layout.key()],
                |row| row.get(0),
            )
            .optional()?;
        encoded
            .map(|encoded| {
                let asset: MainBackground =
                    serde_json::from_str(&encoded).context("背景图片设置已损坏")?;
                asset.options.validate()?;
                Ok(asset)
            })
            .transpose()
    }

    pub(crate) fn save_main_background(
        &self,
        layout: BackgroundLayout,
        asset: &MainBackground,
    ) -> Result<()> {
        asset.options.validate()?;
        self.conn.execute(
            "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [layout.key(), &serde_json::to_string(asset)?],
        )?;
        Ok(())
    }

    pub(crate) fn update_main_background_options(
        &self,
        layout: BackgroundLayout,
        options: BackgroundOptions,
    ) -> Result<()> {
        let mut asset = self
            .load_main_background(layout)?
            .context("请先选择背景图片")?;
        asset.options = options;
        self.save_main_background(layout, &asset)
    }

    pub(crate) fn remove_main_background(&self, layout: BackgroundLayout) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key=?1", [layout.key()])?;
        Ok(())
    }
}
