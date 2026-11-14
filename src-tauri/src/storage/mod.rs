#[cfg(test)]
mod compression_tests;
pub mod crypto;
mod detail;
mod groups;
#[cfg(test)]
mod groups_tests;
pub use groups::GroupView;
mod maintenance;
#[cfg(test)]
mod maintenance_tests;
#[cfg(test)]
mod migration_tests;
mod migrations;
mod predict;
#[cfg(test)]
mod prediction_tests;
mod search;
#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod sensitive_tests;
mod sensitivity;
mod settings;
#[cfg(test)]
mod source_tests;
use crate::{
    model::{self, Clip, ItemView},
    thumbnail,
};
use anyhow::{bail, Context, Result};
pub use maintenance::{validate_retention_policy, DeleteReport, RetentionPolicy, ScanReport};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ListFilter {
    pub query: Option<String>,
    pub kind: Option<String>,
    pub favorite_only: bool,
    pub group_id: Option<i64>,
}

pub const PAGE_LIMIT: usize = 500;
pub const QUICK_PAGE_SIZE: usize = 200;

#[derive(Debug, PartialEq, Eq)]
pub struct SaveOutcome {
    pub id: String,
    pub inserted: bool,
}

struct SaveIdentity {
    id: String,
    dedup_key: String,
}
impl SaveIdentity {
    fn new(clip: &Clip) -> Self {
        let id = clip.hash();
        let dedup_key = clip.file_dedup_key().unwrap_or_else(|| id.clone());
        Self { id, dedup_key }
    }
}
struct BlobBatch<'a> {
    key: &'a [u8; 32],
    objects: HashMap<String, Vec<u8>>,
}
impl BlobBatch<'_> {
    fn add(&mut self, plain: &[u8]) -> Result<String> {
        let id = model::digest(plain);
        if !self.objects.contains_key(&id) {
            self.objects
                .insert(id.clone(), crypto::encrypt_blob(self.key, &id, plain)?);
        }
        Ok(id)
    }
}
struct StoredFormat {
    name: String,
    inline: Option<Vec<u8>>,
    blob: Option<String>,
}
struct PreparedSave {
    detection: Option<sensitivity::Detection>,
    source: Option<crate::capture_source::CaptureSource>,
    identity: SaveIdentity,
    view: ItemView,
    search: String,
    search_pinyin: String,
    is_link: bool,
    formats: Vec<StoredFormat>,
    blobs: HashMap<String, Vec<u8>>,
}
impl PreparedSave {
    fn new(clip: &Clip, identity: SaveIdentity, key: &[u8; 32]) -> Result<Self> {
        let mut blobs = BlobBatch {
            key,
            objects: HashMap::new(),
        };
        let files = clip
            .get("CF_HDROP")
            .and_then(model::decode_hdrop)
            .unwrap_or_default();
        let summary = if !files.is_empty() {
            files
                .iter()
                .map(|p| p.rsplit(['\\', '/']).next().unwrap_or(p))
                .collect::<Vec<_>>()
                .join("、")
        } else {
            clip.text().unwrap_or_else(|| {
                if clip.kind() == "image" {
                    "图片".into()
                } else {
                    "富文本".into()
                }
            })
        };
        let preview = thumbnail::generate(clip);
        let thumb = preview.png.as_ref().map(|b| blobs.add(b)).transpose()?;
        let view = ItemView {
            id: identity.id.clone(),
            kind: clip.kind().into(),
            summary: summary.chars().take(180).collect(),
            formats: clip.formats.iter().map(|f| f.name.clone()).collect(),
            thumbnail: thumb,
            preview: preview
                .large_png
                .as_ref()
                .map(|b| blobs.add(b))
                .transpose()?,
            placeholder: preview.placeholder,
            files,

            size: clip.formats.iter().map(|f| f.bytes.len() as i64).sum(),
            ..ItemView::default()
        };
        let mut stored = Vec::new();
        for f in &clip.formats {
            let external =
                model::image_priority().contains(&f.name.as_str()) || f.bytes.len() > 64 * 1024;
            stored.push(StoredFormat {
                name: f.name.clone(),
                inline: if external {
                    None
                } else {
                    Some(f.bytes.clone())
                },
                blob: if external {
                    Some(blobs.add(&f.bytes)?)
                } else {
                    None
                },
            });
        }

        let search = search_text(clip);
        let search_pinyin = search::pinyin_text(&search);
        let is_link = search::is_link(&view.kind, &clip.text().unwrap_or_default());
        Ok(Self {
            detection: None,
            source: None,
            identity,
            view,
            search,
            search_pinyin,
            is_link,
            formats: stored,
            blobs: blobs.objects,
        })
    }
}

pub struct Storage {
    conn: Connection,
    blobs: PathBuf,
    key: Zeroizing<[u8; 32]>,
}
impl Storage {
    pub fn open(root: &Path, master: &[u8; 32]) -> Result<Self> {
        Self::open_with_hook(root, master, |_| Ok(()))
    }
    fn open_with_hook(
        root: &Path,
        master: &[u8; 32],
        hook: impl FnMut(&rusqlite::Transaction<'_>) -> Result<()>,
    ) -> Result<Self> {
        fs::create_dir_all(root.join("data/blobs"))?;
        let conn = Connection::open(root.join("history.db"))?;
        // SQLCipher 原始密钥避免口令编码歧义；该 SQL 从不写入日志。
        let hex: Zeroizing<String> =
            Zeroizing::new(master.iter().map(|b| format!("{b:02x}")).collect());
        conn.pragma_update(None, "key", format!("x'{}'", hex.as_str()))?;
        let version: String = conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .context("SQLCipher 未启用")?;
        if version.is_empty() {
            bail!("SQLCipher 未启用");
        }
        // 先读取已有库页，区分密钥不匹配与普通 IO、权限或锁冲突。
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|error| {
            if error.sqlite_error_code() == Some(rusqlite::ErrorCode::NotADatabase) {
                anyhow::Error::new(crate::platform::DataError::DatabaseKey)
            } else {
                anyhow::Error::new(error)
            }
        })?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA temp_store=MEMORY; PRAGMA secure_delete=ON; PRAGMA foreign_keys=ON;")?;
        let mut storage = Self {
            conn,
            blobs: root.join("data/blobs"),
            key: crypto::derive(master)?,
        };
        migrations::upgrade(&mut storage, root, hook)?;
        Ok(storage)
    }
    #[cfg(test)]
    fn write_blob(&self, plain: &[u8]) -> Result<String> {
        let id = model::digest(plain);
        self.install_blob(&id, &crypto::encrypt_blob(&self.key, &id, plain)?)?;
        Ok(id)
    }
    fn install_blob(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let dest = self.blobs.join(id);
        if !dest.exists() {
            let temp = self.blobs.join(format!("{id}.tmp"));
            fs::write(&temp, bytes)?;
            fs::rename(temp, dest)?;
        }
        Ok(())
    }
    pub fn read_thumbnail(&self, id: &str) -> Result<Vec<u8>> {
        if !model::is_hash(id) {
            bail!("无效对象标识");
        }
        // 协议只访问登记过的缩略图或预览，不能通过猜测哈希读取其他对象。
        let allowed: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM items WHERE (thumbnail=?1 OR preview=?1) AND sensitive<>1) AND NOT EXISTS(SELECT 1 FROM items WHERE (thumbnail=?1 OR preview=?1) AND sensitive=1)",
            [id],
            |r| r.get(0),
        )?;
        if !allowed {
            bail!("预览未登记或记录不存在");
        }
        let bytes = fs::read(self.blobs.join(id)).map_err(|error| {
            anyhow::anyhow!(if error.kind() == std::io::ErrorKind::NotFound {
                "预览对象文件不存在"
            } else {
                "预览对象文件读取失败"
            })
        })?;
        crypto::decrypt(&self.key, id, &bytes)
            .map_err(|_| anyhow::anyhow!("预览对象解密失败（密钥不匹配或数据损坏）"))
    }
    fn read_blob(&self, id: &str) -> Result<Vec<u8>> {
        if !model::is_hash(id) {
            bail!("无效对象标识");
        }
        crypto::decrypt(&self.key, id, &fs::read(self.blobs.join(id))?)
    }
    /// 兼容原有调用者；快速面板只缓存第一页。
    pub fn list(&self) -> Result<Vec<ItemView>> {
        self.list_page(&ListFilter::default(), 0, QUICK_PAGE_SIZE)
    }
    /// 只查询快速面板快照中的 id，不受历史第一页缓存或排序影响。
    pub fn existing_panel_ids(&self, ids: &[&str]) -> Result<HashSet<String>> {
        if ids.len() > 9 {
            bail!("快速面板快照不能超过 9 条");
        }
        if ids.is_empty() {
            return Ok(HashSet::new());
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id FROM items WHERE id IN ({placeholders})"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |row| row.get(0))?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()
            .map_err(Into::into)
    }
    /// 独立预览按 id 获取安全视图，不受快速面板第一页缓存限制。
    pub fn preview_item(&self, id: &str) -> Result<ItemView> {
        Ok(self.conn.query_row("SELECT view,id,kind,created,used_at,size,pinned,favorite,group_id,source_exe,source_title,sensitive,expires_at,thumbnail,preview FROM items WHERE id=?1", [id], item_view)?)
    }
    pub fn list_page(
        &self,
        filter: &ListFilter,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ItemView>> {
        let mut sql = String::from("SELECT view,id,kind,created,used_at,size,pinned,favorite,group_id,source_exe,source_title,sensitive,expires_at,thumbnail,preview FROM items WHERE 1=1");
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(kind) = &filter.kind {
            match kind.as_str() {
                "text" => sql.push_str(" AND kind IN ('text','rich')"),
                "link" => sql.push_str(" AND is_link=1"),
                // 仅按已保存的路径类型识别，不依赖缩略图或实时文件访问；EXISTS 避免分页重复。
                "image" => sql.push_str(" AND (kind='image' OR (kind='files' AND EXISTS (SELECT 1 FROM json_each(items.view,'$.files') f WHERE lower(f.value) GLOB '*.png' OR lower(f.value) GLOB '*.jpg' OR lower(f.value) GLOB '*.jpeg' OR lower(f.value) GLOB '*.gif' OR lower(f.value) GLOB '*.bmp' OR lower(f.value) GLOB '*.webp')))"),
                _ => {
                    sql.push_str(" AND kind=?");
                    values.push(kind.clone().into());
                }
            }
        }
        if filter.favorite_only {
            sql.push_str(" AND favorite=1");
        }
        if let Some(group) = filter.group_id {
            if group == -1 {
                sql.push_str(" AND group_id IS NULL");
            } else {
                sql.push_str(" AND group_id=?");
                values.push(group.into());
            }
        }
        search::append_query(&mut sql, &mut values, filter.query.as_deref());
        sql.push_str(" ORDER BY pinned DESC,touched DESC,id ASC LIMIT ? OFFSET ?");
        values.push((limit.min(PAGE_LIMIT) as i64).into());
        values.push(i64::try_from(offset).unwrap_or(i64::MAX).into());
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(values), item_view)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
    pub fn save(&mut self, clip: &Clip) -> Result<()> {
        self.save_with_outcome(clip).map(|_| ())
    }
    /// 无共享锁的调用者保留原接口；runtime 必须使用 save_shared。
    pub fn save_with_outcome(&mut self, clip: &Clip) -> Result<bool> {
        let identity = SaveIdentity::new(clip);
        let detection = sensitivity::Detection::for_clip(clip, &self.load_settings()?);
        if self
            .update_duplicate(&identity, None, detection.as_ref())?
            .is_some()
        {
            return Ok(false);
        }
        let mut prepared = PreparedSave::new(clip, identity, &self.key)?;
        prepared.detection = detection;
        self.commit_prepared(prepared)
            .map(|outcome| outcome.inserted)
    }
    /// 采集路径：锁内查重 → 锁外生成预览/压缩/加密 → 锁内复查并提交。
    pub fn save_shared(storage: &Mutex<Self>, clip: &Clip) -> Result<SaveOutcome> {
        Self::save_shared_with_source(storage, clip, None)
    }
    pub(crate) fn save_shared_with_source(
        storage: &Mutex<Self>,
        clip: &Clip,
        source: Option<&crate::capture_source::CaptureSource>,
    ) -> Result<SaveOutcome> {
        let identity = SaveIdentity::new(clip);
        let (key, detection) = {
            let store = storage
                .lock()
                .map_err(|_| anyhow::anyhow!("存储锁不可用"))?;
            let detection = sensitivity::Detection::for_clip(clip, &store.load_settings()?);
            if let Some(id) = store.update_duplicate(&identity, source, detection.as_ref())? {
                return Ok(SaveOutcome {
                    id,
                    inserted: false,
                });
            }
            (store.key.clone(), detection)
        };
        // 只准备内存对象；尚未登记的文件不能提前暴露给孤儿扫描。
        let mut prepared = PreparedSave::new(clip, identity, &key)?;
        prepared.source = source.cloned();
        prepared.detection = detection;
        storage
            .lock()
            .map_err(|_| anyhow::anyhow!("存储锁不可用"))?
            .commit_prepared(prepared)
    }
    fn update_duplicate(
        &self,
        identity: &SaveIdentity,
        source: Option<&crate::capture_source::CaptureSource>,
        detection: Option<&sensitivity::Detection>,
    ) -> Result<Option<String>> {
        let now = unix_millis();
        let order = self.next_order()?;
        // RETURNING 返回实际命中的旧 id；文件去重命中时它可能不同于本次 clip.hash()。
        let tx = self.conn.unchecked_transaction()?;
        let result: Option<String> = tx.query_row(
            "UPDATE items SET touched=?1,used_at=?3,dedup_key=?2,source_exe=CASE WHEN ?5 THEN ?6 ELSE source_exe END,source_title=CASE WHEN ?5 THEN ?7 ELSE source_title END WHERE id=(SELECT id FROM items WHERE dedup_key=?2 OR id=?4 ORDER BY touched DESC,id ASC LIMIT 1) RETURNING id",
            params![order, identity.dedup_key, now, identity.id, source.is_some(), source.and_then(|s| s.exe.as_deref()), source.and_then(|s| s.title.as_deref())],
            |row| row.get(0),
        ).optional()?;
        if let (Some(id), Some(detection)) = (&result, detection) {
            sensitivity::apply_detection(&tx, id, detection, now)?;
        }
        tx.commit()?;
        Ok(result)
    }
    fn next_order(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(MAX(touched),0)+1 FROM items", [], |r| {
                r.get(0)
            })?)
    }
    fn commit_prepared(&mut self, mut prepared: PreparedSave) -> Result<SaveOutcome> {
        // 锁外准备期间可能有另一条相同记录完成保存；仍保留首次保存内容。
        if let Some(id) = self.update_duplicate(
            &prepared.identity,
            prepared.source.as_ref(),
            prepared.detection.as_ref(),
        )? {
            return Ok(SaveOutcome {
                id,
                inserted: false,
            });
        }
        for (id, bytes) in &prepared.blobs {
            self.install_blob(id, bytes)?;
        }
        let order = self.next_order()?;
        let now = unix_millis();
        prepared.view.created = now;
        prepared.view.used_at = now;
        if let Some(detection) = &prepared.detection {
            prepared.search.clear();
            prepared.search_pinyin.clear();
            prepared.view.summary = detection.summary.clone();
        }
        let view = &prepared.view;
        let id = &prepared.identity.id;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO items(id,view,touched,kind,created,used_at,size,search_text,thumbnail,preview,dedup_key,search_pinyin,is_link,source_exe,source_title) VALUES(?1,?2,?3,?4,?5,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![id, display_json(view)?, order, view.kind, now, view.size, prepared.search, view.thumbnail, view.preview, prepared.identity.dedup_key, prepared.search_pinyin, prepared.is_link, prepared.source.as_ref().and_then(|s| s.exe.as_deref()), prepared.source.as_ref().and_then(|s| s.title.as_deref())],
        )?;
        if let Some(detection) = &prepared.detection {
            sensitivity::apply_detection(&tx, id, detection, now)?;
        }
        for format in prepared.formats {
            tx.execute(
                "INSERT INTO formats(item,name,inline,blob) VALUES(?1,?2,?3,?4)",
                params![id, format.name, format.inline, format.blob],
            )?;
        }
        tx.commit()?;
        Ok(SaveOutcome {
            id: prepared.identity.id,
            inserted: true,
        })
    }
    /// 只更新已有记录的顺序；不加载格式、不哈希内容、不重写加密对象。
    pub fn touch(&self, id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE items SET touched=(SELECT COALESCE(MAX(touched),0)+1 FROM items),used_at=?2 WHERE id=?1",
            params![id, unix_millis()],
        )?;
        if changed == 0 {
            bail!("记录不存在");
        }
        Ok(())
    }
    pub fn load(&self, id: &str) -> Result<Clip> {
        let mut stmt = self
            .conn
            .prepare("SELECT name,inline,blob FROM formats WHERE item=?1 ORDER BY name")?;
        let rows = stmt.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<Vec<u8>>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut clip = Clip::default();
        for row in rows {
            let (name, inline, blob) = row?;
            let bytes = match (inline, blob) {
                (Some(b), _) => b,
                (_, Some(b)) => self.read_blob(&b)?,
                _ => bail!("记录格式不完整"),
            };
            clip.formats.push(model::FormatData { name, bytes });
        }
        if clip.formats.is_empty() {
            bail!("记录不存在");
        }
        Ok(clip)
    }
}
pub(super) fn unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// JSON 只保存展示字段，独立列不在两处重复保存。
fn display_json(view: &ItemView) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::json!({
        "id": view.id, "summary": view.summary, "formats": view.formats,
        "placeholder": view.placeholder, "files": view.files
    }))?)
}

fn truncate_search_text(mut text: String) -> String {
    let mut end = text.len().min(16 * 1024);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

fn search_text(clip: &Clip) -> String {
    let text = match clip.kind() {
        "image" => String::new(),
        "files" => clip
            .get("CF_HDROP")
            .and_then(model::decode_hdrop)
            .unwrap_or_default()
            .iter()
            .map(|path| {
                format!(
                    "{}\n{}",
                    path.rsplit(['\\', '/']).next().unwrap_or(path),
                    path
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => clip.text().unwrap_or_default(),
    };
    truncate_search_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    // 此查询只依赖 items.id；内存库避免读取真实记录或创建 blob 文件。
    fn panel_id_storage() -> Storage {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY)")
            .unwrap();
        Storage {
            conn,
            blobs: PathBuf::new(),
            key: Zeroizing::new([0; 32]),
        }
    }
    #[test]
    fn panel_ids_include_records_outside_cached_page_and_exclude_deleted_records() {
        let s = panel_id_storage();
        for index in 0..=QUICK_PAGE_SIZE {
            s.conn
                .execute(
                    "INSERT INTO items(id) VALUES (?1)",
                    [format!("item-{index:03}")],
                )
                .unwrap();
        }
        // 最旧记录已被挤出整个缓存页，仍必须保留在呼出时的快照中。
        let in_cached_page: bool = s.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM (SELECT id FROM items ORDER BY id DESC LIMIT ?1) WHERE id='item-000')",
            [QUICK_PAGE_SIZE], |row| row.get(0),
        ).unwrap();
        assert!(!in_cached_page);
        assert_eq!(
            s.existing_panel_ids(&["item-000", "item-001", "missing"])
                .unwrap(),
            HashSet::from(["item-000".into(), "item-001".into()])
        );
        s.conn
            .execute("DELETE FROM items WHERE id=?1", ["item-001"])
            .unwrap();
        assert_eq!(
            s.existing_panel_ids(&["item-000", "item-001"]).unwrap(),
            HashSet::from(["item-000".into()])
        );
    }
    #[test]
    fn panel_ids_handle_empty_and_maximum_snapshot_sizes() {
        let s = panel_id_storage();
        assert!(s.existing_panel_ids(&[]).unwrap().is_empty());
        let ids: Vec<_> = (0..9).map(|index| format!("item-{index}")).collect();
        for id in &ids {
            s.conn
                .execute("INSERT INTO items(id) VALUES (?1)", [id])
                .unwrap();
        }
        let mut requested: Vec<_> = ids.iter().map(String::as_str).collect();
        assert_eq!(
            s.existing_panel_ids(&requested).unwrap(),
            ids.iter().cloned().collect()
        );
        requested.push("tenth");
        assert!(s.existing_panel_ids(&requested).is_err());
    }
    #[test]
    fn encrypted_persistence_and_dedup() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!("storage-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut s = Storage::open(&root, &[31; 32]).unwrap();
        let c = Clip {
            formats: vec![model::FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: "合成测试记录"
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            }],
        };
        s.save(&c).unwrap();
        s.save(&c).unwrap();
        assert_eq!(s.list().unwrap().len(), 1);
        assert_eq!(s.load(&c.hash()).unwrap().hash(), c.hash());
        drop(s);
        assert_ne!(
            &fs::read(root.join("history.db")).unwrap()[..16],
            b"SQLite format 3\0"
        );
        let error = Storage::open(&root, &[32; 32]).err().unwrap();
        assert_eq!(
            error.downcast_ref::<crate::platform::DataError>(),
            Some(&crate::platform::DataError::DatabaseKey)
        );
        assert!(error.to_string().contains("数据库密钥不匹配"));
        assert!(error.to_string().contains("请勿删除数据"));
        assert_eq!(
            Storage::open(&root, &[31; 32])
                .unwrap()
                .list()
                .unwrap()
                .len(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use std::{
        io::Cursor,
        time::{SystemTime, UNIX_EPOCH},
    };
    fn root(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "preview-{name}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn png() -> Vec<u8> {
        let image = image::RgbaImage::from_fn(1000, 400, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 64, 128])
        });
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }
    fn image_clip(bytes: Vec<u8>) -> Clip {
        Clip {
            formats: vec![model::FormatData {
                name: "PNG".into(),
                bytes,
            }],
        }
    }
    fn files(paths: &[String]) -> Clip {
        Clip {
            formats: vec![model::FormatData {
                name: "CF_HDROP".into(),
                bytes: model::encode_hdrop(paths),
            }],
        }
    }
    #[test]
    fn registered_preview_is_encrypted_limited_and_readable_after_reopen() {
        let root = root("registered");
        let bytes = png();
        let original = model::digest(&bytes);
        let mut store = Storage::open(&root, &[17; 32]).unwrap();
        store.save(&image_clip(bytes)).unwrap();
        let view = store.list().unwrap().remove(0);
        let id = view.preview.unwrap();
        assert_ne!(view.thumbnail.as_ref(), Some(&id));
        let decoded = image::load_from_memory(&store.read_thumbnail(&id).unwrap()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (800, 320));
        assert_eq!(decoded.to_rgba8().get_pixel(0, 0).0[3], 128);
        let encrypted = fs::read(root.join("data/blobs").join(&id)).unwrap();
        assert!(encrypted.starts_with(b"CM01"));
        assert!(!encrypted.starts_with(b"\x89PNG"));
        assert!(store.read_thumbnail(&original).is_err());
        assert!(store.read_thumbnail(&"0".repeat(64)).is_err());
        assert!(store.read_thumbnail("../history.db").is_err());
        drop(store);
        let store = Storage::open(&root, &[17; 32]).unwrap();
        assert!(store.read_thumbnail(&id).is_ok());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn removing_preview_registration_revokes_protocol_access_and_old_views_load() {
        let root = root("legacy");
        let mut store = Storage::open(&root, &[18; 32]).unwrap();
        store.save(&image_clip(png())).unwrap();
        let view = store.list().unwrap().remove(0);
        let preview = view.preview.unwrap();
        store
            .conn
            .execute("UPDATE items SET preview=NULL", [])
            .unwrap();
        let old = store.list().unwrap().remove(0);
        assert!(old.preview.is_none());
        assert!(store.read_thumbnail(&old.thumbnail.unwrap()).is_ok());
        assert!(store.read_thumbnail(&preview).is_err());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn first_image_file_gets_two_previews_without_changing_hdrop() {
        let root = root("file");
        let source = root.join("测试.PNG");
        fs::write(&source, png()).unwrap();
        let clip = files(&[source.to_string_lossy().into_owned()]);
        let mut store = Storage::open(&root, &[19; 32]).unwrap();
        store.save(&clip).unwrap();
        let view = store.list().unwrap().remove(0);
        assert_eq!(view.kind, "files");
        assert!(view.thumbnail.is_some());
        assert!(view.preview.is_some());
        assert_eq!(
            store.load(&clip.hash()).unwrap().get("CF_HDROP"),
            clip.get("CF_HDROP")
        );
        fs::remove_file(source).unwrap();
        assert!(store.read_thumbnail(&view.preview.unwrap()).is_ok());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unreadable_or_oversize_image_files_do_not_prevent_saving() {
        let root = root("file-errors");
        let oversized = root.join("large.png");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(50 * 1024 * 1024 + 1)
            .unwrap();
        let corrupt = root.join("invalid.jpg");
        fs::write(&corrupt, b"not an image").unwrap();
        let mut store = Storage::open(&root, &[20; 32]).unwrap();
        for (path, reason) in [
            (root.join("missing.webp"), "读取失败"),
            (oversized, "50 MiB"),
            (corrupt, "无法解码"),
        ] {
            let clip = files(&[path.to_string_lossy().into_owned()]);
            store.save(&clip).unwrap();
            let view = store.list().unwrap().remove(0);
            assert!(view.thumbnail.is_none() && view.preview.is_none());
            assert!(view.placeholder.unwrap().contains(reason));
            assert!(store.load(&clip.hash()).is_ok());
        }
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn only_the_first_file_is_considered_for_image_preview() {
        let root = root("first");
        let image = root.join("second.png");
        fs::write(&image, png()).unwrap();
        let clip = files(&[
            root.join("first.txt").to_string_lossy().into_owned(),
            image.to_string_lossy().into_owned(),
        ]);
        let result = thumbnail::generate(&clip);
        assert!(result.png.is_none() && result.large_png.is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_preview_returns_specific_reason() {
        let root = root("missing-blob");
        let mut store = Storage::open(&root, &[21; 32]).unwrap();
        store.save(&image_clip(png())).unwrap();
        let id = store.list().unwrap().remove(0).preview.unwrap();
        fs::remove_file(root.join("data/blobs").join(&id)).unwrap();
        assert_eq!(
            store.read_thumbnail(&id).unwrap_err().to_string(),
            "预览对象文件不存在"
        );
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod touch_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn store() -> (PathBuf, Storage) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "touch-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let storage = Storage::open(&root, &[41; 32]).unwrap();
        (root, storage)
    }
    fn clip(value: u8) -> Clip {
        Clip {
            formats: vec![model::FormatData {
                name: "Rich Text Format".into(),
                bytes: vec![value; 70 * 1024],
            }],
        }
    }
    #[test]
    fn touch_reorders_without_changing_metadata_formats_or_blobs() {
        let (root, mut storage) = store();
        let first = clip(65);
        let second = clip(66);
        let id = first.hash();
        storage.save(&first).unwrap();
        storage.save(&second).unwrap();
        let view_before: String = storage
            .conn
            .query_row("SELECT view FROM items WHERE id=?1", [&id], |r| r.get(0))
            .unwrap();
        let blobs_before: Vec<_> = fs::read_dir(&storage.blobs)
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        assert_eq!(storage.list().unwrap()[0].id, second.hash());
        storage.touch(&id).unwrap();
        storage.touch(&id).unwrap();
        let items = storage.list().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, id);
        assert_eq!(storage.load(&id).unwrap().formats, first.formats);
        let (view_after, touched): (String, i64) = storage
            .conn
            .query_row("SELECT view,touched FROM items WHERE id=?1", [&id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(view_before, view_after);
        assert_eq!(touched, 4);
        assert_eq!(
            fs::read_dir(&storage.blobs).unwrap().count(),
            blobs_before.len()
        );
        for (path, bytes) in blobs_before {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn touch_does_not_read_or_recreate_blob() {
        let (root, mut storage) = store();
        let clip = clip(65);
        let id = clip.hash();
        storage.save(&clip).unwrap();
        // 删除本测试创建的对象，确认置顶路径完全不读取格式数据。
        let blob = storage.blobs.join(model::digest(&clip.formats[0].bytes));
        fs::remove_file(&blob).unwrap();
        storage.touch(&id).unwrap();
        assert!(!blob.exists());
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn touching_missing_id_errors_without_inserting_or_reordering() {
        let (root, mut storage) = store();
        storage.save(&clip(65)).unwrap();
        for id in ["", "missing", "' OR 1=1 --"] {
            assert_eq!(storage.touch(id).unwrap_err().to_string(), "记录不存在");
        }
        assert_eq!(storage.list().unwrap().len(), 1);
        let touched: i64 = storage
            .conn
            .query_row("SELECT touched FROM items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(touched, 1);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }
}

fn item_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<ItemView> {
    let json: String = r.get(0)?;
    let mut view: ItemView = serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    view.id = r.get(1)?;
    view.kind = r.get(2)?;
    view.created = r.get(3)?;
    view.used_at = r.get(4)?;
    view.size = r.get(5)?;
    view.pinned = r.get(6)?;
    view.favorite = r.get(7)?;
    view.group_id = r.get(8)?;
    view.source_exe = r.get(9)?;
    view.source_title = r.get(10)?;
    view.sensitive = r.get::<_, i64>(11)? == 1;
    view.expires_at = r.get(12)?;
    view.thumbnail = r.get(13)?;
    view.preview = r.get(14)?;
    if view.sensitive {
        if !serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .is_some_and(|v| v.get("sensitive_hits").is_some())
        {
            view.summary = "••••••••".into();
        }
        view.thumbnail = None;
        view.preview = None;
        view.placeholder = None;
        view.files.clear();
        view.source_title = None;
    }
    Ok(view)
}
