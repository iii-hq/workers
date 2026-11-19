// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN (R3-READ). Real core production -> seal ->
//! publish_into_guard_with_reader; the generic owner-read profile is exercised
//! unchanged. Every fixture object has an explicit mode.
use super::*;
use crate::worker_admission::{MutationPlan, PathRequest, PathRole, testing, with_paths};
use std::{fs, io::Cursor, os::unix::fs::PermissionsExt};
/// Beyond the generic OwnerReadGuard object cap (4096) and depth cap (64).
const MANY: usize = 4200;
const DEEP: usize = 70;
struct Fixture {
    _temp: tempfile::TempDir,
    context: crate::worker_admission::AdmissionContext,
    lease: Lease,
    data: PathBuf,
    target: PathBuf,
}
fn fixture() -> Fixture {
    let temp = testing::private_tempdir().unwrap();
    let ns = temp.path().join("catalog");
    fs::create_dir(&ns).unwrap();
    fs::set_permissions(&ns, fs::Permissions::from_mode(0o700)).unwrap();
    let context = testing::provision_context(&ns).unwrap();
    let data = temp.path().to_path_buf();
    let target = data.join("payload");
    let lease = context.local(&target).unwrap();
    Fixture {
        _temp: temp,
        context,
        lease,
        data,
        target,
    }
}
fn metadata(mode: u32) -> PayloadMetadata {
    PayloadMetadata {
        mode,
        uid: uid(),
        gid: unsafe { libc::getegid() },
        atime_sec: 1700000000,
        atime_nsec: 123,
        mtime_sec: 1700000001,
        mtime_nsec: 456,
        xattrs: BTreeMap::new(),
    }
}
fn entry(path: &str, mode: u32, kind: PayloadKind) -> PayloadEntry {
    PayloadEntry {
        relative: PathBuf::from(path),
        metadata: metadata(mode),
        kind,
    }
}
fn deep_dir(depth: usize) -> String {
    let mut path = String::from("deep");
    for level in 1..depth {
        path.push_str(&format!("/d{level}"));
    }
    path
}
/// The common OCI shapes the generic profile refuses: 1777 and 0777
/// directories, a 0666 file, an internal hardlink, a symlink and a 0000 file;
/// optionally more than 4096 members and a chain deeper than 64.
fn inventory(many: usize, deep: usize) -> Vec<PayloadEntry> {
    let mut entries = vec![
        entry("", 0o755, PayloadKind::Directory),
        entry("tmp", 0o1777, PayloadKind::Directory),
        entry("shared", 0o777, PayloadKind::Directory),
        entry("shared/file", 0o666, PayloadKind::File { size: 4 }),
        entry("data", 0o644, PayloadKind::File { size: 4 }),
        entry(
            "link",
            0o644,
            PayloadKind::Hardlink {
                source: PathBuf::from("data"),
            },
        ),
        entry(
            "sym",
            0o777,
            PayloadKind::Symlink {
                target: b"data".to_vec(),
            },
        ),
        entry("secret", 0o000, PayloadKind::File { size: 6 }),
    ];
    if many > 0 {
        entries.push(entry("many", 0o755, PayloadKind::Directory));
        for n in 0..many {
            entries.push(entry(
                &format!("many/f{n:05}"),
                0o644,
                PayloadKind::File { size: 1 },
            ));
        }
    }
    if deep > 0 {
        for depth in 1..=deep {
            entries.push(entry(&deep_dir(depth), 0o755, PayloadKind::Directory));
        }
        entries.push(entry(
            &format!("{}/leaf", deep_dir(deep)),
            0o644,
            PayloadKind::File { size: 4 },
        ));
    }
    entries
}
/// Real Reclone production through the core writer, sealed and published
/// with its retained read capability; the original guard then commits.
fn produce(f: &Fixture, many: usize, deep: usize) -> SealedPayloadRead {
    let guard = f
        .lease
        .begin(MutationPlan::Reclone {
            root: f.target.clone(),
        })
        .unwrap();
    let mut builder = guard
        .produce_payload(
            &f.target,
            PayloadInventory::new(inventory(many, deep)).unwrap(),
        )
        .unwrap();
    builder
        .write_file(Path::new("shared/file"), &mut Cursor::new(b"W666"))
        .unwrap();
    builder
        .write_file(Path::new("data"), &mut Cursor::new(b"DATA"))
        .unwrap();
    builder
        .write_file(Path::new("secret"), &mut Cursor::new(b"SECRET"))
        .unwrap();
    for n in 0..many {
        builder
            .write_file(Path::new(&format!("many/f{n:05}")), &mut Cursor::new(b"x"))
            .unwrap();
    }
    if deep > 0 {
        builder
            .write_file(
                Path::new(&format!("{}/leaf", deep_dir(deep))),
                &mut Cursor::new(b"DEEP"),
            )
            .unwrap();
    }
    let (guard, reader) = builder
        .seal()
        .unwrap()
        .publish_into_guard_with_reader()
        .unwrap();
    guard.commit().unwrap();
    reader
}
#[derive(Debug, PartialEq)]
enum Seen {
    Dir(u32),
    File(u32, Vec<u8>, (u64, u64)),
    Link(PathBuf),
}
/// The EROFS builder's walk shape: read_dir, original_metadata, open_file
/// (read to EOF) and read_link, all through the sealed capability.
fn walk(
    reader: &mut SealedPayloadRead,
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<PathBuf, Seen>,
) -> io::Result<()> {
    for member in reader.read_dir(dir)? {
        let path = dir.join(&member.name);
        let relative = path
            .strip_prefix(root)
            .map_err(io::Error::other)?
            .to_path_buf();
        let meta = reader.original_metadata(&path)?;
        let mode = meta.mode() & 0o7777;
        if meta.is_dir() {
            out.insert(relative, Seen::Dir(mode));
            walk(reader, root, &path, out)?;
        } else if meta.is_file() {
            let mut bytes = Vec::new();
            reader.open_file(&path)?.read_to_end(&mut bytes)?;
            out.insert(relative, Seen::File(mode, bytes, (meta.dev(), meta.ino())));
        } else {
            out.insert(relative, Seen::Link(reader.read_link(&path)?));
        }
    }
    Ok(())
}
fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().mode() & 0o7777
}
#[test]
fn r3_sealed_reader_reads_back_world_writable_dirs_hardlinks_symlinks_0000_and_many_entries() {
    let f = fixture();
    let mut reader = produce(&f, MANY, DEEP);
    let mut seen = BTreeMap::new();
    walk(&mut reader, &f.target, &f.target, &mut seen).unwrap();
    // Every sealed member below the root, exactly once.
    assert_eq!(seen.len(), inventory(MANY, DEEP).len() - 1);
    assert_eq!(seen[Path::new("tmp")], Seen::Dir(0o1777));
    assert_eq!(seen[Path::new("shared")], Seen::Dir(0o777));
    let Seen::File(mode, bytes, _) = &seen[Path::new("shared/file")] else {
        panic!("shared/file is a file");
    };
    assert_eq!((*mode, bytes.as_slice()), (0o666, &b"W666"[..]));
    // The 0000 member is read through the retained core descriptor.
    let Seen::File(mode, bytes, _) = &seen[Path::new("secret")] else {
        panic!("secret is a file");
    };
    assert_eq!((*mode, bytes.as_slice()), (0, &b"SECRET"[..]));
    // The internal hardlink is one inode with one content.
    let (Seen::File(_, a, first), Seen::File(_, b, second)) =
        (&seen[Path::new("data")], &seen[Path::new("link")])
    else {
        panic!("data and link are files");
    };
    assert_eq!((a.as_slice(), b.as_slice()), (&b"DATA"[..], &b"DATA"[..]));
    assert_eq!(first, second);
    assert_eq!(seen[Path::new("sym")], Seen::Link(PathBuf::from("data")));
    assert_eq!(
        seen.keys().filter(|p| p.starts_with("many")).count(),
        MANY + 1
    );
    let leaf = PathBuf::from(format!("{}/leaf", deep_dir(DEEP)));
    assert!(matches!(&seen[&leaf], Seen::File(0o644, bytes, _) if bytes == b"DEEP"));
    reader.finish().unwrap();
    // Nothing was granted or relabelled to read it.
    assert_eq!(mode_of(&f.target.join("secret")), 0);
    assert_eq!(mode_of(&f.target.join("tmp")), 0o1777);
    assert_eq!(mode_of(&f.target.join("shared")), 0o777);
    f.lease.revalidate().unwrap();
}
#[test]
fn r3_sealed_read_generic_owner_read_still_refuses_an_unsealed_world_writable_tree() {
    let f = fixture();
    let cache = f.data.join("cache");
    fs::create_dir(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    let open = cache.join("open");
    fs::create_dir(&open).unwrap();
    fs::set_permissions(&open, fs::Permissions::from_mode(0o777)).unwrap();
    let lease = with_paths(
        f.context.local(&f.data.join("consumer")).unwrap(),
        &[PathRequest {
            path: &cache,
            role: PathRole::ManagedCacheRead,
        }],
    )
    .unwrap();
    let refused = lease
        .owner_read_cache(&cache)
        .err()
        .expect("the generic owner-read profile is not widened");
    assert_eq!(
        refused.to_string(),
        "unowned or unsafe managed cache object"
    );
    assert_eq!(mode_of(&open), 0o777);
}
#[test]
fn r3_sealed_read_refuses_a_member_replaced_after_seal() {
    let f = fixture();
    let mut reader = produce(&f, 0, 0);
    fs::rename(f.target.join("data"), f.target.join("data.saved")).unwrap();
    fs::write(f.target.join("data"), b"DATA").unwrap();
    fs::set_permissions(f.target.join("data"), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(reader.open_file(&f.target.join("data")).is_err());
    assert!(reader.finish().is_err());
}
#[test]
fn r3_sealed_read_refuses_an_extra_member_in_a_sticky_world_writable_dir() {
    let f = fixture();
    let mut reader = produce(&f, 0, 0);
    let extra = f.target.join("tmp/extra");
    fs::write(&extra, b"x").unwrap();
    fs::set_permissions(&extra, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(reader.read_dir(&f.target.join("tmp")).is_err());
    assert!(reader.finish().is_err());
    assert!(extra.exists(), "refusal never deletes evidence");
}
#[test]
fn r3_sealed_read_refuses_an_external_hardlink_added_after_seal() {
    let f = fixture();
    let reader = produce(&f, 0, 0);
    fs::hard_link(f.target.join("data"), f.data.join("external")).unwrap();
    assert!(reader.original_metadata(&f.target.join("data")).is_err());
    assert!(reader.finish().is_err());
}
#[test]
fn r3_sealed_read_refuses_a_mode_changed_after_seal_even_if_restored() {
    let f = fixture();
    let mut reader = produce(&f, 0, 0);
    let shared = f.target.join("shared");
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(reader.read_dir(&shared).is_err());
    // Restoring the mode cannot restore ctime, and the failure is sticky.
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(reader.read_dir(&shared).is_err());
    assert!(reader.checkpoint().is_err());
    assert!(reader.finish().is_err());
}
#[test]
fn r3_sealed_read_refuses_in_place_content_change_of_a_world_writable_file() {
    let f = fixture();
    let mut reader = produce(&f, 0, 0);
    fs::write(f.target.join("shared/file"), b"EVIL").unwrap();
    assert!(reader.open_file(&f.target.join("shared/file")).is_err());
    assert!(reader.finish().is_err());
}
#[test]
fn r3_sealed_read_finish_refuses_an_unconsumed_member_changed_after_seal() {
    let f = fixture();
    let reader = produce(&f, 0, 0);
    // Never opened by the consumer: finish still re-verifies it.
    fs::write(f.target.join("shared/file"), b"EVIL").unwrap();
    assert!(reader.finish().is_err());
}
#[test]
fn r3_sealed_read_foreign_owner_never_enters_a_seal() {
    let mut entries = vec![
        entry("", 0o755, PayloadKind::Directory),
        entry("file", 0o644, PayloadKind::File { size: 1 }),
    ];
    entries[1].metadata.uid = uid().wrapping_add(1);
    assert!(PayloadInventory::new(entries).is_err());
}
#[test]
fn r3_sealed_file_refuses_bytes_that_differ_from_the_seal() {
    let f = fixture();
    let path = f.data.join("probe");
    fs::write(&path, b"DATA").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let verified = Arc::new(Mutex::new(BTreeSet::new()));
    let sealed = |expected: &[u8], failed: &Arc<AtomicBool>, size: u64| SealedFile {
        file: File::open(&path).unwrap(),
        size,
        at: 0,
        hash: Sha256::new(),
        expected: hex::encode(Sha256::digest(expected)),
        inode: (1, 2),
        done: false,
        failed: failed.clone(),
        verified: verified.clone(),
    };
    let failed = Arc::new(AtomicBool::new(false));
    let mut bytes = Vec::new();
    assert!(sealed(b"NOPE", &failed, 4).read_to_end(&mut bytes).is_err());
    assert!(failed.load(Ordering::Acquire));
    assert!(verified.lock().unwrap().is_empty());
    // A file shorter than its sealed size is refused, never zero-padded.
    let short = Arc::new(AtomicBool::new(false));
    let mut bytes = Vec::new();
    assert!(sealed(b"DATA", &short, 8).read_to_end(&mut bytes).is_err());
    assert!(short.load(Ordering::Acquire));
    // Exactly the sealed bytes verify and are recorded once consumed.
    let ok = Arc::new(AtomicBool::new(false));
    let mut bytes = Vec::new();
    sealed(b"DATA", &ok, 4).read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"DATA");
    assert!(verified.lock().unwrap().contains(&(1, 2)));
}
#[test]
fn r3_sealed_read_interrupted_read_has_no_catalog_or_tree_effect_and_is_refused_cleanly() {
    let f = fixture();
    let mut reader = produce(&f, 0, 0);
    let catalog = f.context.namespace.directory.path.join("catalog.json");
    let before = fs::read(&catalog).unwrap();
    let mut file = reader.open_file(&f.target.join("secret")).unwrap();
    let mut half = [0u8; 3];
    file.read_exact(&mut half).unwrap();
    assert_eq!(&half, b"SEC");
    // Interrupted: neither the file nor the capability is finished.
    drop(file);
    drop(reader);
    assert_eq!(fs::read(&catalog).unwrap(), before);
    f.lease.revalidate().unwrap();
    assert_eq!(mode_of(&f.target.join("secret")), 0);
    assert_eq!(mode_of(&f.target.join("tmp")), 0o1777);
}
#[test]
fn r3_sealed_read_finish_verifies_content_the_consumer_only_partly_read() {
    let f = fixture();
    let mut reader = produce(&f, 0, 0);
    let mut file = reader.open_file(&f.target.join("data")).unwrap();
    let mut first = [0u8; 2];
    file.read_exact(&mut first).unwrap();
    drop(file);
    // The partly read inode was never verified on EOF; finish hashes it in full.
    reader.finish().unwrap();
}
