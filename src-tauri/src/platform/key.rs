//! 主密钥持久化与 DPAPI 边界。
use anyhow::{bail, Result};
use std::{fs, io::Write, path::Path};
use windows::{
    core::w,
    Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::Cryptography::*,
    },
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, PartialEq, Eq)]
pub enum DataError {
    DpapiDecrypt,
    KeyLength,
    DatabaseKey,
}
impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DpapiDecrypt => "DPAPI 解密 key.bin 失败，可能是更换了 Windows 账户或电脑，也可能是密钥文件损坏。请勿删除数据，请保留完整数据目录并尝试在原账户、原电脑上恢复。",
            Self::KeyLength => "key.bin 长度异常（文件为空或解密后的主密钥不是 32 字节）。请勿删除数据或重新生成密钥，请保留完整数据目录并检查备份。",
            Self::DatabaseKey => "数据库密钥不匹配，或数据库文件已损坏，无法解密历史记录。请勿删除数据，请保留 key.bin、history.db 及其配套文件，并检查是否来自同一份备份。",
        })
    }
}
impl std::error::Error for DataError {}

fn input_blob(input: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    }
}

fn protect(input: &[u8]) -> Result<Vec<u8>> {
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input_blob(input),
            w!("ClipboardM0"),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )?;
    }
    Ok(unsafe { take_output(output) })
}

fn unprotect(input: &[u8]) -> Result<Vec<u8>> {
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input_blob(input),
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )?;
    }
    Ok(unsafe { take_output(output) })
}

// 只接收 DPAPI 成功后分配的缓冲区，复制后先清零再释放。
unsafe fn take_output(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let slice = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
    let bytes = slice.to_vec();
    slice.zeroize();
    if !LocalFree(Some(HLOCAL(output.pbData.cast()))).is_invalid() {
        crate::diag::write("dpapi output release failed".into());
    }
    bytes
}

pub fn master_key(root: &Path) -> Result<Zeroizing<[u8; 32]>> {
    fs::create_dir_all(root)?;
    let path = root.join("key.bin");
    if path.exists() {
        let encrypted = fs::read(path)?;
        if encrypted.is_empty() {
            return Err(DataError::KeyLength.into());
        }
        let decrypted = Zeroizing::new(unprotect(&encrypted).map_err(|_| DataError::DpapiDecrypt)?);
        if decrypted.len() != 32 {
            return Err(DataError::KeyLength.into());
        }
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&decrypted);
        return Ok(key);
    }
    if root.join("history.db").exists() {
        bail!("数据库已存在但密钥缺失，拒绝生成新密钥");
    }
    let key = crate::storage::crypto::random_key()?;
    let protected = protect(key.as_ref())?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&protected)?;
    file.sync_all()?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn dpapi_roundtrip() {
        let c = protect(&[53; 32]).unwrap();
        assert_ne!(c, vec![53; 32]);
        assert_eq!(unprotect(&c).unwrap(), vec![53; 32]);
    }

    fn isolated_key_root(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "key-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        assert!(!root.exists());
        fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn dpapi_failure_has_actionable_message_and_preserves_file() {
        let root = isolated_key_root("dpapi");
        let bytes = b"synthetic-invalid-dpapi";
        fs::write(root.join("key.bin"), bytes).unwrap();
        let error = master_key(&root).unwrap_err();
        assert_eq!(
            error.downcast_ref::<DataError>(),
            Some(&DataError::DpapiDecrypt)
        );
        assert!(error.to_string().contains("Windows 账户或电脑"));
        assert!(error.to_string().contains("请勿删除数据"));
        assert_eq!(fs::read(root.join("key.bin")).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_key_length_is_distinct_and_preserves_file() {
        let root = isolated_key_root("length");
        for bytes in [
            Vec::new(),
            protect(&[7; 31]).unwrap(),
            protect(&[7; 33]).unwrap(),
        ] {
            fs::write(root.join("key.bin"), &bytes).unwrap();
            let error = master_key(&root).unwrap_err();
            assert_eq!(
                error.downcast_ref::<DataError>(),
                Some(&DataError::KeyLength)
            );
            assert!(error.to_string().contains("key.bin 长度异常"));
            assert!(error.to_string().contains("请勿删除数据"));
            assert_eq!(fs::read(root.join("key.bin")).unwrap(), bytes);
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_key_refuses_to_create_when_database_exists() {
        let root = isolated_key_root("nokey");
        let database = b"synthetic-history-database".to_vec();
        fs::write(root.join("history.db"), &database).unwrap();
        let error = master_key(&root).unwrap_err();
        assert!(error.to_string().contains("拒绝生成新密钥"));
        assert!(!root.join("key.bin").exists(), "存在数据库时不得生成新密钥");
        assert_eq!(fs::read(root.join("history.db")).unwrap(), database);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fresh_root_generates_stable_key_and_leaves_database_absent() {
        let root = isolated_key_root("fresh");
        let first = master_key(&root).unwrap();
        assert!(root.join("key.bin").exists());
        let stored = fs::read(root.join("key.bin")).unwrap();
        assert!(!stored.is_empty());
        assert!(!root.join("history.db").exists(), "生成密钥不得创建数据库");
        let second = master_key(&root).unwrap();
        assert_eq!(*first, *second, "重复读取必须得到同一主密钥");
        assert_eq!(
            fs::read(root.join("key.bin")).unwrap(),
            stored,
            "已有密钥不得被改写"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn key_failures_leave_existing_database_untouched() {
        let root = isolated_key_root("preserve-db");
        let database = b"synthetic-history-database-bytes".to_vec();
        fs::write(root.join("history.db"), &database).unwrap();
        for bytes in [
            b"synthetic-invalid-dpapi".to_vec(),
            protect(&[7; 31]).unwrap(),
        ] {
            fs::write(root.join("key.bin"), &bytes).unwrap();
            let error = master_key(&root).unwrap_err();
            // 两类失败都必须保留 key.bin 与 history.db 原样，不得删除或重建。
            assert!(error.downcast_ref::<DataError>().is_some());
            assert_eq!(fs::read(root.join("key.bin")).unwrap(), bytes);
            assert_eq!(
                fs::read(root.join("history.db")).unwrap(),
                database,
                "密钥读取失败时不得改动数据库"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
