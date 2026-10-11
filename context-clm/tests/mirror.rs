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
    // A leading dot is encoded too (it was left as-is before fix round 1).
    assert_eq!(safe_id("../x"), "%2E.%2Fx");
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

#[test]
fn safe_id_is_never_a_dot_name_or_empty_and_stays_injective() {
    for sid in ["..", ".", ".x", "..."] {
        assert!(
            !safe_id(sid).starts_with('.'),
            "{sid:?} -> {}",
            safe_id(sid)
        );
    }
    assert_eq!(safe_id("."), "%2E");
    assert_eq!(safe_id(".."), "%2E.");
    assert_eq!(safe_id(""), "%e");
    let ids = [
        "", ".", "..", "...", ".x", "x.", "a.b", "%2E", "%2E.", "%252E", ".%2E", "%e", "%25e",
        "%h", "%2Eh",
    ];
    let encoded: std::collections::HashSet<String> = ids.iter().map(|s| safe_id(s)).collect();
    assert_eq!(encoded.len(), ids.len());
    // The mirror file name is never a dot-name either.
    assert_eq!(mirror_rel(".."), ".clm/%2E..ctx");
    assert_eq!(mirror_rel(""), ".clm/%e.ctx");
}

#[test]
fn invalid_utf8_is_replaced_not_rejected() {
    let root = tempfile::tempdir().unwrap();
    write_mirror(root.path(), "s_1", "x").unwrap();
    fs::write(root.path().join(".clm/s_1.ctx"), b"a\xffb").unwrap();
    assert_eq!(
        read_mirror(root.path(), "s_1").unwrap().as_deref(),
        Some("a\u{FFFD}b")
    );
}

#[cfg(unix)]
#[test]
fn symlinked_clm_directory_is_never_followed() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let kept = outside.path().join("s_1.ctx");
    let kept_tmp = outside.path().join("s_1.ctx.tmp");
    fs::write(&kept, "secret").unwrap();
    fs::write(&kept_tmp, "stale").unwrap();
    symlink(outside.path(), root.path().join(".clm")).unwrap();

    assert!(write_mirror(root.path(), "s_1", "x").is_err());
    assert!(read_mirror(root.path(), "s_1").is_err());
    assert!(remove_mirror(root.path(), "s_1").is_err());

    // Nothing outside was created, read into the result, replaced or unlinked.
    let mut names: Vec<_> = fs::read_dir(outside.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["s_1.ctx", "s_1.ctx.tmp"]);
    assert_eq!(fs::read_to_string(&kept).unwrap(), "secret");
    assert_eq!(fs::read_to_string(&kept_tmp).unwrap(), "stale");
}

#[cfg(unix)]
#[test]
fn clm_that_is_a_file_is_refused() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(".clm"), "not a dir").unwrap();
    assert!(write_mirror(root.path(), "s_1", "x").is_err());
    assert!(read_mirror(root.path(), "s_1").is_err());
    assert!(remove_mirror(root.path(), "s_1").is_err());
    assert_eq!(
        fs::read_to_string(root.path().join(".clm")).unwrap(),
        "not a dir"
    );
}

#[cfg(unix)]
#[test]
fn hardlinked_mirror_is_refused() {
    let root = tempfile::tempdir().unwrap();
    write_mirror(root.path(), "s_1", "x").unwrap();
    let mirror = root.path().join(".clm/s_1.ctx");
    let other = root.path().join("other");
    fs::remove_file(&mirror).unwrap();
    fs::write(&other, "linked").unwrap();
    fs::hard_link(&other, &mirror).unwrap();
    let err = read_mirror(root.path(), "s_1").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[cfg(unix)]
#[test]
fn existing_gitignore_is_left_alone_and_loose_dir_is_tightened() {
    use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret");
    fs::write(&secret, "do not touch").unwrap();
    let clm = root.path().join(".clm");
    fs::DirBuilder::new().mode(0o755).create(&clm).unwrap();
    fs::set_permissions(&clm, fs::Permissions::from_mode(0o755)).unwrap();
    symlink(&secret, clm.join(".gitignore")).unwrap();

    write_mirror(root.path(), "s_1", "x").unwrap();

    assert!(fs::symlink_metadata(clm.join(".gitignore"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_to_string(&secret).unwrap(), "do not touch");
    assert_eq!(
        fs::metadata(&clm).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(!clm.join(".gitignore.tmp").exists());
}
