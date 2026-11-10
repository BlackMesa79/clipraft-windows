pub mod crypto;
use crate::{
    model::{self, Clip, ItemView},
    thumbnail,
};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use std::{
    fs,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub struct Storage {
    conn: Connection,
    blobs: PathBuf,
    key: Zeroizing<[u8; 32]>,
}
impl Storage {
    pub fn open(root: &Path, master: &[u8; 32]) -> Result<Self> {
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
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA temp_store=MEMORY; PRAGMA secure_delete=ON;
            CREATE TABLE IF NOT EXISTS items(id TEXT PRIMARY KEY, view TEXT NOT NULL, touched INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS formats(item TEXT NOT NULL REFERENCES items(id), name TEXT NOT NULL, inline BLOB, blob TEXT, PRIMARY KEY(item,name));")?;
        Ok(Self {
            conn,
            blobs: root.join("data/blobs"),
            key: crypto::derive(master),
        })
    }
    fn write_blob(&self, plain: &[u8]) -> Result<String> {
        let id = model::digest(plain);
        let dest = self.blobs.join(&id);
        if !dest.exists() {
            let bytes = crypto::encrypt(&self.key, &id, plain)?;
            let temp = self.blobs.join(format!("{id}.tmp"));
            fs::write(&temp, bytes)?;
            fs::rename(temp, dest)?;
        }
        Ok(id)
    }
    pub fn read_thumbnail(&self, id: &str) -> Result<Vec<u8>> {
        if !model::is_hash(id) {
            bail!("无效对象标识");
        }
        // 协议只访问登记过的缩略图或预览，不能通过猜测哈希读取其他对象。
        let allowed: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM items WHERE json_extract(view,'$.thumbnail')=?1 OR json_extract(view,'$.preview')=?1)",
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
    pub fn list(&self) -> Result<Vec<ItemView>> {
        let mut stmt = self
            .conn
            .prepare("SELECT view FROM items ORDER BY touched DESC LIMIT 200")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn save(&mut self, clip: &Clip) -> Result<()> {
        let id = clip.hash();
        let order: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(touched),0)+1 FROM items", [], |r| {
                    r.get(0)
                })?;
        if self.conn.execute(
            "UPDATE items SET touched=?1 WHERE id=?2",
            params![order, id],
        )? > 0
        {
            return Ok(());
        }
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
        let thumb = preview
            .png
            .as_ref()
            .map(|b| self.write_blob(b))
            .transpose()?;
        let view = ItemView {
            id: id.clone(),
            kind: clip.kind().into(),
            summary: summary.chars().take(180).collect(),
            formats: clip.formats.iter().map(|f| f.name.clone()).collect(),
            thumbnail: thumb,
            preview: preview
                .large_png
                .as_ref()
                .map(|b| self.write_blob(b))
                .transpose()?,
            placeholder: preview.placeholder,
            files,
        };
        let mut stored = Vec::new();
        for f in &clip.formats {
            let external =
                model::image_priority().contains(&f.name.as_str()) || f.bytes.len() > 64 * 1024;
            stored.push((
                f.name.clone(),
                if external {
                    None
                } else {
                    Some(f.bytes.clone())
                },
                if external {
                    Some(self.write_blob(&f.bytes)?)
                } else {
                    None
                },
            ));
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO items(id,view,touched) VALUES(?1,?2,?3)",
            params![id, serde_json::to_string(&view)?, order],
        )?;
        for (name, inline, blob) in stored {
            tx.execute(
                "INSERT INTO formats(item,name,inline,blob) VALUES(?1,?2,?3,?4)",
                params![id, name, inline, blob],
            )?;
        }
        tx.commit()?;
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
#[cfg(test)]
mod tests {
    use super::*;
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
            .execute("UPDATE items SET view=json_remove(view,'$.preview')", [])
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
