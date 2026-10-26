// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Integration tests for OCI worker operations.
//! Covers requirements OCI-01 through OCI-05.
//!
//! Tests are split into ungated (pure/filesystem) and gated (network-dependent):
//! - Ungated: extract_layer_with_limits safety, oci_image_for_kind, OCI config env reading
//! - Gated (#[cfg(feature = "integration-oci")]): pull_and_extract_rootfs end-to-end

use sandbox_libkrun::cli::worker_manager::oci::{
    extract_layer_with_limits, oci_image_for_kind, read_oci_env,
};

/// Build a tar.gz archive from a list of (path, content, mode) entries.
fn make_layer_targz(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut archive = tar::Builder::new(&mut encoder);
        for (path, content, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            archive.append(&header, *content as &[u8]).unwrap();
        }
        archive.finish().unwrap();
    }
    encoder.finish().unwrap()
}

/// Build raw tar bytes with a custom path, bypassing tar::Builder path validation.
/// This is needed to test path traversal and absolute path rejection.
fn make_raw_tar_gz(path_bytes: &[u8], data: &[u8]) -> Vec<u8> {
    use std::io::Write;

    let mut raw_tar = Vec::new();

    // 512-byte GNU tar header
    let mut header_block = [0u8; 512];
    header_block[..path_bytes.len()].copy_from_slice(path_bytes);
    // mode (offset 100, 8 bytes)
    header_block[100..107].copy_from_slice(b"0000644");
    // size (offset 124, 12 bytes) -- octal
    let size_str = format!("{:011o}", data.len());
    header_block[124..135].copy_from_slice(size_str.as_bytes());
    // typeflag (offset 156) -- '0' regular file
    header_block[156] = b'0';
    // magic (offset 257, 6 bytes) + version (offset 263, 2 bytes)
    header_block[257..263].copy_from_slice(b"ustar\0");
    header_block[263..265].copy_from_slice(b"00");
    // checksum (offset 148, 8 bytes): fill with spaces, compute, then write
    header_block[148..156].copy_from_slice(b"        ");
    let cksum: u32 = header_block.iter().map(|&b| b as u32).sum();
    let cksum_str = format!("{:06o}\0 ", cksum);
    header_block[148..156].copy_from_slice(cksum_str.as_bytes());

    raw_tar.extend_from_slice(&header_block);
    raw_tar.extend_from_slice(data);
    // Pad to 512-byte boundary
    let padding = 512 - (data.len() % 512);
    if padding < 512 {
        raw_tar.extend(std::iter::repeat_n(0u8, padding));
    }
    // Two zero blocks to end the archive
    raw_tar.extend(std::iter::repeat_n(0u8, 1024));

    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&raw_tar).unwrap();
    gz.finish().unwrap()
}

// =============================================================================
// Group 1: Layer extraction -- valid (OCI-02)
// =============================================================================

#[test]
fn extract_layer_valid_single_file() {
    let dir = tempfile::tempdir().unwrap();
    let layer = make_layer_targz(&[("test.txt", b"hello world", 0o644)]);

    let mut total_size = 0u64;
    extract_layer_with_limits(&layer, dir.path(), 0, 1, &mut total_size).unwrap();

    let content = std::fs::read_to_string(dir.path().join("test.txt")).unwrap();
    assert_eq!(content, "hello world");
    assert_eq!(total_size, 11);
}

#[test]
fn extract_layer_valid_multiple_files() {
    let dir = tempfile::tempdir().unwrap();
    let layer = make_layer_targz(&[
        ("bin/app", b"binary", 0o755),
        ("etc/config", b"key=value", 0o644),
    ]);

    let mut total_size = 0u64;
    extract_layer_with_limits(&layer, dir.path(), 0, 1, &mut total_size).unwrap();

    let app_content = std::fs::read_to_string(dir.path().join("bin/app")).unwrap();
    assert_eq!(app_content, "binary");

    let config_content = std::fs::read_to_string(dir.path().join("etc/config")).unwrap();
    assert_eq!(config_content, "key=value");
}

#[cfg(unix)]
#[test]
fn extract_layer_strips_setuid_and_setgid_bits() {
    // Host extraction runs as a regular user, so setuid binaries in the
    // layer end up owned by that user on disk. PassthroughFs surfaces host
    // ownership verbatim to the guest, so a setuid binary inside the VM
    // *drops* privileges from PID-1 root to the host user's UID on exec,
    // which is exactly what broke `mount --bind` during worker startup.
    // Strip the setid bits at extraction time so the guest inherits the
    // PID-1 euid.
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let layer = make_layer_targz(&[
        ("bin/mount", b"fake mount binary", 0o4755), // setuid
        ("bin/wall", b"fake wall binary", 0o2755),   // setgid
        ("bin/odd", b"suid+sgid", 0o6755),           // setuid + setgid
        ("bin/plain", b"plain binary", 0o0755),      // control: no setid
    ]);

    let mut total_size = 0u64;
    extract_layer_with_limits(&layer, dir.path(), 0, 1, &mut total_size).unwrap();

    let mode = |p: &str| -> u32 {
        std::fs::metadata(dir.path().join(p))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777
    };

    assert_eq!(mode("bin/mount") & 0o6000, 0, "setuid bit must be stripped");
    assert_eq!(mode("bin/wall") & 0o6000, 0, "setgid bit must be stripped");
    assert_eq!(
        mode("bin/odd") & 0o6000,
        0,
        "setuid+setgid must be stripped"
    );
    assert_eq!(mode("bin/plain") & 0o777, 0o755, "plain perms survive");
}

#[cfg(unix)]
#[test]
fn extract_layer_does_not_chmod_symlinks() {
    // std::fs::set_permissions follows symlinks on Unix. The setid-stripping
    // guard must exclude symlinks — otherwise a symlink with setid bits in
    // its header would chase the link and strip perms from the target
    // (potentially a file outside the rootfs). Build a tar with a symlink
    // whose header claims setuid+setgid and verify the link's target file
    // is untouched.
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();

    // Pre-create the target file with setid bits set on disk. If the
    // symlink guard breaks, set_permissions("bin/link", ...) would follow
    // the link and strip setid from this sentinel. Without the setid bits
    // here, the inner `current & SETID_BITS != 0` gate would short-circuit
    // before the chmod — so the test would pass even with a broken guard.
    // The sentinel is NOT in the tarball (extraction would overwrite its
    // mode with the header's 0o755), so we create it manually first.
    std::fs::create_dir_all(dir.path().join("bin")).unwrap();
    std::fs::write(dir.path().join("bin/real"), b"data").unwrap();
    std::fs::set_permissions(
        dir.path().join("bin/real"),
        std::fs::Permissions::from_mode(0o6755),
    )
    .unwrap();

    let layer = {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut archive = tar::Builder::new(&mut encoder);

            // Symlink: header claims setuid+setgid. If the guard breaks,
            // chmod would follow `link` to `real` and strip its perms.
            let mut link_header = tar::Header::new_gnu();
            link_header.set_size(0);
            link_header.set_mode(0o6755);
            link_header.set_entry_type(tar::EntryType::Symlink);
            link_header.set_cksum();
            archive
                .append_link(&mut link_header, "bin/link", "real")
                .unwrap();

            archive.finish().unwrap();
        }
        encoder.finish().unwrap()
    };

    let mut total_size = 0u64;
    extract_layer_with_limits(&layer, dir.path(), 0, 1, &mut total_size).unwrap();

    let link_meta = std::fs::symlink_metadata(dir.path().join("bin/link")).unwrap();
    assert!(
        link_meta.file_type().is_symlink(),
        "entry must remain a symlink"
    );
    let link_target = std::fs::read_link(dir.path().join("bin/link")).unwrap();
    assert_eq!(link_target.to_str(), Some("real"));

    // The sentinel target must still have its setid bits. If the symlink
    // guard broke, set_permissions on `link` would have followed to `real`
    // and stripped them.
    let target_mode = std::fs::metadata(dir.path().join("bin/real"))
        .unwrap()
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(
        target_mode & 0o6000,
        0o6000,
        "sentinel target must retain setid bits; a broken symlink guard would have stripped them"
    );
    assert_eq!(target_mode & 0o777, 0o755, "target base perms unchanged");
}

#[test]
fn extract_layer_preserves_directory_structure() {
    let dir = tempfile::tempdir().unwrap();
    let layer = make_layer_targz(&[("usr/local/bin/tool", b"#!/bin/sh\necho hi", 0o755)]);

    let mut total_size = 0u64;
    extract_layer_with_limits(&layer, dir.path(), 0, 1, &mut total_size).unwrap();

    assert!(dir.path().join("usr/local/bin/tool").exists());
    let content = std::fs::read_to_string(dir.path().join("usr/local/bin/tool")).unwrap();
    assert_eq!(content, "#!/bin/sh\necho hi");
}

// =============================================================================
// Group 2: Layer extraction -- safety rejections (OCI-04)
// =============================================================================

#[test]
fn extract_layer_rejects_path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let gz_data = make_raw_tar_gz(b"../escape.txt", b"malicious content");

    let mut total_size = 0u64;
    let result = extract_layer_with_limits(&gz_data, dir.path(), 0, 1, &mut total_size);
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("path traversal"), "got: {}", err_msg);
}

#[test]
fn extract_layer_rejects_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let gz_data = make_raw_tar_gz(b"/etc/passwd", b"root:x:0:0");

    let mut total_size = 0u64;
    let result = extract_layer_with_limits(&gz_data, dir.path(), 0, 1, &mut total_size);
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("absolute path"), "got: {}", err_msg);
}

#[test]
fn extract_layer_rejects_oversized_total() {
    let dir = tempfile::tempdir().unwrap();
    // Create a small valid tar.gz with a 2-byte file
    let layer = make_layer_targz(&[("small.txt", b"ab", 0o644)]);

    // Pre-set total_size to just below MAX_TOTAL_SIZE (10 GiB - 1 byte)
    let mut total_size: u64 = 10 * 1024 * 1024 * 1024 - 1;
    let result = extract_layer_with_limits(&layer, dir.path(), 0, 1, &mut total_size);
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("total extraction size exceeded"),
        "got: {}",
        err_msg
    );
}

#[test]
fn extract_layer_handles_whiteout_files() {
    let dir = tempfile::tempdir().unwrap();

    // Create a pre-existing file that the whiteout should remove
    let target_path = dir.path().join("deleted-file");
    std::fs::write(&target_path, "should be removed").unwrap();
    assert!(target_path.exists());

    // Create a tar.gz with a whiteout entry
    let layer = make_layer_targz(&[(".wh.deleted-file", b"", 0o644)]);

    let mut total_size = 0u64;
    extract_layer_with_limits(&layer, dir.path(), 0, 1, &mut total_size).unwrap();

    // The whiteout should have removed the pre-existing file
    assert!(
        !target_path.exists(),
        "whiteout should have removed 'deleted-file'"
    );
}

/// Build a tar.gz from (path, `Some(content)` for a file / `None` for a
/// directory, mode) entries.
#[cfg(unix)]
fn make_layer_with_dirs(entries: &[(&str, Option<&[u8]>, u32)]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut archive = tar::Builder::new(&mut encoder);
        for (path, content, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_mode(*mode);
            let data: &[u8] = match content {
                Some(c) => {
                    header.set_entry_type(tar::EntryType::Regular);
                    c
                }
                None => {
                    header.set_entry_type(tar::EntryType::Directory);
                    &[]
                }
            };
            header.set_size(data.len() as u64);
            header.set_cksum();
            archive.append(&header, data).unwrap();
        }
        archive.finish().unwrap();
    }
    encoder.finish().unwrap()
}

/// Red Hat images ship directories without owner write bits (UBI 9:
/// `/usr/bin`, `/usr/lib`, `/usr/sbin` and ca-trust's `directory-hash` are
/// 0555, `/root` is 0550), each followed by its own entries. An
/// unprivileged puller must still extract those entries, apply a later
/// layer's files and whiteouts inside them, and be able to delete the tree
/// (failed staging dir, stale cache). Only the owner bits may change: a
/// 0000 directory gains exactly u+rwx, and special bits (sticky) are kept.
#[cfg(unix)]
#[test]
fn extract_layer_handles_directories_without_owner_write() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let rootfs = dir.path().join("rootfs");
    std::fs::create_dir(&rootfs).unwrap();

    let layer1 = make_layer_with_dirs(&[
        ("usr/", None, 0o755),
        ("usr/bin/", None, 0o555),
        ("usr/bin/tool", Some(b"v1"), 0o755),
        ("usr/bin/old", Some(b"old"), 0o755),
        ("root/", None, 0o550),
        ("root/.bashrc", Some(b"rc"), 0o644),
        ("etc/", None, 0o755),
        ("etc/shadow", Some(b"root:!::"), 0o000),
        ("locked/", None, 0o000),
        ("locked/f", Some(b"x"), 0o644),
        ("spool/", None, 0o1555),
    ]);
    let layer2 = make_layer_with_dirs(&[
        ("usr/bin/", None, 0o555),
        ("usr/bin/tool", Some(b"v2"), 0o755),
        ("usr/bin/.wh.old", Some(b""), 0o644),
    ]);

    let mut total_size = 0u64;
    extract_layer_with_limits(&layer1, &rootfs, 0, 2, &mut total_size)
        .expect("layer 1: entries inside a 0555 directory must extract");
    extract_layer_with_limits(&layer2, &rootfs, 1, 2, &mut total_size)
        .expect("layer 2: files and whiteouts inside a 0555 directory must apply");

    assert_eq!(std::fs::read(rootfs.join("usr/bin/tool")).unwrap(), b"v2");
    assert!(
        !rootfs.join("usr/bin/old").exists(),
        "whiteout inside a 0555 directory must delete"
    );
    assert_eq!(std::fs::read(rootfs.join("root/.bashrc")).unwrap(), b"rc");

    let mode = |p: &str| {
        std::fs::symlink_metadata(rootfs.join(p))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777
    };
    assert_eq!(mode("usr/bin"), 0o755, "owner gains rwx, group/other kept");
    assert_eq!(mode("root"), 0o750, "owner gains rwx, group/other kept");
    assert_eq!(
        mode("locked"),
        0o700,
        "a 0000 directory gains exactly u+rwx"
    );
    assert_eq!(std::fs::read(rootfs.join("locked/f")).unwrap(), b"x");
    assert_eq!(mode("spool"), 0o1755, "special bits are kept");
    assert_eq!(mode("etc/shadow"), 0o000, "file modes are not touched");

    std::fs::remove_dir_all(&rootfs).expect("an extracted tree must stay deletable");
}

// =============================================================================
// Group 3: Image-to-language mapping (OCI-05)
// =============================================================================

#[test]
fn oci_image_for_typescript() {
    assert_eq!(
        oci_image_for_kind("typescript"),
        "docker.io/iiidev/node:latest"
    );
}

#[test]
fn oci_image_for_javascript() {
    assert_eq!(
        oci_image_for_kind("javascript"),
        "docker.io/iiidev/node:latest"
    );
}

#[test]
fn oci_image_for_python() {
    assert_eq!(
        oci_image_for_kind("python"),
        "docker.io/iiidev/python:latest"
    );
}

#[test]
fn oci_image_for_rust() {
    assert_eq!(
        oci_image_for_kind("rust"),
        "docker.io/library/rust:slim-bookworm"
    );
}

#[test]
fn oci_image_for_unknown_defaults_to_node() {
    assert_eq!(oci_image_for_kind("go"), "docker.io/iiidev/node:latest");

    assert_eq!(
        oci_image_for_kind("unknown_lang"),
        "docker.io/iiidev/node:latest"
    );
}

// =============================================================================
// Group 4: OCI config reading (OCI-03)
// =============================================================================

#[test]
fn read_oci_env_parses_key_value() {
    let dir = tempfile::tempdir().unwrap();
    let config = r#"{"config": {"Env": ["PATH=/usr/bin", "HOME=/root"]}}"#;
    std::fs::write(dir.path().join(".oci-config.json"), config).unwrap();

    let env = read_oci_env(dir.path());
    assert_eq!(
        env,
        vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/root".to_string()),
        ]
    );
}

#[test]
fn read_oci_env_empty() {
    let dir = tempfile::tempdir().unwrap();
    let config = r#"{"config": {}}"#;
    std::fs::write(dir.path().join(".oci-config.json"), config).unwrap();

    let env = read_oci_env(dir.path());
    assert!(env.is_empty());
}

// =============================================================================
// Group 5: Feature-gated network tests (OCI-01)
// =============================================================================
// Previous pull_and_extract_rootfs_placeholder was a type-check-only
// stub — removed. Type checking is handled by
// `cargo check --features integration-oci`.
