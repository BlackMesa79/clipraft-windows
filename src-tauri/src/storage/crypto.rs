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
pub fn encrypt(key: &[u8; 32], id: &str, plain: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| anyhow!("系统随机数不可用"))?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let body = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("对象加密失败"))?;
    let mut out = b"CM01".to_vec();
    out.extend(nonce);
    out.extend(body);
    Ok(out)
}
pub fn decrypt(key: &[u8; 32], id: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() < BLOB_OVERHEAD as usize || &bytes[..4] != b"CM01" {
        bail!("对象格式无效");
    }
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&bytes[4..28]),
            Payload {
                msg: &bytes[28..],
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("对象认证失败"))
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
}
