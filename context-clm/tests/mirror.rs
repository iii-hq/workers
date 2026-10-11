//! The mirror file: path encoding, atomic private writes, and refusal of
//! anything an agent could plant in its root.
use std::fs::{self, File};
use std::io;

use context_clm::mirror::{mirror_rel, read_mirror, remove_mirror, safe_id, write_mirror};

/// `read_mirror` refuses files above 16 MiB.
const MAX_MIRROR_BYTES: u64 = 16 << 20;

#[test]
fn safe_id_keeps_plain_ids_and_never_collides() {
    assert_eq!(safe_id("s_0123abcDEF.x-y"), "s_0123abcDEF.x-y");
    assert_eq!(safe_id("a/b"), "a%2Fb");
    assert_eq!(safe_id("a%2Fb"), "a%252Fb");
    assert_eq!(safe_id("../x"), "..%2Fx");
    assert_eq!(safe_id("é"), "%C3%A9");
    let long_a = "a".repeat(300);
    let long_b = "b".repeat(300);
    assert!(safe_id(&long_a).starts_with("%h"));
    assert_eq!(safe_id(&long_a).len(), 66);
    assert_ne!(safe_id(&long_a), safe_id(&long_b));
    assert_eq!(mirror_rel("s_1"), ".clm/s_1.ctx");
}

#[test]
fn write_then_read_round_trips_and_replaces() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(read_mirror(root.path(), "s_1").unwrap(), None);
    write_mirror(root.path(), "s_1", "[[CTX_TURN 1 role=user]]\nhi").unwrap();
    assert_eq!(
        read_mirror(root.path(), "s_1").unwrap().as_deref(),
        Some("[[CTX_TURN 1 role=user]]\nhi")
    );
    write_mirror(root.path(), "s_1", "second").unwrap();
    assert_eq!(
        read_mirror(root.path(), "s_1").unwrap().as_deref(),
        Some("second")
    );
    assert!(!root.path().join(".clm/s_1.ctx.tmp").exists());
    assert_eq!(
        fs::read_to_string(root.path().join(".clm/.gitignore")).unwrap(),
        "*\n"
    );
}

#[cfg(unix)]
#[test]
fn mirror_and_directory_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    write_mirror(root.path(), "s_1", "x").unwrap();
    let mode = |p: &str| {
        fs::metadata(root.path().join(p))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode(".clm"), 0o700);
    assert_eq!(mode(".clm/s_1.ctx"), 0o600);
}

#[test]
fn missing_root_is_not_created() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("gone");
    let err = write_mirror(&root, "s_1", "x").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    assert!(!root.exists());
    assert_eq!(read_mirror(&root, "s_1").unwrap(), None);
}

#[test]
fn remove_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    write_mirror(root.path(), "s_1", "x").unwrap();
    remove_mirror(root.path(), "s_1").unwrap();
    remove_mirror(root.path(), "s_1").unwrap();
    assert_eq!(read_mirror(root.path(), "s_1").unwrap(), None);
}

#[test]
fn oversized_mirror_is_refused() {
    let root = tempfile::tempdir().unwrap();
    write_mirror(root.path(), "s_1", "").unwrap();
    let big = File::create(root.path().join(".clm/s_1.ctx")).unwrap();
    big.set_len(MAX_MIRROR_BYTES + 1).unwrap();
    let err = read_mirror(root.path(), "s_1").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[cfg(unix)]
#[test]
fn planted_symlinks_are_never_followed() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret");
    fs::write(&secret, "do not touch").unwrap();
    write_mirror(root.path(), "s_1", "x").unwrap();

    // The mirror itself replaced by a link: reads refuse, writes replace the link.
    let mirror = root.path().join(".clm/s_1.ctx");
    fs::remove_file(&mirror).unwrap();
    symlink(&secret, &mirror).unwrap();
    assert!(read_mirror(root.path(), "s_1").is_err());
    write_mirror(root.path(), "s_1", "mine").unwrap();
    assert!(!fs::symlink_metadata(&mirror)
        .unwrap()
        .file_type()
        .is_symlink());

    // A link planted at the temp path is unlinked, not written through.
    symlink(&secret, root.path().join(".clm/s_1.ctx.tmp")).unwrap();
    write_mirror(root.path(), "s_1", "again").unwrap();
    assert_eq!(fs::read_to_string(&secret).unwrap(), "do not touch");

    // The mirror directory replaced by a link: refused.
    let other = tempfile::tempdir().unwrap();
    symlink(outside.path(), other.path().join(".clm")).unwrap();
    assert!(write_mirror(other.path(), "s_1", "x").is_err());
    assert!(!outside.path().join("s_1.ctx").exists());
}

#[cfg(unix)]
#[test]
fn fifo_mirror_is_refused_without_blocking() {
    let root = tempfile::tempdir().unwrap();
    write_mirror(root.path(), "s_1", "x").unwrap();
    let mirror = root.path().join(".clm/s_1.ctx");
    fs::remove_file(&mirror).unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(&mirror)
        .status()
        .unwrap()
        .success());
    assert!(read_mirror(root.path(), "s_1").is_err());
}
