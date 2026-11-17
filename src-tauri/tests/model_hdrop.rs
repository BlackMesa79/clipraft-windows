//! DROPFILES（CF_HDROP）编解码测试。
//!
//! 文件记录必须能作为真实文件重新粘贴（docs 第一节根因 2、第二节 P0 差异点），
//! 因此结构必须符合 DROPFILES 契约：`pFiles` 偏移、`fWide=1`、路径双零结尾。

use clipraft_windows::model::{decode_hdrop, digest, encode_hdrop, is_hash, Clip, FormatData};

const HEADER: usize = 20;

fn utf16_units(s: &str) -> usize {
    s.encode_utf16().count()
}

fn le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn push_text(clip_formats: &mut Vec<FormatData>, text: &str) {
    clip_formats.push(FormatData {
        name: "CF_UNICODETEXT".into(),
        bytes: text
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect(),
    });
}

#[test]
fn header_fields_match_dropfiles_contract() {
    let paths = vec![r"C:\a.txt".to_string()];
    let blob = encode_hdrop(&paths);

    assert_eq!(
        le(&blob, 0) as usize,
        HEADER,
        "pFiles 必须指向 20 字节头之后"
    );
    assert_eq!(
        le(&blob, 16),
        1,
        "fWide 必须为 1，否则消费方按 ANSI 解析导致中文路径乱码"
    );
    assert!(blob[4..16].iter().all(|b| *b == 0), "pt 与 fNC 应清零");
    // 20 字节头 + 路径 UTF-16 + 结尾 0 + 双零终止的额外 0。
    assert_eq!(blob.len(), HEADER + (utf16_units(&paths[0]) + 1) * 2 + 2);
    assert_eq!(&blob[blob.len() - 2..], &[0, 0], "列表必须以双零结束");
    assert_eq!(decode_hdrop(&blob), Some(paths));
}

#[test]
fn chinese_path_roundtrip() {
    let paths = vec![
        r"C:\测试数据\示例文件.txt".to_string(),
        r"D:\用户目录\中文 空格\图片.png".to_string(),
    ];
    let blob = encode_hdrop(&paths);
    assert_eq!(decode_hdrop(&blob), Some(paths));
}

#[test]
fn long_path_roundtrip() {
    // 超过 MAX_PATH 的长路径（\\?\ 前缀）与接近采集上限（32768 字符）的路径。
    let long_dir = "长".repeat(300);
    let paths = vec![
        format!(r"\\?\C:\{long_dir}\file.txt"),
        format!(r"C:\{}", "a".repeat(32000)),
    ];
    let blob = encode_hdrop(&paths);
    assert_eq!(decode_hdrop(&blob), Some(paths));
}

#[test]
fn many_files_roundtrip() {
    let paths: Vec<String> = (0..64).map(|i| format!(r"C:\批量\{i:03}.bin")).collect();
    let blob = encode_hdrop(&paths);
    assert_eq!(decode_hdrop(&blob), Some(paths));
}

#[test]
fn empty_list_roundtrip() {
    let blob = encode_hdrop(&[]);
    assert_eq!(blob.len(), HEADER + 2);
    assert_eq!(decode_hdrop(&blob), Some(Vec::new()));
}

#[test]
fn non_bmp_characters_survive() {
    let paths = vec![r"C:\表情😀\文件🚀.txt".to_string()];
    assert_eq!(decode_hdrop(&encode_hdrop(&paths)), Some(paths));
}

#[test]
fn trailing_garbage_after_terminator_is_ignored() {
    let paths = vec![r"C:\a.txt".to_string()];
    let mut blob = encode_hdrop(&paths);
    blob.extend_from_slice(&[0xAA; 32]);
    assert_eq!(decode_hdrop(&blob), Some(paths));
}

#[test]
fn corrupt_and_truncated_inputs_are_rejected_without_panic() {
    let valid = encode_hdrop(&[r"C:\测试\a.txt".to_string(), r"C:\b.txt".to_string()]);

    let mut wrong_wide = valid.clone();
    wrong_wide[16..20].copy_from_slice(&0u32.to_le_bytes());

    let mut zero_offset = valid.clone();
    zero_offset[0..4].copy_from_slice(&0u32.to_le_bytes());

    let mut tiny_offset = valid.clone();
    tiny_offset[0..4].copy_from_slice(&19u32.to_le_bytes());

    let mut offset_past_end = valid.clone();
    offset_past_end[0..4].copy_from_slice(&((valid.len() + 100) as u32).to_le_bytes());

    let mut huge_offset = valid.clone();
    huge_offset[0..4].copy_from_slice(&u32::MAX.to_le_bytes());

    // 只有一个结束 0（缺少双零终止）。
    let missing_terminator = valid[..valid.len() - 2].to_vec();

    // 头部齐全但正文全为 0xFF，永远找不到终止符。
    let mut junk = valid[..HEADER].to_vec();
    junk.extend_from_slice(&[0xFF; 16]);

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("空输入", Vec::new()),
        ("仅 1 字节", vec![0]),
        ("全零", vec![0; 20]),
        ("21 字节全零", vec![0; 21]),
        ("fWide=0", wrong_wide),
        ("pFiles=0", zero_offset),
        ("pFiles=19", tiny_offset),
        ("pFiles 越界", offset_past_end),
        ("pFiles=u32::MAX", huge_offset),
        ("缺少双零终止", missing_terminator),
        ("正文无终止符", junk),
        ("奇数长度尾部", {
            // 单个 0 结尾 + 半截 UTF-16 单元（总长为奇数），没有任何双零终止。
            let mut b = valid[..HEADER].to_vec();
            for w in r"C:\x.txt".encode_utf16().chain([0]) {
                b.extend_from_slice(&w.to_le_bytes());
            }
            b.push(0x41);
            b
        }),
    ];
    for (name, blob) in cases {
        assert_eq!(decode_hdrop(&blob), None, "用例「{name}」应被拒绝");
    }
}

#[test]
fn every_truncation_of_a_valid_blob_is_safe() {
    // 逐字节截断：要么完整解出，要么被判为无效，绝不 panic、绝不给出空字符串路径。
    let paths = vec![r"C:\测试\a.txt".to_string(), r"C:\b.txt".to_string()];
    let valid = encode_hdrop(&paths);
    for cut in 0..valid.len() {
        match decode_hdrop(&valid[..cut]) {
            None => {}
            Some(decoded) => {
                assert!(
                    decoded.len() <= paths.len(),
                    "截断到 {cut} 字节时解出了多余路径：{decoded:?}"
                );
                assert!(
                    decoded.iter().all(|p| !p.is_empty()),
                    "截断到 {cut} 字节时解出了空路径"
                );
            }
        }
    }
    assert_eq!(decode_hdrop(&valid), Some(paths));
}

#[test]
fn decode_accepts_alternate_p_files_offset() {
    // 某些程序把路径数据放在自定义偏移（保留额外头信息），解码应遵守 pFiles。
    let extra = 8usize;
    let mut blob = vec![0u8; HEADER + extra];
    blob[0..4].copy_from_slice(&((HEADER + extra) as u32).to_le_bytes());
    blob[16..20].copy_from_slice(&1u32.to_le_bytes());
    for w in r"C:\x.txt".encode_utf16().chain([0, 0]) {
        blob.extend_from_slice(&w.to_le_bytes());
    }
    assert_eq!(decode_hdrop(&blob), Some(vec![r"C:\x.txt".to_string()]));
}

#[test]
fn hash_identity_is_order_independent_but_content_sensitive() {
    let mut a = Vec::new();
    push_text(&mut a, "合成测试文本");
    a.push(FormatData {
        name: "HTML Format".into(),
        bytes: b"<b>x</b>".to_vec(),
    });
    let clip_a = Clip { formats: a.clone() };
    let mut reversed = a;
    reversed.reverse();
    let clip_b = Clip { formats: reversed };

    assert_eq!(clip_a.hash(), clip_b.hash(), "格式顺序不应影响内容身份");
    assert!(is_hash(&clip_a.hash()));
    assert_eq!(clip_a.hash().len(), 64);

    // 空字节格式与缺失格式必须区分开。
    let with_empty = Clip {
        formats: vec![FormatData {
            name: "CF_UNICODETEXT".into(),
            bytes: Vec::new(),
        }],
    };
    let without = Clip {
        formats: Vec::new(),
    };
    assert_ne!(with_empty.hash(), without.hash());
    assert_ne!(with_empty.hash(), clip_a.hash());
    // 哈希稳定可复现。
    assert_eq!(clip_a.hash(), clip_a.hash());
    assert_eq!(digest(b"abc"), digest(b"abc"));
    assert_ne!(digest(b"abc"), digest(b"abd"));
}

#[test]
fn is_hash_rejects_non_object_names() {
    assert!(is_hash(&"0".repeat(64)));
    assert!(is_hash(&"abcdef0123456789".repeat(4)));
    assert!(!is_hash(&"0".repeat(63)));
    assert!(!is_hash(&"0".repeat(65)));
    assert!(!is_hash(&"A".repeat(64)), "大写十六进制应被拒绝");
    assert!(!is_hash("../../key.bin"));
    assert!(!is_hash("..\\..\\key.bin"));
    assert!(!is_hash("C:/Windows/win.ini"));
    assert!(!is_hash(""));
}

#[test]
fn text_decoding_is_lossy_and_bounded() {
    let mut clip = Clip::default();
    clip.formats.push(FormatData {
        name: "CF_UNICODETEXT".into(),
        bytes: "合成"
            .encode_utf16()
            .chain([0, 0])
            .flat_map(u16::to_le_bytes)
            .collect(),
    });
    assert_eq!(clip.text().as_deref(), Some("合成"));

    // 孤立代理项不应 panic，按替换字符处理。
    let mut broken = Clip::default();
    broken.formats.push(FormatData {
        name: "CF_UNICODETEXT".into(),
        bytes: vec![0x00, 0xD8, 0x41, 0x00, 0x00, 0x00],
    });
    assert_eq!(broken.text().as_deref(), Some("\u{FFFD}A"));

    // 奇数长度（截断的 UTF-16）也不 panic，尾部单字节被丢弃。
    let mut odd = Clip::default();
    odd.formats.push(FormatData {
        name: "CF_UNICODETEXT".into(),
        bytes: vec![0x41, 0x00, 0x42],
    });
    assert_eq!(odd.text().as_deref(), Some("A"));

    // 其它格式缺少终止符时按全部内容解码。
    assert_eq!(
        Clip {
            formats: vec![FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: vec![0x41, 0x00],
            }]
        }
        .text()
        .as_deref(),
        Some("A")
    );

    // 未包含 CF_UNICODETEXT 时返回 None（纯图片/文件记录用于错误提示）。
    assert_eq!(
        Clip {
            formats: vec![FormatData {
                name: "PNG".into(),
                bytes: vec![1, 2, 3]
            }]
        }
        .text(),
        None
    );
}

#[test]
fn kind_prefers_files_then_images_then_text() {
    let files = Clip {
        formats: vec![
            FormatData {
                name: "CF_HDROP".into(),
                bytes: encode_hdrop(&[r"C:\a.txt".to_string()]),
            },
            FormatData {
                name: "PNG".into(),
                bytes: vec![1],
            },
        ],
    };
    assert_eq!(files.kind(), "files");
    let image = Clip {
        formats: vec![FormatData {
            name: "CF_DIB".into(),
            bytes: vec![1],
        }],
    };
    assert_eq!(image.kind(), "image");
    let text = Clip {
        formats: vec![FormatData {
            name: "CF_UNICODETEXT".into(),
            bytes: vec![0x41, 0, 0, 0],
        }],
    };
    assert_eq!(text.kind(), "text");
}
