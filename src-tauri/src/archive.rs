//! 密码归档的有界流加密层；解密输出只能写入未发布的暂存区。
//! 完整成功（包括结束认证和 EOF）之前，调用方不得使用输出。
use anyhow::{anyhow, bail, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use std::io::{Read, Write};
use zeroize::Zeroizing;
pub mod abandoned;
pub mod bundle;
pub(crate) mod cleanup_review;
pub mod control;
pub mod export_job;
pub mod jobs;
pub mod restore_job;
pub(crate) mod staging;
mod staging_owner;

const MAGIC: &[u8; 8] = b"CRAF0001";
const HEADER_SIZE: usize = 40;
const CHUNK_SIZE: usize = 1024 * 1024;
const TAG_SIZE: usize = 16;
const MAX_PASSWORD_BYTES: usize = 1024;
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024 * 1024;

pub(crate) fn validate_password(password: &[u8]) -> Result<()> {
    if password.is_empty() || password.len() > MAX_PASSWORD_BYTES {
        bail!("密码长度必须为 1 到 1024 字节");
    }
    Ok(())
}

struct Cipher {
    header: [u8; HEADER_SIZE],
    key: Zeroizing<[u8; 32]>,
    sequence: u64,
}
impl Cipher {
    fn new(header: [u8; HEADER_SIZE], password: &[u8]) -> Result<Self> {
        if header[..8] != *MAGIC {
            bail!("不支持的归档版本或密码派生配置");
        }
        validate_password(password)?;
        // 格式版本固定参数，未认证头部不能指定任意内存开销。
        let params =
            Params::new(64 * 1024, 3, 4, Some(32)).map_err(|_| anyhow!("密码派生参数无效"))?;
        let mut key = Zeroizing::new([0; 32]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(password, &header[8..24], key.as_mut())
            .map_err(|_| anyhow!("密码派生失败"))?;
        Ok(Self {
            header,
            key,
            sequence: 0,
        })
    }
    fn nonce(&self) -> [u8; 24] {
        let mut nonce = [0; 24];
        nonce[..16].copy_from_slice(&self.header[24..]);
        nonce[16..].copy_from_slice(&self.sequence.to_le_bytes());
        nonce
    }
    fn aad(&self, size: u32) -> Vec<u8> {
        let mut aad = self.header.to_vec();
        aad.extend_from_slice(&self.sequence.to_le_bytes());
        aad.extend_from_slice(&size.to_le_bytes());
        aad
    }
    fn advance(&mut self) -> Result<()> {
        self.sequence = self.sequence.checked_add(1).context("归档块序号溢出")?;
        Ok(())
    }
    fn write_block(&mut self, output: &mut impl Write, plain: &[u8]) -> Result<()> {
        let size = plain.len() as u32;
        let encrypted = XChaCha20Poly1305::new((&*self.key).into())
            .encrypt(
                XNonce::from_slice(&self.nonce()),
                Payload {
                    msg: plain,
                    aad: &self.aad(size),
                },
            )
            .map_err(|_| anyhow!("归档块加密失败"))?;
        output.write_all(&size.to_le_bytes())?;
        output.write_all(&encrypted)?;
        self.advance()
    }
    fn read_block(&mut self, input: &mut impl Read) -> Result<Zeroizing<Vec<u8>>> {
        let mut length = [0; 4];
        input.read_exact(&mut length).context("归档被截断")?;
        let size = u32::from_le_bytes(length);
        if size as usize > CHUNK_SIZE {
            bail!("归档块超过大小限制");
        }
        let mut encrypted = vec![0; size as usize + TAG_SIZE];
        input.read_exact(&mut encrypted).context("归档被截断")?;
        let plain = XChaCha20Poly1305::new((&*self.key).into())
            .decrypt(
                XNonce::from_slice(&self.nonce()),
                Payload {
                    msg: &encrypted,
                    aad: &self.aad(size),
                },
            )
            .map_err(|_| anyhow!("密码错误或归档损坏"))?;
        self.advance()?;
        Ok(Zeroizing::new(plain))
    }
}

/// 对未压缩的归档内容进行分块认证；不读取目录或写入用户配置。
pub fn encrypt(input: &mut impl Read, output: &mut impl Write, password: &[u8]) -> Result<()> {
    let mut header = [0; HEADER_SIZE];
    header[..8].copy_from_slice(MAGIC);
    getrandom::fill(&mut header[8..]).map_err(|_| anyhow!("系统随机数不可用"))?;
    let mut cipher = Cipher::new(header, password)?;
    output.write_all(&header)?;
    let mut buffer = Zeroizing::new(vec![0; CHUNK_SIZE]);
    let mut total = 0u64;
    loop {
        let count = read_chunk(input, &mut buffer)?;
        total = checked_total(total, count)?;
        cipher.write_block(output, &buffer[..count])?;
        if count == 0 {
            return Ok(());
        }
    }
}

/// 必须写入独立暂存区；返回错误时调用方须丢弃所有已输出内容。
pub fn decrypt(input: &mut impl Read, output: &mut impl Write, password: &[u8]) -> Result<()> {
    let mut header = [0; HEADER_SIZE];
    input.read_exact(&mut header).context("归档头部不完整")?;
    let mut cipher = Cipher::new(header, password)?;
    let mut total = 0u64;
    loop {
        let plain = cipher.read_block(input)?;
        if plain.is_empty() {
            return require_eof(input);
        }
        total = checked_total(total, plain.len())?;
        output.write_all(&plain)?;
    }
}
fn checked_total(total: u64, count: usize) -> Result<u64> {
    let total = total.checked_add(count as u64).context("归档长度溢出")?;
    if total > MAX_ARCHIVE_BYTES {
        bail!("归档超过 1 TiB 上限");
    }
    Ok(total)
}
fn read_chunk(input: &mut impl Read, buffer: &mut [u8]) -> Result<usize> {
    let mut count = 0;
    while count < buffer.len() {
        match input.read(&mut buffer[count..]) {
            Ok(0) => break,
            Ok(size) => count += size,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(count)
}
fn require_eof(input: &mut impl Read) -> Result<()> {
    if read_chunk(input, &mut [0])? != 0 {
        bail!("归档结束后存在多余内容");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
