//! 内部快照容器；恢复目标是独立暂存目录，成功后由上层决定是否切换。
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    path::Path,
};

#[path = "restore_content.rs"]
mod restore_content;
pub(super) use restore_content::PreparedRestore;

const BUNDLE_MAGIC: &[u8; 8] = b"CRAB0001";
const CONTENT_MAGIC: &[u8; 8] = b"CRCONT01";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    bytes: u64,
    hash: String,
}

/// 输入只能是已冻结的独立快照；输出由调用方独占创建并在成功后发布。
pub fn export(
    root: &Path,
    master: &[u8; 32],
    output: &mut impl Write,
    password: &[u8],
) -> Result<()> {
    let plan = PreparedExport::new(root, &control::Control::default())?;
    plan.write(root, master, output, password)
}

pub(super) struct PreparedExport {
    entries: Vec<Entry>,
    manifest: Vec<u8>,
}
impl PreparedExport {
    pub(super) fn new(root: &Path, control: &control::Control) -> Result<Self> {
        let entries = inventory(root, control)?;
        let manifest = serde_json::to_vec(&entries)?;
        if manifest.len() > CHUNK_SIZE {
            bail!("快照清单超过 1 MiB 上限");
        }
        Ok(Self { entries, manifest })
    }
    pub(super) fn bytes(&self) -> u64 {
        const BLOCK_OVERHEAD: u64 = 4 + TAG_SIZE as u64;
        let headers = 8 + HEADER_SIZE as u64 + 40 + self.manifest.len() as u64;
        headers
            + 3 * BLOCK_OVERHEAD
            + self
                .entries
                .iter()
                .map(|entry| entry.bytes + entry.bytes.div_ceil(CHUNK_SIZE as u64) * BLOCK_OVERHEAD)
                .sum::<u64>()
    }
    pub(super) fn write(
        &self,
        root: &Path,
        master: &[u8; 32],
        output: &mut impl Write,
        password: &[u8],
    ) -> Result<()> {
        let mut header = [0; HEADER_SIZE];
        header[..8].copy_from_slice(MAGIC);
        getrandom::fill(&mut header[8..]).map_err(|_| anyhow!("系统随机数不可用"))?;
        let mut cipher = Cipher::new(header, password)?;
        output.write_all(BUNDLE_MAGIC)?;
        output.write_all(&header)?;
        let mut secret = Zeroizing::new(CONTENT_MAGIC.to_vec());
        secret.extend_from_slice(master);
        cipher.write_block(output, &secret)?;
        cipher.write_block(output, &self.manifest)?;
        for entry in &self.entries {
            write_entry(root, entry, &mut cipher, output)?;
        }
        cipher.write_block(output, &[])
    }
}

/// 不覆盖任何目录；校验通过后才写 key.bin，失败保留暂存区，不更新启动定位。
pub fn restore(input: &mut impl Read, target: &Path, password: &[u8]) -> Result<()> {
    let control = control::Control::default();
    control.start()?;
    let mut plan = PreparedRestore::new(input, password)?;
    control.writing(plan.bytes())?;
    plan.extract(input, target, &control)
}

fn valid_name(name: &str) -> bool {
    if name == "history.db" {
        return true;
    }
    name.strip_prefix("data/blobs/").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn validate(entries: &[Entry]) -> Result<()> {
    let mut names = HashSet::new();
    let mut total = 0u64;
    for entry in entries {
        if !valid_name(&entry.name) || !names.insert(&entry.name) {
            bail!("快照清单路径非法或重复");
        }
        total = total.checked_add(entry.bytes).context("快照长度溢出")?;
        if total > MAX_ARCHIVE_BYTES {
            bail!("快照超过大小上限");
        }
    }
    if !names.contains(&"history.db".to_owned()) {
        bail!("快照缺少数据库");
    }
    Ok(())
}
fn plain_file(path: &Path) -> Result<File> {
    use std::os::windows::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        bail!("快照条目不是普通文件");
    }
    Ok(File::open(path)?)
}
fn inventory(root: &Path, control: &control::Control) -> Result<Vec<Entry>> {
    control.check()?;
    let mut names = vec!["history.db".to_owned()];
    for entry in fs::read_dir(root.join("data/blobs"))? {
        control.check()?;
        names.push(format!(
            "data/blobs/{}",
            entry?.file_name().to_str().context("对象名称无效")?
        ));
    }
    names.sort();
    let mut entries = Vec::new();
    for name in names {
        if !valid_name(&name) {
            bail!("快照存在非法条目");
        }
        let (bytes, hash) = fingerprint(&mut plain_file(&root.join(&name))?, control)?;
        entries.push(Entry { name, bytes, hash });
    }
    validate(&entries)?;
    Ok(entries)
}
fn fingerprint(input: &mut impl Read, control: &control::Control) -> Result<(u64, String)> {
    let mut digest = Sha256::new();
    let mut bytes = 0;
    let mut buffer = vec![0; CHUNK_SIZE];
    loop {
        control.check()?;
        let count = read_chunk(input, &mut buffer)?;
        if count == 0 {
            return Ok((bytes, format!("{:x}", digest.finalize())));
        }
        bytes = checked_total(bytes, count)?;
        digest.update(&buffer[..count]);
    }
}
fn write_entry(
    root: &Path,
    entry: &Entry,
    cipher: &mut Cipher,
    output: &mut impl Write,
) -> Result<()> {
    let mut input = plain_file(&root.join(&entry.name))?;
    let mut buffer = vec![0; CHUNK_SIZE];
    let mut digest = Sha256::new();
    let mut remaining = entry.bytes;
    while remaining > 0 {
        let count = remaining.min(CHUNK_SIZE as u64) as usize;
        input.read_exact(&mut buffer[..count])?;
        cipher.write_block(output, &buffer[..count])?;
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    require_eof(&mut input)?;
    if format!("{:x}", digest.finalize()) != entry.hash {
        bail!("导出期间快照发生变化");
    }
    Ok(())
}
#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;
