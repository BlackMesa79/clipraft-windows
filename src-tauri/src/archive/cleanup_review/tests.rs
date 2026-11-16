use super::*;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("clipraft-cleanup-review-{}", nonce().unwrap()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn stage(&self) -> PathBuf {
        let directory = Directory::create(&self.0, "restore").unwrap();
        let path = directory.path().to_path_buf();
        fs::create_dir_all(path.join("database/data/blobs")).unwrap();
        fs::write(
            path.join("database/history.db"),
            b"encrypted partial database",
        )
        .unwrap();
        fs::write(
            path.join("database/data/blobs").join("a".repeat(64)),
            b"encrypted partial blob",
        )
        .unwrap();
        drop(directory);
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn review_reports_size_and_confirm_removes_only_approved_task() {
    let f = Fixture::new();
    let root = f.stage();
    fs::write(f.0.join("keep"), b"user file").unwrap();
    let review = Review::prepare(&root).unwrap();
    assert_eq!(review.preview.files, 2);
    assert!(review.preview.bytes > 0);
    let ticket = review.preview.ticket.clone();
    assert!(Directory::claim(&root).unwrap().is_none());
    review.confirm(&ticket).unwrap();
    assert!(!root.exists());
    assert_eq!(fs::read(f.0.join("keep")).unwrap(), b"user file");
}
#[test]
fn changed_or_replaced_file_and_stale_ticket_are_preserved() {
    let f = Fixture::new();
    let root = f.stage();
    let review = Review::prepare(&root).unwrap();
    let ticket = review.preview.ticket.clone();
    fs::write(root.join("database/history.db"), b"changed").unwrap();
    assert!(review.confirm(&ticket).is_err());
    assert!(root
        .join("database/data/blobs")
        .join("a".repeat(64))
        .exists());
    let review = Review::prepare(&root).unwrap();
    fs::rename(root.join("database/history.db"), root.join("saved")).unwrap();
    fs::write(root.join("database/history.db"), b"changed").unwrap();
    let ticket = review.preview.ticket.clone();
    assert!(review.confirm(&ticket).is_err());
    assert!(Review::prepare(&root).is_err());
    fs::remove_file(root.join("saved")).unwrap();
    assert!(Review::prepare(&root)
        .unwrap()
        .confirm("wrong-ticket")
        .is_err());
    assert!(root.join("database/history.db").exists());
}
#[test]
fn unknown_content_or_active_task_cannot_be_prepared() {
    let f = Fixture::new();
    let root = f.stage();
    fs::write(root.join("notes.txt"), b"keep").unwrap();
    assert!(Review::prepare(&root).is_err());
    fs::remove_file(root.join("notes.txt")).unwrap();
    let owner = Directory::claim(&root).unwrap().unwrap();
    assert!(Review::prepare(&root).is_err());
    drop(owner);
    assert!(Review::prepare(&root).is_ok());
}
#[test]
fn new_unknown_file_after_review_is_preserved_and_retains_owner_marker() {
    let f = Fixture::new();
    let root = f.stage();
    let review = Review::prepare(&root).unwrap();
    let ticket = review.preview.ticket.clone();
    fs::write(root.join("database/user.txt"), b"keep").unwrap();
    assert!(review.confirm(&ticket).is_err());
    assert_eq!(fs::read(root.join("database/user.txt")).unwrap(), b"keep");
    assert!(root.join(super::super::staging_owner::NAME).exists());
}
