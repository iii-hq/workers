// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! `iii-loadtest rootfs`: the smallest guest root that runs host binaries.
//!
//! The engine and this tool are built for the host, so the guest gets the
//! host's own shared libraries: every binary is copied with what `ldd` says
//! it loads, at the same paths. Add `/bin/sh` (iii-init runs the command
//! through it) and iii-init itself as `/init.krun`, and nothing else: no
//! distribution image, no package manager, no services competing with the
//! engine for the VM.
//!
//! iii-init boots on a tmpfs and binds only the usual top-level directories
//! of the rootfs into it (`bin`, `etc`, `usr`, `opt`, …), so the binaries go
//! to `/opt/bench/bin`.

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

#[derive(clap::Args, Debug, Clone)]
pub struct RootfsArgs {
    /// Directory to create (or replace, when an earlier `rootfs` built it).
    #[arg(long)]
    pub out: PathBuf,

    /// iii-init, the guest's PID 1 (static; sandbox-libkrun/scripts/build-init.sh builds it).
    #[arg(long)]
    pub init: Option<PathBuf>,

    /// Binary to copy into `/opt/bench/bin`, with its libraries. Repeatable.
    #[arg(long = "bin")]
    pub bins: Vec<PathBuf>,
}

/// Where the guest finds the binaries, relative to `/`.
pub const BIN_DIR: &str = "opt/bench/bin";

pub fn build(args: &RootfsArgs) -> Result<()> {
    let init = args
        .init
        .clone()
        .context("no iii-init: pass --init (sandbox-libkrun/scripts/build-init.sh builds it)")?;
    // Built in a fresh `<out>.new`, then renamed over `out`, never refreshed
    // in place: a VM serves `out` writable, and writing through a symlink its
    // guest left there (at `etc/passwd`, or as a parent directory) would
    // land outside it.
    let name = args
        .out
        .file_name()
        .with_context(|| format!("{} has no file name", args.out.display()))?;
    let mut staged = name.to_owned();
    staged.push(".new");
    let out = &args.out.with_file_name(staged);
    // Both are deleted below: never a directory this tool did not build.
    for dir in [&args.out, out] {
        if !ours(dir) {
            bail!(
                "{} is not empty and holds no rootfs this tool built: pick another --out",
                dir.display()
            );
        }
    }
    // Left by a build that failed halfway.
    remove_tree(out)?;

    // BIN_DIR first: it marks the tree as ours even if the build then fails.
    for dir in [
        BIN_DIR, "bin", "dev", "etc", "proc", "run", "sys", "tmp", "root",
    ] {
        std::fs::create_dir_all(out.join(dir))?;
    }
    std::fs::set_permissions(out.join("tmp"), std::fs::Permissions::from_mode(0o1777))?;

    copy_file(&init, &out.join("init.krun"))?;

    let shell = std::fs::canonicalize("/bin/sh").context("resolve /bin/sh")?;
    copy_file(&shell, &out.join("bin/sh"))?;
    let mut libs = libraries(&shell)?;

    for bin in &args.bins {
        let name = bin
            .file_name()
            .with_context(|| format!("{} has no file name", bin.display()))?;
        copy_file(bin, &out.join(BIN_DIR).join(name))?;
        libs.extend(libraries(bin)?);
    }
    for lib in libs {
        let real = std::fs::canonicalize(&lib).with_context(|| format!("resolve {lib}"))?;
        copy_file(&real, &out.join(lib.trim_start_matches('/')))?;
    }

    std::fs::write(out.join("etc/passwd"), "root:x:0:0:root:/root:/bin/sh\n")?;
    std::fs::write(out.join("etc/group"), "root:x:0:\n")?;
    std::fs::write(out.join("etc/hosts"), "127.0.0.1 localhost\n")?;

    remove_tree(&args.out)?;
    std::fs::rename(out, &args.out)
        .with_context(|| format!("move {} to {}", out.display(), args.out.display()))
}

/// Whether `dir` is absent, empty or built by this tool: only it makes
/// `BIN_DIR` (sandbox-libkrun's worker rootfses also hold an `init.krun`).
fn ours(dir: &Path) -> bool {
    std::fs::read_dir(dir).map_or(true, |mut d| d.next().is_none())
        || dir
            .join(BIN_DIR)
            .symlink_metadata()
            .is_ok_and(|m| m.is_dir())
}

/// Removes `dir` and everything in it, following no symlink; absent is fine.
fn remove_tree(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("remove {}", dir.display()))
        }
        _ => Ok(()),
    }
}

/// The absolute paths `ldd` lists for `bin`, the loader included.
fn libraries(bin: &Path) -> Result<BTreeSet<String>> {
    let output = Command::new("ldd")
        .arg(bin)
        .output()
        .context("run ldd (from the C library's tools)")?;
    let text = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        // A static binary: ldd says so and exits 1.
        if text.contains("not a dynamic executable") || text.contains("statically linked") {
            return Ok(BTreeSet::new());
        }
        bail!(
            "ldd {} failed: {}",
            bin.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(parse_ldd(&text))
}

fn parse_ldd(text: &str) -> BTreeSet<String> {
    let mut libs = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.contains("not found") {
            continue;
        }
        let path = match line.split_once("=>") {
            Some((_, rest)) => rest.split_whitespace().next(),
            None => line.split_whitespace().next(),
        };
        if let Some(path) = path.filter(|p| p.starts_with('/')) {
            libs.insert(path.to_string());
        }
    }
    libs
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to)
        .with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
    let mode = std::fs::metadata(from)?.permissions().mode();
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(mode | 0o500))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rebuild over a rootfs a guest wrote to follows none of its symlinks.
    #[test]
    fn a_rebuild_writes_through_no_symlink_the_guest_left() {
        let dir = std::env::temp_dir().join(format!("iii-loadtest-rootfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = dir.join("rootfs");
        let outside = dir.join("outside");
        std::fs::create_dir_all(out.join("etc")).unwrap();
        std::fs::create_dir_all(out.join(BIN_DIR)).unwrap();
        std::fs::write(out.join("init.krun"), "an earlier build").unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("passwd"), "host\n").unwrap();
        std::os::unix::fs::symlink(outside.join("passwd"), out.join("etc/passwd")).unwrap();
        std::os::unix::fs::symlink(&outside, out.join("bin")).unwrap();
        let init = dir.join("init");
        std::fs::write(&init, "init").unwrap();

        build(&RootfsArgs {
            out: out.clone(),
            init: Some(init),
            bins: Vec::new(),
        })
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(outside.join("passwd")).unwrap(),
            "host\n"
        );
        assert!(!outside.join("sh").exists());
        assert!(!out.join("etc/passwd").is_symlink());
        assert!(out.join("bin/sh").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Neither `out` nor `<out>.new` is replaced when this tool did not build
    /// it, even when it is a rootfs (a worker's, with its `init.krun`).
    #[test]
    fn a_build_leaves_a_directory_it_did_not_build() {
        let dir = std::env::temp_dir().join(format!("iii-loadtest-other-{}", std::process::id()));
        let init = dir.join("init");
        for other in ["rootfs", "rootfs.new"] {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join(other)).unwrap();
            std::fs::write(dir.join(other).join("init.krun"), "mine").unwrap();
            std::fs::write(&init, "init").unwrap();

            let built = build(&RootfsArgs {
                out: dir.join("rootfs"),
                init: Some(init.clone()),
                bins: Vec::new(),
            });

            assert!(built.is_err(), "{other}");
            let kept = std::fs::read_to_string(dir.join(other).join("init.krun"));
            assert_eq!(kept.unwrap(), "mine", "{other}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ldd_output_gives_every_absolute_path() {
        let text = "\tlinux-vdso.so.1 (0x00007ffd)\n\
            \tlibgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x00007f)\n\
            \tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f)\n\
            \t/lib64/ld-linux-x86-64.so.2 (0x00007f)\n\
            \tlibmissing.so => not found\n";
        let libs: Vec<String> = parse_ldd(text).into_iter().collect();
        assert_eq!(
            libs,
            vec![
                "/lib/x86_64-linux-gnu/libc.so.6",
                "/lib/x86_64-linux-gnu/libgcc_s.so.1",
                "/lib64/ld-linux-x86-64.so.2",
            ]
        );
    }
}
