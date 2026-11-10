use anyhow::{anyhow, bail, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

/// CM01 标记（4）+ XChaCha20 nonce（24）+ Poly1305 tag（16）。
pub const BLOB_OVERHEAD: u64 = 44;

pub fn random_key() -> Result<Zeroizing<[u8; 32]>> {
    let mut key = Zeroizing::new([0; 32]);
    getrandom::fill(key.as_mut()).map_err(|_| anyhow!("系统随机数不可用"))?;
    Ok(key)
}
pub fn derive(master: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(b"ClipboardM0/v1"), master)
        .expand(b"encrypted-blob", key.as_mut())
        .expect("固定派生长度有效");
    key
}
/// CM02 基线压缩；PNG/JPEG 已编码图片直接沿用 CM01。
/// 只有至少节省 5% 才采用 CM02，整数比较避免浮点误差和乘法溢出。
pub fn compress_payload(plain: &[u8]) -> Result<Option<Vec<u8>>> {
    if plain.len() > MAX_PLAIN_BYTES {
        bail!("对象明文超过 128 MiB 上限");
    }
    if crate::thumbnail::preview_mime(plain).is_some() || plain.is_empty() {
        return Ok(None);
    }
    let compressed = zstd::bulk::compress(plain, 3)?;
    Ok(compression_worthwhile(plain.len(), compressed.len()).then_some(compressed))
}

/// CM02 多出的 8 字节长度字段也计入成本，避免小对象压缩后反而变大。
fn compression_worthwhile(plain_len: usize, compressed_len: usize) -> bool {
    (compressed_len as u128 + 8) * 100 < plain_len as u128 * 95
}

/// 与剪贴板采集共享上限；CM01 读取保持旧格式兼容。
pub const MAX_PLAIN_BYTES: usize = 128 * 1024 * 1024;

/// 保留 CM01 编码入口，供旧库兼容及已压缩的 PNG/JPEG 对象使用。
pub fn encrypt(key: &[u8; 32], id: &str, plain: &[u8]) -> Result<Vec<u8>> {
    seal(key, b"CM01", &[], id.as_bytes(), plain)
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum CompressedPayload {
    Raw,
    Zstd(Vec<u8>),
    Predicted(Vec<u8>),
}

/// 纯函数：保留 CM02/CM01 基线；CM03 只有更小且至少节省 5% 才替换。
pub(super) fn select_payload(plain: &[u8]) -> Result<CompressedPayload> {
    let baseline = compress_payload(plain)?;
    if crate::thumbnail::preview_mime(plain).is_none() && plain.len() < MAX_PLAIN_BYTES {
        if let Some(predicted) = super::predict::encode(plain) {
            let compressed = zstd::bulk::compress(&predicted, 3)?;
            if compression_worthwhile(plain.len(), compressed.len())
                && baseline
                    .as_ref()
                    .is_none_or(|old| compressed.len() < old.len())
            {
                return Ok(CompressedPayload::Predicted(compressed));
            }
        }
    }
    Ok(match baseline {
        Some(bytes) => CompressedPayload::Zstd(bytes),
        None => CompressedPayload::Raw,
    })
}

pub fn encrypt_blob(key: &[u8; 32], id: &str, plain: &[u8]) -> Result<Vec<u8>> {
    let (magic, compressed) = match select_payload(plain)? {
        CompressedPayload::Raw => return encrypt(key, id, plain),
        CompressedPayload::Zstd(bytes) => (b"CM02", bytes),
        CompressedPayload::Predicted(bytes) => (b"CM03", bytes),
    };
    let length = (plain.len() as u64).to_le_bytes();
    seal(
        key,
        magic,
        &length,
        &compressed_aad(magic, id, &length),
        &compressed,
    )
}
fn compressed_aad(magic: &[u8; 4], id: &str, length: &[u8]) -> Vec<u8> {
    let mut aad = magic.to_vec();
    aad.push(b':');
    aad.extend_from_slice(id.as_bytes());
    aad.extend_from_slice(length);
    aad
}
#[cfg(test)]
fn cm02_aad(id: &str, length: &[u8]) -> Vec<u8> {
    compressed_aad(b"CM02", id, length)
}
fn seal(
    key: &[u8; 32],
    magic: &[u8; 4],
    header: &[u8],
    aad: &[u8],
    plain: &[u8],
) -> Result<Vec<u8>> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| anyhow!("系统随机数不可用"))?;
    let body = XChaCha20Poly1305::new(key.into())
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad })
        .map_err(|_| anyhow!("对象加密失败"))?;
    let mut out = Vec::with_capacity(4 + header.len() + nonce.len() + body.len());
    out.extend_from_slice(magic);
    out.extend_from_slice(header);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&body);
    Ok(out)
}
pub fn decrypt(key: &[u8; 32], id: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    decrypt_with_limit(key, id, bytes, MAX_PLAIN_BYTES)
}
fn decrypt_with_limit(key: &[u8; 32], id: &str, bytes: &[u8], limit: usize) -> Result<Vec<u8>> {
    if bytes.len() < BLOB_OVERHEAD as usize {
        bail!("对象格式无效");
    }
    let (offset, length, aad) = match &bytes[..4] {
        b"CM01" => (4, None, id.as_bytes().to_vec()),
        b"CM02" | b"CM03" if bytes.len() >= BLOB_OVERHEAD as usize + 8 => {
            let length = u64::from_le_bytes(bytes[4..12].try_into().unwrap());
            if length > limit as u64 {
                bail!("对象解压长度超过上限");
            }
            let magic: &[u8; 4] = bytes[..4].try_into().unwrap();
            if magic == b"CM03" && length >= limit.min(MAX_PLAIN_BYTES) as u64 {
                bail!("对象预测解压长度超过上限");
            }
            (
                12,
                Some(length as usize),
                compressed_aad(magic, id, &bytes[4..12]),
            )
        }
        _ => bail!("对象格式无效"),
    };
    // 必须认证通过才把载荷交给解压器；CM02/CM03 的版本和长度都绑定 AAD。
    let body = XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&bytes[offset..offset + 24]),
            Payload {
                msg: &bytes[offset + 24..],
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("对象认证失败"))?;
    match length {
        None => Ok(body),
        Some(length) => {
            // bulk 解码器的容量就是硬输出上限，不使用无界 decode_all。
            let predicted = &bytes[..4] == b"CM03";
            let decoded_len = length
                .checked_add(usize::from(predicted))
                .filter(|&n| n <= limit.min(MAX_PLAIN_BYTES))
                .ok_or_else(|| anyhow!("对象解压长度超过上限"))?;
            let plain = zstd::bulk::decompress(&body, decoded_len)
                .map_err(|_| anyhow!("对象解压失败或长度超过上限"))?;
            if plain.len() != decoded_len {
                bail!("对象解压长度不匹配");
            }
            if predicted {
                super::predict::decode(plain, length)
            } else {
                Ok(plain)
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_tamper() {
        let key = derive(&[7; 32]);
        let mut c = encrypt(&key, "id", b"fixture").unwrap();
        assert_eq!(decrypt(&key, "id", &c).unwrap(), b"fixture");
        assert!(decrypt(&key, "other", &c).is_err());
        assert!(decrypt(&[9; 32], "id", &c).is_err());
        c[30] ^= 1;
        assert!(decrypt(&key, "id", &c).is_err());
        assert!(decrypt(&key, "id", b"").is_err());
    }
    #[test]
    fn nonce_is_random() {
        assert_ne!(
            encrypt(&[1; 32], "id", b"a").unwrap(),
            encrypt(&[1; 32], "id", b"a").unwrap()
        );
    }
    #[test]
    fn cm02_roundtrip_and_identity() {
        let plain = vec![42; 64 * 1024];
        let id = crate::model::digest(&plain);
        let bytes = encrypt_blob(&[1; 32], &id, &plain).unwrap();
        assert_eq!(&bytes[..4], b"CM02");
        assert_eq!(
            u64::from_le_bytes(bytes[4..12].try_into().unwrap()),
            plain.len() as u64
        );
        assert_eq!(decrypt(&[1; 32], &id, &bytes).unwrap(), plain);
        assert!(decrypt(&[2; 32], &id, &bytes).is_err());
        assert!(decrypt(&[1; 32], "other", &bytes).is_err());
    }
    #[test]
    fn cm02_authenticates_body_and_nonce() {
        let bytes = encrypt_blob(&[1; 32], "id", &[7; 4096]).unwrap();
        for index in [12, 36, bytes.len() - 1] {
            let mut changed = bytes.clone();
            changed[index] ^= 1;
            assert_eq!(
                decrypt(&[1; 32], "id", &changed).unwrap_err().to_string(),
                "对象认证失败"
            );
        }
    }
    #[test]
    fn cm02_version_is_bound_to_authentication() {
        let mut bytes = encrypt_blob(&[1; 32], "id", &[7; 4096]).unwrap();
        bytes[..4].copy_from_slice(b"CM01");
        assert_eq!(
            decrypt(&[1; 32], "id", &bytes).unwrap_err().to_string(),
            "对象认证失败"
        );
        // 去掉长度字段后，nonce/body 位置即使与 CM01 对齐也必须认证失败。
        bytes.drain(4..12);
        assert_eq!(
            decrypt(&[1; 32], "id", &bytes).unwrap_err().to_string(),
            "对象认证失败"
        );
    }
    #[test]
    fn cm02_authenticates_plain_length() {
        let mut bytes = encrypt_blob(&[1; 32], "id", &[7; 4096]).unwrap();
        bytes[4..12].copy_from_slice(&4095u64.to_le_bytes());
        assert_eq!(
            decrypt(&[1; 32], "id", &bytes).unwrap_err().to_string(),
            "对象认证失败"
        );
    }
    #[test]
    fn cm02_output_limit_inclusive_and_excess_returns_error() {
        // 同一生产入口缩小预算，覆盖输出上限而无需分配 128 MiB。
        let plain = vec![7; 4096];
        let bytes = encrypt_blob(&[1; 32], "id", &plain).unwrap();
        assert_eq!(
            decrypt_with_limit(&[1; 32], "id", &bytes, 4096).unwrap(),
            plain
        );
        assert!(decrypt_with_limit(&[1; 32], "id", &bytes, 4095)
            .unwrap_err()
            .to_string()
            .contains("超过上限"));
    }
    #[test]
    fn short_or_unknown_objects_return_error_without_panic() {
        let bytes = encrypt_blob(&[1; 32], "id", &[7; 4096]).unwrap();
        for len in 0..bytes.len() {
            assert!(decrypt(&[1; 32], "id", &bytes[..len]).is_err());
        }
        let mut unknown = bytes;
        unknown[..4].copy_from_slice(b"CM99");
        assert!(decrypt(&[1; 32], "id", &unknown).is_err());
    }
    #[test]
    fn png_and_small_objects_stay_cm01() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend([0; 4096]);
        for plain in [png, b"short".to_vec(), Vec::new()] {
            assert!(compress_payload(&plain).unwrap().is_none());
            let bytes = encrypt_blob(&[1; 32], "id", &plain).unwrap();
            assert_eq!(&bytes[..4], b"CM01");
            assert_eq!(decrypt(&[1; 32], "id", &bytes).unwrap(), plain);
        }
    }
    #[test]
    fn incompressible_bytes_fall_back_to_cm01() {
        let mut state = 0x12345678u32;
        let plain: Vec<u8> = (0..65536)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        assert!(compress_payload(&plain).unwrap().is_none());
        let bytes = encrypt_blob(&[1; 32], "id", &plain).unwrap();
        assert_eq!(&bytes[..4], b"CM01");
        assert_eq!(decrypt(&[1; 32], "id", &bytes).unwrap(), plain);
    }
    #[test]
    fn compression_threshold_accounts_for_length_header() {
        assert!(compression_worthwhile(1000, 941));
        assert!(!compression_worthwhile(1000, 942));
        assert!(!compression_worthwhile(19, 17));
        assert!(!compression_worthwhile(0, 0));
        assert!(!compression_worthwhile(usize::MAX, usize::MAX));
    }
    #[test]
    fn compression_decision_is_deterministic_and_saves_five_percent() {
        for plain in [
            vec![65; 65536],
            b"<html><p>synthetic</p></html>".repeat(3000),
            b"{\\rtf1 synthetic}".repeat(3000),
        ] {
            let compressed = compress_payload(&plain).unwrap().unwrap();
            assert_eq!(compress_payload(&plain).unwrap().unwrap(), compressed);
            assert!(compressed.len() * 100 < plain.len() * 95);
            assert_eq!(
                zstd::bulk::decompress(&compressed, plain.len()).unwrap(),
                plain
            );
        }
    }
    #[test]
    fn cm02_declared_length_must_match_decoded_length() {
        let compressed = zstd::bulk::compress(&[7; 32], 3).unwrap();
        // 小型固定样本覆盖解码器容量不足和实际长度不足两个错误分支。
        for length in [16u64, 64] {
            let header = length.to_le_bytes();
            let bytes = seal(
                &[1; 32],
                b"CM02",
                &header,
                &cm02_aad("id", &header),
                &compressed,
            )
            .unwrap();
            assert!(decrypt(&[1; 32], "id", &bytes).is_err());
        }
    }
}

#[cfg(test)]
mod cm03_tests {
    use super::*;
    use crate::storage::{predict, prediction_tests::sample};
    fn encoded(data: &[u8], length: usize) -> Vec<u8> {
        let header = (length as u64).to_le_bytes();
        seal(
            &[1; 32],
            b"CM03",
            &header,
            &compressed_aad(b"CM03", "id", &header),
            &zstd::bulk::compress(data, 3).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn jpeg_signature_skips_compression() {
        let plain = [b"\xff\xd8\xff".as_slice(), &[0; 8192]].concat();
        assert!(compress_payload(&plain).unwrap().is_none());
        assert_eq!(select_payload(&plain).unwrap(), CompressedPayload::Raw);
        let encrypted = encrypt_blob(&[1; 32], "id", &plain).unwrap();
        assert_eq!(&encrypted[..4], b"CM01");
        assert_eq!(decrypt(&[1; 32], "id", &encrypted).unwrap(), plain);
    }
    #[test]
    fn cm03_authenticates_format_length_identity_nonce_and_body() {
        let plain = sample("photo", 96, 64);
        let bytes = encrypt_blob(&[1; 32], "id", &plain).unwrap();
        assert_eq!(&bytes[..4], b"CM03");
        assert_eq!(decrypt(&[1; 32], "id", &bytes).unwrap(), plain);
        assert!(decrypt(&[2; 32], "id", &bytes).is_err());
        assert!(decrypt(&[1; 32], "other", &bytes).is_err());
        for index in [4, 12, 36, bytes.len() - 1] {
            let mut changed = bytes.clone();
            changed[index] ^= 1;
            assert!(decrypt(&[1; 32], "id", &changed).is_err());
        }
        for magic in [b"CM01", b"CM02"] {
            let mut changed = bytes.clone();
            changed[..4].copy_from_slice(magic);
            assert!(decrypt(&[1; 32], "id", &changed).is_err());
            if magic == b"CM01" {
                changed.drain(4..12);
                assert!(decrypt(&[1; 32], "id", &changed).is_err());
            }
        }
        for end in [0, 3, 4, 11, 12, 35, 36, 51, bytes.len() - 1] {
            assert!(decrypt(&[1; 32], "id", &bytes[..end]).is_err());
        }
    }
    #[test]
    fn cm03_rejects_authenticated_invalid_filter_header_and_truncation() {
        let plain = sample("photo", 16, 8);
        let data = predict::encode(&plain).unwrap();
        let mut filter = data.clone();
        *filter.last_mut().unwrap() = 255;
        let mut header = data.clone();
        header[..4].copy_from_slice(&39u32.to_le_bytes());
        for invalid in [filter, header, data[..data.len() - 1].to_vec()] {
            assert!(decrypt(&[1; 32], "id", &encoded(&invalid, plain.len())).is_err());
        }
    }
    #[test]
    fn cm03_bounded_decode_checks_exact_length_and_expansion_budget() {
        // 用 KiB 级样本验证同一容量边界，不生成大体积载荷。
        let plain = sample("solid", 16, 8);
        let data = predict::encode(&plain).unwrap();
        let blob = encoded(&data, plain.len());
        assert_eq!(
            decrypt_with_limit(&[1; 32], "id", &blob, data.len()).unwrap(),
            plain
        );
        assert!(decrypt_with_limit(&[1; 32], "id", &blob, plain.len()).is_err());
        for length in [
            32,
            plain.len() - 1,
            plain.len() + 1,
            MAX_PLAIN_BYTES,
            MAX_PLAIN_BYTES + 1,
        ] {
            assert!(decrypt(&[1; 32], "id", &encoded(&data, length)).is_err());
        }
    }
    #[test]
    fn explicit_legacy_cm02_dib_still_reads() {
        let plain = sample("photo", 32, 32);
        let length = (plain.len() as u64).to_le_bytes();
        let bytes = seal(
            &[1; 32],
            b"CM02",
            &length,
            &cm02_aad("id", &length),
            &compress_payload(&plain).unwrap().unwrap(),
        )
        .unwrap();
        assert_eq!(decrypt(&[1; 32], "id", &bytes).unwrap(), plain);
    }
}
