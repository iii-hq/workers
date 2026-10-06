//! Process-level write confinement for a scoped host exec
//! (`fs.exec_confinement: landlock`).

use std::path::{Path, PathBuf};

/// The git dir a commit in `root` writes to: the common dir of a linked
/// worktree, or the `.git` directory of the repository enclosing `root`.
pub fn repo_git_dir(root: &Path) -> Option<PathBuf> {
    for dir in root.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return dot_git.canonicalize().ok();
        }
        if dot_git.is_file() {
            // A linked worktree (or submodule): `gitdir: <path>`, and the
            // worktree's gitdir names the shared repo in `commondir`.
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = dir.join(text.strip_prefix("gitdir:")?.trim());
            let common = std::fs::read_to_string(gitdir.join("commondir"))
                .map(|c| gitdir.join(c.trim()))
                .unwrap_or(gitdir);
            return common.canonicalize().ok();
        }
    }
    None
}

/// Every path a confined exec may write under: the (canonical) scope root and
/// grants, the root's git dir, and the operator's `fs.exec_writable` entries.
/// A `~/` entry expands against `home` (dropped without one); an entry that
/// does not exist is skipped.
pub fn writable_roots(
    scope_root: &Path,
    grants: &[PathBuf],
    extra: &[String],
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = vec![scope_root.to_path_buf()];
    roots.extend(grants.iter().cloned());
    roots.extend(repo_git_dir(scope_root));
    for entry in extra {
        let path = match (entry.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest),
            (Some(_), None) => continue,
            (None, _) => PathBuf::from(entry),
        };
        if let Ok(canon) = path.canonicalize() {
            roots.push(canon);
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

/// S-code of a scoped exec refused because its confinement cannot be applied.
pub const UNAVAILABLE_CODE: &str = "S222";

/// The fail-closed refusal. It leads with its S-code because `exec_bg`
/// returns spawn failures as plain strings; `shell::exec` lifts the code back
/// out (see `host::host_exec_error`).
pub fn unavailable(reason: impl std::fmt::Display) -> String {
    format!(
        "{UNAVAILABLE_CODE}: exec confinement unavailable: {reason}; \
         set fs.exec_confinement: off to run unconfined"
    )
}

/// Confine `cmd`'s writes to `write_roots`. The ruleset is built here, in the
/// parent, so every failure is an error before anything runs.
#[cfg(target_os = "linux")]
pub fn apply(cmd: &mut tokio::process::Command, write_roots: &[PathBuf]) -> Result<(), String> {
    use std::os::fd::AsFd;
    let ruleset = landlock::ruleset(write_roots)?;
    // SAFETY: the hook runs in the forked child before exec and only makes
    // raw syscalls, which are async-signal-safe.
    unsafe {
        cmd.pre_exec(move || landlock::restrict_self(ruleset.as_fd()));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn apply(_cmd: &mut tokio::process::Command, _write_roots: &[PathBuf]) -> Result<(), String> {
    Err(unavailable("Landlock needs Linux"))
}

/// Landlock through raw syscalls (no crate): write rights only, so reads stay
/// open. See `Documentation/userspace-api/landlock.rst` for the constants.
#[cfg(target_os = "linux")]
pub(crate) mod landlock {
    use std::fs::OpenOptions;
    use std::io;
    use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::PathBuf;

    use super::unavailable;

    const ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
    const ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
    const ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
    const ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
    const ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
    const ACCESS_FS_MAKE_REG: u64 = 1 << 8;
    const ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
    const ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
    const ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
    const ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
    /// ABI 2: link or rename into another directory.
    pub(crate) const ACCESS_FS_REFER: u64 = 1 << 13;
    /// ABI 3: truncate(2) and `O_TRUNC`.
    pub(crate) const ACCESS_FS_TRUNCATE: u64 = 1 << 14;
    /// The rights a rule on a non-directory may carry.
    const FILE_RIGHTS: u64 = ACCESS_FS_WRITE_FILE | ACCESS_FS_TRUNCATE;
    const CREATE_RULESET_VERSION: u32 = 1;
    const RULE_PATH_BENEATH: libc::c_int = 1;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C, packed)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    /// The write rights this kernel's ABI knows. A right from a later ABI
    /// would make the ruleset fail, so it is left out.
    pub(crate) fn handled_write_access(abi: libc::c_long) -> u64 {
        let mut rights = ACCESS_FS_WRITE_FILE
            | ACCESS_FS_REMOVE_DIR
            | ACCESS_FS_REMOVE_FILE
            | ACCESS_FS_MAKE_CHAR
            | ACCESS_FS_MAKE_DIR
            | ACCESS_FS_MAKE_REG
            | ACCESS_FS_MAKE_SOCK
            | ACCESS_FS_MAKE_FIFO
            | ACCESS_FS_MAKE_BLOCK
            | ACCESS_FS_MAKE_SYM;
        if abi >= 2 {
            rights |= ACCESS_FS_REFER;
        }
        if abi >= 3 {
            rights |= ACCESS_FS_TRUNCATE;
        }
        rights
    }

    /// The kernel's Landlock ABI version: ENOSYS without Landlock, EOPNOTSUPP
    /// when it is built but disabled at boot.
    pub(crate) fn abi() -> io::Result<libc::c_long> {
        // SAFETY: the version query reads no memory (null attr, size 0).
        let v = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                CREATE_RULESET_VERSION,
            )
        };
        if v <= 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(v)
    }

    pub(crate) fn ruleset(write_roots: &[PathBuf]) -> Result<OwnedFd, String> {
        ruleset_for(abi(), write_roots)
    }

    /// Build the ruleset for `abi` (split from [`ruleset`] so the unavailable
    /// branch is testable on a kernel that has Landlock).
    pub(crate) fn ruleset_for(
        abi: io::Result<libc::c_long>,
        write_roots: &[PathBuf],
    ) -> Result<OwnedFd, String> {
        let abi = abi.map_err(|e| unavailable(format!("Landlock: {e}")))?;
        let handled = handled_write_access(abi);
        let attr = RulesetAttr {
            handled_access_fs: handled,
        };
        // SAFETY: `attr` outlives the call and its size is passed with it.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            )
        };
        if fd < 0 {
            return Err(unavailable(format!(
                "Landlock ruleset: {}",
                io::Error::last_os_error()
            )));
        }
        // SAFETY: the syscall returned a fresh close-on-exec fd we now own.
        let ruleset = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        for root in write_roots {
            // A root that cannot be opened has nothing to allow beneath it;
            // skipping it only denies more.
            let Ok(file) = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH)
                .open(root)
            else {
                continue;
            };
            let is_dir = file.metadata().map(|m| m.is_dir()).unwrap_or(false);
            let rule = PathBeneathAttr {
                allowed_access: if is_dir {
                    handled
                } else {
                    handled & FILE_RIGHTS
                },
                parent_fd: file.as_raw_fd(),
            };
            // SAFETY: `rule` and both fds are live for the call.
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_landlock_add_rule,
                    ruleset.as_raw_fd(),
                    RULE_PATH_BENEATH,
                    &rule as *const PathBeneathAttr,
                    0u32,
                )
            };
            if rc != 0 {
                return Err(unavailable(format!(
                    "Landlock rule for {}: {}",
                    root.display(),
                    io::Error::last_os_error()
                )));
            }
        }
        Ok(ruleset)
    }

    /// Enforce `ruleset` on the calling process. Landlock requires
    /// `no_new_privs`, so setuid binaries lose their privilege in a confined
    /// exec. Async-signal-safe: raw syscalls only.
    pub(crate) fn restrict_self(ruleset: BorrowedFd) -> io::Result<()> {
        // SAFETY: plain prctl/syscall with integer arguments.
        unsafe {
            if libc::prctl(
                libc::PR_SET_NO_NEW_PRIVS,
                1 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }
            if libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0u32) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn git_dir_of_a_linked_worktree_is_the_common_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let main_git = tmp.path().join("main/.git");
        let wt_git = main_git.join("worktrees/wt");
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        std::fs::create_dir_all(tmp.path().join("wt/sub")).unwrap();
        std::fs::write(
            tmp.path().join("wt/.git"),
            format!("gitdir: {}\n", wt_git.display()),
        )
        .unwrap();
        assert_eq!(
            repo_git_dir(&tmp.path().join("wt/sub")),
            Some(main_git.canonicalize().unwrap())
        );
    }

    #[test]
    fn git_dir_of_a_subfolder_root_is_the_enclosing_dot_git() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("r/.git")).unwrap();
        std::fs::create_dir_all(tmp.path().join("r/a")).unwrap();
        assert_eq!(
            repo_git_dir(&tmp.path().join("r/a")),
            Some(tmp.path().join("r/.git").canonicalize().unwrap())
        );
    }

    #[test]
    fn writable_roots_expand_home_and_skip_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let grant = tmp.path().join("grant");
        let home = tmp.path().join("home");
        for dir in [&root, &grant, &home.join(".cache")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let extra: Vec<String> = ["/tmp", "~/.cache", "~/.missing", "/nope-xyz-5167"]
            .map(String::from)
            .into();
        let roots = writable_roots(&root, std::slice::from_ref(&grant), &extra, Some(&home));
        for want in [
            root.clone(),
            grant.clone(),
            PathBuf::from("/tmp").canonicalize().unwrap(),
            home.join(".cache").canonicalize().unwrap(),
        ] {
            assert!(roots.contains(&want), "{want:?} missing from {roots:?}");
        }
        assert!(!roots.iter().any(|r| r.ends_with(".missing")), "{roots:?}");
        assert!(!roots.contains(&PathBuf::from("/nope-xyz-5167")));
        // No HOME: `~/` entries are dropped, not resolved against `/`.
        let roots = writable_roots(&root, &[], &extra, None);
        assert!(!roots.iter().any(|r| r.ends_with(".cache")), "{roots:?}");
    }

    /// An older kernel must get a ruleset it understands: rights from a later
    /// ABI are dropped, not sent (which would fail the ruleset).
    #[cfg(target_os = "linux")]
    #[test]
    fn handled_rights_follow_the_abi() {
        use landlock::{handled_write_access, ACCESS_FS_REFER, ACCESS_FS_TRUNCATE};
        assert_eq!(handled_write_access(1) & ACCESS_FS_REFER, 0);
        assert_eq!(handled_write_access(1) & ACCESS_FS_TRUNCATE, 0);
        assert_ne!(handled_write_access(2) & ACCESS_FS_REFER, 0);
        assert_eq!(handled_write_access(2) & ACCESS_FS_TRUNCATE, 0);
        assert_ne!(handled_write_access(3) & ACCESS_FS_TRUNCATE, 0);
        assert_eq!(handled_write_access(6), handled_write_access(3));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unavailable_landlock_fails_closed() {
        let enosys = std::io::Error::from_raw_os_error(libc::ENOSYS);
        let err = landlock::ruleset_for(Err(enosys), &[]).expect_err("no ruleset, no exec");
        assert!(
            err.starts_with("S222: exec confinement unavailable"),
            "{err}"
        );
        assert!(err.contains("fs.exec_confinement: off"), "{err}");
    }
}
