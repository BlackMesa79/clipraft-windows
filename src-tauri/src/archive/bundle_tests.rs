use super::*;

fn entry(name: &str, bytes: u64) -> Entry {
    Entry {
        name: name.into(),
        bytes,
        hash: "0".repeat(64),
    }
}

#[test]
fn manifest_rejects_paths_duplicates_missing_database_and_overflow() {
    for name in [
        "../outside",
        "data/blobs/../key.bin",
        "C:/outside",
        "history.db:stream",
        "key.bin",
    ] {
        assert!(validate(&[entry("history.db", 1), entry(name, 1)]).is_err());
    }
    assert!(validate(&[entry("history.db", 1), entry("history.db", 1)]).is_err());
    assert!(validate(&[]).is_err());
    assert!(validate(&[entry("history.db", u64::MAX)]).is_err());
    assert!(validate(&[
        entry("history.db", 1),
        entry(&format!("data/blobs/{}", "a".repeat(64)), 1)
    ])
    .is_ok());
}
