use super::*;
use std::io::Cursor;

fn seal(plain: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    encrypt(&mut Cursor::new(plain), &mut output, b"synthetic-password").unwrap();
    output
}
fn open(bytes: &[u8], password: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    decrypt(&mut Cursor::new(bytes), &mut output, password)?;
    Ok(output)
}
#[test]
fn empty_and_multiple_blocks_round_trip() {
    for plain in [vec![], vec![42; CHUNK_SIZE + 17]] {
        assert_eq!(open(&seal(&plain), b"synthetic-password").unwrap(), plain);
    }
}
#[test]
fn rejects_wrong_password_corruption_truncation_and_trailing_bytes() {
    let original = seal(b"synthetic content");
    assert!(open(&original, b"wrong-password").is_err());
    let mut corrupt = original.clone();
    corrupt[HEADER_SIZE + 4] ^= 1;
    assert!(open(&corrupt, b"synthetic-password").is_err());
    assert!(open(&original[..original.len() - 20], b"synthetic-password").is_err());
    let mut trailing = original;
    trailing.push(0);
    assert!(open(&trailing, b"synthetic-password").is_err());
}
#[test]
fn rejects_repeated_block_and_unknown_version() {
    let original = seal(&vec![17; CHUNK_SIZE + 3]);
    let block_end = HEADER_SIZE + 4 + CHUNK_SIZE + TAG_SIZE;
    let mut repeated = original[..block_end].to_vec();
    repeated.extend_from_slice(&original[HEADER_SIZE..]);
    assert!(open(&repeated, b"synthetic-password").is_err());
    let mut unknown = original;
    unknown[7] = b'9';
    assert!(open(&unknown, b"synthetic-password").is_err());
}
#[test]
fn fresh_exports_use_distinct_salt_and_nonce() {
    assert_ne!(seal(b"same"), seal(b"same"));
}

#[test]
fn rejects_unbounded_block_length_and_incomplete_header() {
    let mut oversized = seal(b"bounded");
    oversized[HEADER_SIZE..HEADER_SIZE + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(open(&oversized, b"synthetic-password")
        .unwrap_err()
        .to_string()
        .contains("大小限制"));
    assert!(open(&oversized[..HEADER_SIZE - 1], b"synthetic-password").is_err());
    assert!(open(&oversized, b"").is_err());
}

#[test]
fn propagates_output_failure_without_reporting_success() {
    struct FailedWriter;
    impl Write for FailedWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("synthetic disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let sealed = seal(b"synthetic");
    assert!(decrypt(
        &mut Cursor::new(sealed),
        &mut FailedWriter,
        b"synthetic-password"
    )
    .is_err());
    assert!(encrypt(
        &mut Cursor::new(b"synthetic"),
        &mut FailedWriter,
        b"synthetic-password"
    )
    .is_err());
}
