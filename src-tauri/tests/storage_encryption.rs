//! 存储与加密测试：去重置顶、错误密钥、blob 篡改、内容哈希一致性、缩略图访问控制、
//! 主密钥文件语义，以及落盘数据的加密检查（对应 docs 第四节加密方案）。

use clipraft_windows::model::{digest, encode_hdrop, Clip, FormatData};
use clipraft_windows::storage::{crypto, Storage};
use clipraft_windows::{platform, thumbnail};
use image::RgbaImage;
use std::{collections::HashSet, fs, io::Cursor, path::PathBuf};

const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

fn temp_root(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.test-data")
        .join(format!("it-{}-{}", std::process::id(), name));
    if root.exists() {
        fs::remove_dir_all(&root).expect("清理隔离测试目录失败");
    }
    fs::create_dir_all(&root).expect("创建隔离测试目录失败");
    root
}

fn text_clip(text: &str) -> Clip {
    Clip {
        formats: vec![FormatData {
            name: "CF_UNICODETEXT".into(),
            bytes: text
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
        }],
    }
}

fn png_bytes(color: [u8; 4]) -> Vec<u8> {
    let img = RgbaImage::from_pixel(8, 8, image::Rgba(color));
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("测试用 PNG 编码失败");
    out.into_inner()
}

fn image_clip(png: Vec<u8>) -> Clip {
    Clip {
        formats: vec![FormatData {
            name: "PNG".into(),
            bytes: png,
        }],
    }
}

#[test]
fn save_dedupes_and_promotes_existing_record() {
    let root = temp_root("dedup");
    let mut s = Storage::open(&root, &[7u8; 32]).unwrap();
    let a = text_clip("合成内容 A");
    let b = text_clip("合成内容 B");
    let c = text_clip("合成内容 C");

    s.save(&a).unwrap();
    s.save(&b).unwrap();
    s.save(&c).unwrap();
    let ids =
        |s: &Storage| -> Vec<String> { s.list().unwrap().iter().map(|v| v.id.clone()).collect() };
    assert_eq!(
        ids(&s),
        vec![c.hash(), b.hash(), a.hash()],
        "应按最近使用倒序"
    );

    // 再次复制相同内容：不新增条目，只置顶。
    s.save(&a).unwrap();
    assert_eq!(ids(&s), vec![a.hash(), c.hash(), b.hash()]);
    assert_eq!(s.list().unwrap().len(), 3, "相同内容不得产生重复条目");

    let loaded = s.load(&a.hash()).unwrap();
    assert_eq!(loaded.hash(), a.hash(), "load 后内容哈希必须一致");
    assert_eq!(loaded.text().as_deref(), Some("合成内容 A"));

    s.save(&b).unwrap();
    assert_eq!(ids(&s), vec![b.hash(), a.hash(), c.hash()]);
    drop(s);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_content_in_different_format_order_is_one_record() {
    let root = temp_root("hash-order");
    let mut s = Storage::open(&root, &[7u8; 32]).unwrap();
    let one = Clip {
        formats: vec![
            FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: vec![0x41, 0x00, 0x00, 0x00],
            },
            FormatData {
                name: "HTML Format".into(),
                bytes: b"<b>A</b>".to_vec(),
            },
        ],
    };
    let mut reversed = one.clone();
    reversed.formats.reverse();
    s.save(&one).unwrap();
    s.save(&reversed).unwrap();
    assert_eq!(s.list().unwrap().len(), 1);
    assert_eq!(one.hash(), reversed.hash());
    assert_eq!(s.load(&one.hash()).unwrap().hash(), one.hash());
    drop(s);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_key_is_rejected_without_damaging_data() {
    let root = temp_root("wrong-key");
    let clip = text_clip("保留记录");
    {
        let mut s = Storage::open(&root, &[7u8; 32]).unwrap();
        s.save(&clip).unwrap();
    }
    let failure = Storage::open(&root, &[8u8; 32]);
    assert!(failure.is_err(), "错误主密钥必须打开失败，而不是静默建新库");
    if let Err(e) = failure {
        // 只记录错误文本，不记录任何密钥或内容。
        eprintln!("[预期诊断] 错误密钥打开失败：{e}");
    }

    // 失败尝试不得破坏既有数据。
    let s = Storage::open(&root, &[7u8; 32]).unwrap();
    assert_eq!(s.list().unwrap().len(), 1);
    assert_eq!(
        s.load(&clip.hash()).unwrap().text().as_deref(),
        Some("保留记录")
    );
    drop(s);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn database_and_blobs_are_encrypted_at_rest() {
    let root = temp_root("at-rest");
    let png = png_bytes([10, 20, 30, 255]);
    let clip = image_clip(png.clone());
    let thumb_id = {
        let mut s = Storage::open(&root, &[7u8; 32]).unwrap();
        s.save(&clip).unwrap();
        s.list().unwrap().remove(0).thumbnail.expect("应生成缩略图")
    };

    let db = fs::read(root.join("history.db")).unwrap();
    assert!(
        !db.starts_with(b"SQLite format 3\0"),
        "数据库文件头不得是明文 SQLite"
    );

    let blobs = root.join("data/blobs");
    let original_id = digest(&png);
    for (label, id) in [
        ("原图", original_id.as_str()),
        ("缩略图", thumb_id.as_str()),
    ] {
        let bytes = fs::read(blobs.join(id)).expect("大对象应存在");
        assert_eq!(&bytes[..4], b"CM01", "{label}应为加密容器");
        assert!(
            !bytes.windows(PNG_SIG.len()).any(|w| w == PNG_SIG),
            "{label}的加密容器中不得出现明文 PNG 头"
        );
    }
    // 数据目录里只应留下以内容哈希命名的完成对象，不能残留 .tmp。
    for entry in fs::read_dir(&blobs).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name.len(), 64, "对象名应为 64 位哈希：{name}");
        assert!(
            name.bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "对象名应为小写十六进制：{name}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tampered_blob_fails_authentication() {
    let root = temp_root("tamper");
    let png = png_bytes([1, 2, 3, 255]);
    let clip = image_clip(png.clone());
    let mut s = Storage::open(&root, &[7u8; 32]).unwrap();
    s.save(&clip).unwrap();
    let view = s.list().unwrap().remove(0);
    let thumb_id = view.thumbnail.clone().expect("应生成缩略图");
    let blob_dir = root.join("data/blobs");

    // 正确密钥下能解出真正的 PNG。
    assert!(s.read_thumbnail(&thumb_id).unwrap().starts_with(PNG_SIG));

    // 篡改缩略图密文最后一个字节：认证失败，且不 panic。
    let mut thumb_blob = fs::read(blob_dir.join(&thumb_id)).unwrap();
    let last = thumb_blob.len() - 1;
    thumb_blob[last] ^= 0x01;
    fs::write(blob_dir.join(&thumb_id), &thumb_blob).unwrap();
    assert!(s.read_thumbnail(&thumb_id).is_err());

    // 篡改原图 blob 的随机位置：load 必须报错。
    let original_id = digest(&png);
    let mut original_blob = fs::read(blob_dir.join(&original_id)).unwrap();
    original_blob[9] ^= 0xFF;
    fs::write(blob_dir.join(&original_id), &original_blob).unwrap();
    assert!(s.load(&clip.hash()).is_err());

    // 容器被截断：同样必须报错。
    fs::write(blob_dir.join(&original_id), &original_blob[..16]).unwrap();
    assert!(s.load(&clip.hash()).is_err());
    drop(s);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn blob_ciphertext_is_bound_to_its_object_name() {
    let root = temp_root("aad");
    let a = image_clip(png_bytes([0, 0, 255, 255]));
    let b = image_clip(png_bytes([0, 255, 0, 255]));
    let mut s = Storage::open(&root, &[7u8; 32]).unwrap();
    s.save(&a).unwrap();
    s.save(&b).unwrap();

    let blob_dir = root.join("data/blobs");
    let id_a = digest(a.get("PNG").unwrap());
    let id_b = digest(b.get("PNG").unwrap());
    // 把 B 的合法密文搬到 A 的对象名下：AAD 绑定应使解密失败。
    fs::copy(blob_dir.join(&id_b), blob_dir.join(&id_a)).unwrap();
    assert!(
        s.load(&a.hash()).is_err(),
        "对象名作为 AAD，替换成另一对象的密文必须认证失败"
    );
    drop(s);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn inline_and_external_storage_boundary_roundtrips() {
    let root = temp_root("boundary");
    let mut s = Storage::open(&root, &[7u8; 32]).unwrap();
    for size in [64 * 1024usize, 64 * 1024 + 1] {
        let bytes = vec![0x41u8; size];
        let clip = Clip {
            formats: vec![FormatData {
                name: "Rich Text Format".into(),
                bytes: bytes.clone(),
            }],
        };
        s.save(&clip).unwrap();
        assert_eq!(
            s.load(&clip.hash()).unwrap().get("Rich Text Format"),
            Some(bytes.as_slice()),
            "size={size} 的格式数据必须无损往返"
        );
    }
    assert_eq!(s.list().unwrap().len(), 2);
    drop(s);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn thumbnail_protocol_lookup_is_restricted() {
    let root = temp_root("thumb-acl");
    let clip = image_clip(png_bytes([9, 9, 9, 255]));
    let mut s = Storage::open(&root, &[7u8; 32]).unwrap();
    s.save(&clip).unwrap();
    let thumb_id = s.list().unwrap().remove(0).thumbnail.unwrap();

    assert!(s.read_thumbnail(&thumb_id).is_ok());
    // 未登记的对象名。
    assert!(s.read_thumbnail(&"0".repeat(64)).is_err());
    // 非 64 位小写十六进制：路径穿越、绝对路径、空串全部拒绝。
    let bad_cases = vec![
        "../../key.bin".to_string(),
        "..\\..\\key.bin".to_string(),
        "C:/Windows/win.ini".to_string(),
        "/etc/passwd".to_string(),
        String::new(),
        thumb_id.to_uppercase(),
        thumb_id[..63].to_string(),
        format!("{thumb_id}/../../key.bin"),
    ];
    for bad in &bad_cases {
        assert!(s.read_thumbnail(bad).is_err(), "必须拒绝标识：{bad}");
    }
    // 原图内容不通过协议暴露：只有缩略图对象被登记。
    let original_id = digest(clip.get("PNG").unwrap());
    if original_id != thumb_id {
        assert!(
            s.read_thumbnail(&original_id).is_err(),
            "协议不得按原图哈希读取原图"
        );
    }
    drop(s);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_record_metadata_and_placeholder_are_persisted() {
    let root = temp_root("views");
    let mut s = Storage::open(&root, &[7u8; 32]).unwrap();

    let paths = vec![r"C:\测试\a.txt".to_string(), r"C:\测试\b.png".to_string()];
    let files = Clip {
        formats: vec![FormatData {
            name: "CF_HDROP".into(),
            bytes: encode_hdrop(&paths),
        }],
    };
    s.save(&files).unwrap();
    let view = s.list().unwrap().remove(0);
    assert_eq!(view.kind, "files");
    assert_eq!(view.files, paths);
    assert_eq!(view.summary, "a.txt、b.png");
    assert_eq!(view.formats, vec!["CF_HDROP".to_string()]);
    assert!(view.thumbnail.is_none());

    // 损坏的图片记录仍要落库，并带占位信息。
    let broken = Clip {
        formats: vec![FormatData {
            name: "PNG".into(),
            bytes: vec![1, 2, 3, 4],
        }],
    };
    s.save(&broken).unwrap();
    let view = s
        .list()
        .unwrap()
        .into_iter()
        .find(|v| v.id == broken.hash())
        .expect("损坏图片记录也应存在");
    assert_eq!(view.kind, "image");
    assert!(view.thumbnail.is_none());
    let placeholder = view.placeholder.expect("应带占位说明");
    assert!(placeholder.contains("PNG"), "{placeholder}");
    assert_eq!(
        s.load(&broken.hash()).unwrap().get("PNG"),
        Some([1u8, 2, 3, 4].as_slice()),
        "解码失败仍要保留原始数据"
    );
    drop(s);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn blob_crypto_nonce_key_and_aad_discipline() {
    let master = [3u8; 32];
    let key = crypto::derive(&master).unwrap();
    assert_eq!(
        key.as_slice(),
        crypto::derive(&master).unwrap().as_slice(),
        "派生结果应可复现"
    );
    assert_ne!(key.as_slice(), master.as_slice(), "派生密钥不应等于主密钥");
    assert_ne!(
        crypto::derive(&[4u8; 32]).unwrap().as_slice(),
        key.as_slice(),
        "不同主密钥应派生不同密钥"
    );

    // 相同明文、相同对象名重复加密：nonce 必须每次都不同。
    let mut nonces = HashSet::new();
    let mut containers = HashSet::new();
    for _ in 0..64 {
        let c = crypto::encrypt(&key, "object-id", b"same plaintext").unwrap();
        assert_eq!(&c[..4], b"CM01");
        assert!(nonces.insert(c[4..28].to_vec()), "nonce 出现重复");
        assert!(containers.insert(c.clone()), "相同输入产生了相同密文");
        assert_eq!(
            crypto::decrypt(&key, "object-id", &c).unwrap(),
            b"same plaintext"
        );
    }

    let c = crypto::encrypt(&key, "object-id", b"same plaintext").unwrap();
    // AAD 绑定对象名。
    assert!(crypto::decrypt(&key, "other-id", &c).is_err());
    // 换密钥。
    assert!(crypto::decrypt(&crypto::derive(&[4u8; 32]).unwrap(), "object-id", &c).is_err());
    // 截断、错误魔数、空输入。
    assert!(crypto::decrypt(&key, "object-id", &c[..43]).is_err());
    let mut wrong_magic = c.clone();
    wrong_magic[1] = b'X';
    assert!(crypto::decrypt(&key, "object-id", &wrong_magic).is_err());
    assert!(crypto::decrypt(&key, "object-id", b"").is_err());
    // 只改 nonce 不改正文：认证失败。
    let mut bad_nonce = c.clone();
    bad_nonce[6] ^= 0x40;
    assert!(crypto::decrypt(&key, "object-id", &bad_nonce).is_err());

    // 明文不出现在密文中。
    let marker = b"SYNTHETIC-MARKER-VALUE";
    let c = crypto::encrypt(&key, "id", marker).unwrap();
    assert!(!c.windows(marker.len()).any(|w| w == marker));
}

#[test]
fn master_key_file_semantics() {
    let root = temp_root("master-key");
    let first = platform::master_key(&root).expect("首次应生成主密钥");
    let stored = fs::read(root.join("key.bin")).unwrap();
    assert!(root.join("key.bin").exists());
    assert_ne!(stored.len(), 32, "key.bin 不应是裸 32 字节密钥");
    assert!(
        !stored.windows(32).any(|w| w == first.as_slice()),
        "key.bin 中不得出现明文主密钥"
    );
    let second = platform::master_key(&root).expect("应能再次解出同一主密钥");
    assert_eq!(first.as_slice(), second.as_slice());

    // 数据库存在但密钥缺失：拒绝生成替代密钥。
    let missing = temp_root("master-key-missing");
    fs::write(missing.join("history.db"), b"").unwrap();
    assert!(
        platform::master_key(&missing).is_err(),
        "存在数据库却缺少 key.bin 时必须报错"
    );
    assert!(
        !missing.join("key.bin").exists(),
        "不得在密钥缺失时生成新密钥"
    );

    // key.bin 损坏：DPAPI 解不开，同样必须报错。
    let corrupt = temp_root("master-key-corrupt");
    fs::write(corrupt.join("key.bin"), b"not-a-dpapi-blob").unwrap();
    assert!(platform::master_key(&corrupt).is_err());

    for dir in [root, missing, corrupt] {
        if dir.exists() {
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn thumbnail_module_agrees_with_storage_comparison() {
    // 存储层保存的缩略图必须就是 thumbnail::generate 生成的内容。
    let png = png_bytes([128, 0, 128, 255]);
    let clip = image_clip(png);
    let expected = thumbnail::generate(&clip).png.expect("应生成缩略图");
    let root = temp_root("thumb-consistency");
    let mut s = Storage::open(&root, &[7u8; 32]).unwrap();
    s.save(&clip).unwrap();
    let id = s.list().unwrap().remove(0).thumbnail.unwrap();
    assert_eq!(s.read_thumbnail(&id).unwrap(), expected);
    assert_eq!(id, digest(&expected), "对象名应为内容哈希");
    drop(s);
    fs::remove_dir_all(root).unwrap();
}
