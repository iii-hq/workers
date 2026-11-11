// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Build mode: once the build script exits 0, write the merged root to a tar
//! file the host turns into the image (`sandbox-libkrun worker::build`).
//!
//! The walk stays on the root filesystem, so the kernel filesystems and every
//! share mounted into the build VM stay out, and [`EXCLUDED`] names the paths
//! that stay out whatever is mounted on them. Regular files, directories,
//! symlinks and hard links are written with their mode, owner and mtime;
//! device nodes, FIFOs and sockets are not, as the host's image builder
//! carries none.

use std::collections::HashMap;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Where the host wants the tar, a path on a share it reads back. Set only for a
/// build VM, by the host, in the worker env frame.
pub const III_BUILD_EXPORT_ENV: &str = "III_BUILD_EXPORT";

/// Paths, relative to the root, that never reach the image: the kernel
/// filesystems and runtime dirs, and the build VM's virtiofs mount points.
pub const EXCLUDED: &[&str] = &[
    "proc",
    "sys",
    "dev",
    "run",
    "tmp",
    "opt/iii",
    "mnt/host-src",
];

/// The export destination, taken out of the environment so the build's own
/// scripts never see it. `None` outside a build VM.
pub fn take_export_dest() -> Option<PathBuf> {
    let dest = std::env::var_os(III_BUILD_EXPORT_ENV).filter(|v| !v.is_empty())?;
    // SAFETY: called by PID 1 before it spawns the worker or any helper thread.
    unsafe { std::env::remove_var(III_BUILD_EXPORT_ENV) };
    Some(PathBuf::from(dest))
}

/// Write the tree under `root` to `dest` as a tar stream; returns the number of
/// entries written. Whatever was at `dest` is replaced, never followed.
pub fn export_root(root: &Path, dest: &Path) -> io::Result<u64> {
    match std::fs::remove_file(dest) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dest)?;
    let root_meta = std::fs::symlink_metadata(root)?;
    let mut export = Export {
        tar: tar::Builder::new(BufWriter::with_capacity(1 << 20, file)),
        root_dev: root_meta.dev(),
        links: HashMap::new(),
        entries: 0,
    };
    let mut header = header(&root_meta, tar::EntryType::Directory);
    export.tar.append_data(&mut header, "./", io::empty())?;
    export.entries += 1;
    export.walk(root, Path::new(""))?;
    let file = export
        .tar
        .into_inner()?
        .into_inner()
        .map_err(|e| e.into_error())?;
    file.sync_all()?;
    Ok(export.entries)
}

struct Export {
    tar: tar::Builder<BufWriter<File>>,
    root_dev: u64,
    /// The first path written for each multiply-linked file, by (dev, inode).
    links: HashMap<(u64, u64), PathBuf>,
    entries: u64,
}

impl Export {
    fn walk(&mut self, dir: &Path, rel: &Path) -> io::Result<()> {
        let mut names: Vec<_> = std::fs::read_dir(dir)?
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<io::Result<_>>()?;
        names.sort();
        for name in names {
            let path = dir.join(&name);
            let rel = rel.join(&name);
            if is_excluded(&rel) {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path)?;
            let kind = meta.file_type();
            if kind.is_dir() {
                // A mount point: another filesystem's root.
                if meta.dev() != self.root_dev {
                    continue;
                }
                let mut header = header(&meta, tar::EntryType::Directory);
                self.tar.append_data(&mut header, &rel, io::empty())?;
                self.entries += 1;
                self.walk(&path, &rel)?;
            } else if kind.is_symlink() {
                let target = std::fs::read_link(&path)?;
                let mut header = header(&meta, tar::EntryType::Symlink);
                self.tar.append_link(&mut header, &rel, &target)?;
                self.entries += 1;
            } else if kind.is_file() {
                self.file(&path, &rel, &meta)?;
            }
            // Device nodes, FIFOs and sockets: not carried by the image.
        }
        Ok(())
    }

    fn file(&mut self, path: &Path, rel: &Path, meta: &Metadata) -> io::Result<()> {
        if meta.nlink() > 1 {
            let key = (meta.dev(), meta.ino());
            if let Some(first) = self.links.get(&key) {
                let mut header = header(meta, tar::EntryType::Link);
                header.set_size(0);
                let first = first.clone();
                self.tar.append_link(&mut header, rel, &first)?;
                self.entries += 1;
                return Ok(());
            }
            self.links.insert(key, rel.to_path_buf());
        }
        let len = meta.len();
        let mut header = header(meta, tar::EntryType::Regular);
        // Exactly `len` bytes, whatever the file does meanwhile: a file still
        // being written must not run past its header.
        let data = File::open(path)?.take(len).chain(io::repeat(0)).take(len);
        self.tar.append_data(&mut header, rel, data)?;
        self.entries += 1;
        Ok(())
    }
}

fn is_excluded(rel: &Path) -> bool {
    let rel = rel.as_os_str().as_bytes();
    EXCLUDED.iter().any(|ex| rel == ex.as_bytes())
}

fn header(meta: &Metadata, kind: tar::EntryType) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_mode(meta.mode() & 0o7777);
    header.set_uid(u64::from(meta.uid()));
    header.set_gid(u64::from(meta.gid()));
    header.set_mtime(meta.mtime().max(0) as u64);
    header.set_size(if kind == tar::EntryType::Regular {
        meta.len()
    } else {
        0
    });
    header
}

/// Flushed to the console, so the host's build log shows how the export went.
pub fn log(line: &str) {
    let _ = writeln!(io::stderr(), "iii-init: {line}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn entries(tar_path: &Path) -> Vec<(String, tar::EntryType, Vec<u8>, String)> {
        let mut archive = tar::Archive::new(File::open(tar_path).unwrap());
        archive
            .entries()
            .unwrap()
            .map(|e| {
                let mut e = e.unwrap();
                let name = String::from_utf8_lossy(&e.path_bytes()).into_owned();
                let kind = e.header().entry_type();
                let link = e
                    .link_name_bytes()
                    .map(|l| String::from_utf8_lossy(&l).into_owned())
                    .unwrap_or_default();
                let mut data = Vec::new();
                e.read_to_end(&mut data).unwrap();
                (name, kind, data, link)
            })
            .collect()
    }

    #[test]
    fn the_tree_is_written_without_the_excluded_paths() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        for dir in [
            "usr/bin",
            "proc/1",
            "tmp",
            "opt/iii",
            "opt/app",
            "mnt/host-src",
            "workspace/node_modules/x",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("usr/bin/tool"), "tool").unwrap();
        std::fs::set_permissions(
            root.join("usr/bin/tool"),
            std::fs::Permissions::from_mode(0o4755),
        )
        .unwrap();
        std::fs::write(root.join("proc/1/status"), "x").unwrap();
        std::fs::write(root.join("tmp/scratch"), "x").unwrap();
        std::fs::write(root.join("opt/iii/build.sh"), "x").unwrap();
        std::fs::write(root.join("opt/app/keep"), "keep").unwrap();
        std::fs::write(root.join("mnt/host-src/code.js"), "x").unwrap();
        std::fs::write(root.join("workspace/node_modules/x/index.js"), "dep").unwrap();
        std::fs::hard_link(root.join("usr/bin/tool"), root.join("usr/bin/tool2")).unwrap();
        symlink("tool", root.join("usr/bin/alias")).unwrap();
        let long = "d".repeat(120);
        std::fs::create_dir_all(root.join(&long).join(&long)).unwrap();
        std::fs::write(root.join(&long).join(&long).join("f"), "deep").unwrap();
        nix::unistd::mkfifo(&root.join("fifo"), nix::sys::stat::Mode::S_IRWXU).unwrap();

        let dest = t.path().join("root.tar");
        std::fs::write(&dest, "stale").unwrap();
        export_root(&root, &dest).unwrap();

        let got = entries(&dest);
        let names: Vec<&str> = got.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(names[0], "./");
        for gone in ["proc", "tmp", "opt/iii", "mnt/host-src", "fifo"] {
            assert!(
                !names
                    .iter()
                    .any(|n| n.trim_end_matches('/') == gone || n.starts_with(&format!("{gone}/"))),
                "{gone} was exported: {names:?}"
            );
        }
        let find = |n: &str| got.iter().find(|e| e.0.trim_end_matches('/') == n).unwrap();
        assert_eq!(find("opt/app/keep").2, b"keep");
        assert_eq!(find("workspace/node_modules/x/index.js").2, b"dep");
        assert_eq!(find("usr/bin/tool").2, b"tool");
        assert!(
            names.contains(&"mnt") || names.contains(&"mnt/"),
            "{names:?}"
        );
        let alias = find("usr/bin/alias");
        assert_eq!(
            (alias.1, alias.3.as_str()),
            (tar::EntryType::Symlink, "tool")
        );
        let hard = find("usr/bin/tool2");
        assert_eq!(
            (hard.1, hard.3.as_str()),
            (tar::EntryType::Link, "usr/bin/tool")
        );
        assert_eq!(find(&format!("{long}/{long}/f")).2, b"deep");
        let mut archive = tar::Archive::new(File::open(&dest).unwrap());
        let tool = archive
            .entries()
            .unwrap()
            .map(Result::unwrap)
            .find(|e| e.path_bytes().as_ref() == b"usr/bin/tool")
            .unwrap();
        assert_eq!(tool.header().mode().unwrap(), 0o4755);
    }

    #[test]
    fn a_planted_symlink_at_the_destination_is_replaced_not_followed() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        std::fs::create_dir_all(root.join("etc")).unwrap();
        let victim = t.path().join("victim");
        std::fs::write(&victim, "keep").unwrap();
        let dest = t.path().join("root.tar");
        symlink(&victim, &dest).unwrap();
        export_root(&root, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert!(std::fs::symlink_metadata(&dest).unwrap().is_file());
    }

    #[test]
    fn the_export_destination_leaves_the_environment() {
        // SAFETY: this test alone touches the variable.
        unsafe { std::env::set_var(III_BUILD_EXPORT_ENV, "/opt/iii/root.tar") };
        assert_eq!(take_export_dest(), Some(PathBuf::from("/opt/iii/root.tar")));
        assert!(std::env::var_os(III_BUILD_EXPORT_ENV).is_none());
        assert_eq!(take_export_dest(), None);
    }
}
