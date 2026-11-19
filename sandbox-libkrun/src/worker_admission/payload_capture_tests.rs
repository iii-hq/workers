// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN (R4-NOATIME). Real core capture -> produce -> publish of a
//! symlink on whatever atime policy the host mounts (relatime included). Every
//! fixture object has an explicit mode; the symlink has explicit times.
use super::*;
use crate::worker_admission::{MutationPlan, PathRequest, PathRole, testing, with_paths};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};
/// Far older than the symlink's mtime and ctime: relatime and strictatime
/// both advance it on the next readlink, noatime never does.
const ATIME: (i64, i64) = (1_600_000_000, 7);
const MTIME: (i64, i64) = (1_600_000_001, 9);
struct Fixture {
    _temp: tempfile::TempDir,
    lease: Lease,
    source: PathBuf,
    target: PathBuf,
}
/// `source/` (0755) holding `data` (0644) and `link -> data`, admitted Strict.
fn fixture() -> Fixture {
    let temp = testing::private_tempdir().unwrap();
    let ns = temp.path().join("catalog");
    fs::create_dir(&ns).unwrap();
    fs::set_permissions(&ns, fs::Permissions::from_mode(0o700)).unwrap();
    let context = testing::provision_context(&ns).unwrap();
    let target = temp.path().join("payload");
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(source.join("data"), b"DATA").unwrap();
    fs::set_permissions(source.join("data"), fs::Permissions::from_mode(0o644)).unwrap();
    symlink("data", source.join("link")).unwrap();
    set_link_times(&source.join("link"), Some(ATIME), MTIME);
    let lease = with_paths(
        context.local(&target).unwrap(),
        &[PathRequest {
            path: &source,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    Fixture {
        _temp: temp,
        lease,
        source,
        target,
    }
}
/// utimensat(AT_SYMLINK_NOFOLLOW) on a fixture symlink; `None` keeps atime
/// (UTIME_OMIT). Fixture setup and negative-test mutation only: it advances
/// ctime, which is why the product never uses it to restore a source atime.
fn set_link_times(path: &Path, atime: Option<(i64, i64)>, mtime: (i64, i64)) {
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let spec = |time: Option<(i64, i64)>| match time {
        Some((sec, nsec)) => libc::timespec {
            tv_sec: sec as _,
            tv_nsec: nsec as _,
        },
        None => libc::timespec {
            tv_sec: 0,
            tv_nsec: libc::UTIME_OMIT,
        },
    };
    let times = [spec(atime), spec(Some(mtime))];
    let rc = unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            path.as_ptr(),
            times.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    assert_eq!(rc, 0, "{}", io::Error::last_os_error());
}
/// lstat only: never readlink, which would itself advance the atime under test.
fn lstat(path: &Path) -> fs::Metadata {
    fs::symlink_metadata(path).unwrap()
}
fn reclone(f: &Fixture, readable: ReadablePayload) -> io::Result<()> {
    let guard = f.lease.begin(MutationPlan::Reclone {
        root: f.target.clone(),
    })?;
    readable.produce(guard, &f.target)?.publish()
}
#[test]
fn r4_relatime_symlink_capture_tolerates_its_own_readlink_atime_and_clones_pre_read_atime() {
    let f = fixture();
    let link = f.source.join("link");
    let before = lstat(&link);
    assert_eq!((before.atime(), before.atime_nsec()), ATIME);
    let readable = f
        .lease
        .capture_payload(&f.source)
        .expect("symlink capture needs no noatime mount");
    // relatime/strictatime advanced the source atime here; nothing else moved.
    let read = lstat(&link);
    assert_eq!(
        (read.dev(), read.ino(), read.nlink(), read.mode()),
        (before.dev(), before.ino(), before.nlink(), before.mode())
    );
    assert_eq!(
        (read.ctime(), read.ctime_nsec()),
        (before.ctime(), before.ctime_nsec())
    );
    assert_eq!((read.mtime(), read.mtime_nsec()), MTIME);
    // Both produce pin passes and the seal's source-pin pass run after the read.
    reclone(&f, readable).expect("produce and publish accept the read-side atime");
    let clone = lstat(&f.target.join("link"));
    assert!(clone.file_type().is_symlink());
    assert_eq!((clone.atime(), clone.atime_nsec()), ATIME);
    assert_eq!((clone.mtime(), clone.mtime_nsec()), MTIME);
    // Content last: readlink may advance the atimes asserted above.
    assert_eq!(
        fs::read_link(f.target.join("link")).unwrap(),
        Path::new("data")
    );
    assert_eq!(fs::read(f.target.join("data")).unwrap(), b"DATA");
    let after = lstat(&link);
    assert_eq!(
        (after.ino(), after.ctime(), after.ctime_nsec()),
        (before.ino(), before.ctime(), before.ctime_nsec())
    );
}
#[test]
fn r4_symlink_rebound_between_pin_and_read_is_refused_before_effects() {
    let f = fixture();
    let link = f.source.join("link");
    let race = |relative: &Path| {
        assert_eq!(relative, Path::new("link"));
        let swap = f.source.join("swap");
        symlink("EVIL", &swap).unwrap();
        fs::rename(&swap, &link).unwrap();
    };
    let refused = f
        .lease
        .capture_payload_racing(&f.source, &race)
        .err()
        .expect("a symlink rebound after its pin is refused");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(refused.to_string(), "source replaced while reading link");
    assert!(!f.target.exists());
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("EVIL"));
}
#[test]
fn r4_symlink_mtime_and_ctime_change_after_capture_is_refused_by_its_pin() {
    let f = fixture();
    let readable = f
        .lease
        .capture_payload(&f.source)
        .expect("symlink capture needs no noatime mount");
    // Atime untouched (UTIME_OMIT); mtime moves and ctime advances. Only the
    // atime is exempt, so the symlink's own pin must refuse production.
    set_link_times(&f.source.join("link"), None, (MTIME.0 + 1, MTIME.1));
    let refused = reclone(&f, readable)
        .err()
        .expect("a changed symlink pin refuses production");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(refused.to_string(), "captured source changed");
    assert!(!f.target.exists());
}
