//! The code directory an investigation reads: the user's choice is checked
//! when it is configured, and the code references the LLM cites are checked
//! against it when the investigation completes.
//!
//! No bus function is added: the LLM reads the directory with the existing
//! `coder::*` and shell functions, which the Harness scopes to it through
//! `fs_scope.root`. Everything here is blocking `std::fs`, on the host that
//! runs the monitor.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path};

use crate::contract::{CodeRefV1, MAX_CODE_REFS};

/// The largest file a code reference may point at. The LLM picks the file, so
/// what one citation can make the monitor read is bounded; source files are
/// far below this, and the traces databases and build outputs that live in a
/// workers checkout are far above it.
pub const MAX_REF_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// An absolute path to an existing directory.
pub fn validate_directory(path: &str) -> Result<(), String> {
    if !Path::new(path).is_absolute() {
        return Err("code_repository must be an absolute path".into());
    }
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!("code_repository {path} is not a directory")),
        Err(error) => Err(format!("code_repository {path} is not readable: {error}")),
    }
}

/// Reasons to reject a suggestion's code references: too many, any reference
/// when code access was off, or one that does not point at real lines of a
/// file inside `code_root` (absolute path, `..`, a symlink leading out, a
/// missing file, an impossible range).
///
/// Blocking `std::fs`: each cited file is read only as far as `line_to` and
/// never past [`MAX_REF_FILE_BYTES`], and the caller runs this off the async
/// executor.
// ponytail: a file cited twice is read twice; cache the counts if that ever
// shows up in a profile.
pub fn validate_refs(code_root: Option<&str>, refs: &[CodeRefV1]) -> Vec<String> {
    if refs.is_empty() {
        return Vec::new();
    }
    let Some(root) = code_root else {
        return vec!["code_refs were given but code access was off for this analysis".into()];
    };
    if refs.len() > MAX_CODE_REFS {
        return vec![format!(
            "{} code_refs exceed the limit of {MAX_CODE_REFS}",
            refs.len()
        )];
    }
    let root = match Path::new(root).canonicalize() {
        Ok(root) => root,
        Err(error) => {
            return vec![format!(
                "the code directory {root} is not readable: {error}"
            )]
        }
    };
    refs.iter()
        .filter_map(|reference| check_ref(&root, reference).err())
        .collect()
}

fn check_ref(root: &Path, reference: &CodeRefV1) -> Result<(), String> {
    let label = format!(
        "code ref {}:{}-{}",
        reference.path, reference.line_from, reference.line_to
    );
    let relative = Path::new(&reference.path);
    if relative.components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "{label}: the path must be relative to the code directory, without `..`"
        ));
    }
    // Canonicalizing follows symlinks: a link inside the directory must not
    // lead out of it.
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|_| format!("{label}: no such file in the code directory"))?;
    if !path.starts_with(root) {
        return Err(format!(
            "{label}: the path resolves outside the code directory"
        ));
    }
    let unreadable = |error: std::io::Error| format!("{label}: the file is unreadable: {error}");
    let metadata = std::fs::metadata(&path).map_err(unreadable)?;
    if !metadata.is_file() {
        return Err(format!("{label}: not a file"));
    }
    if reference.line_from < 1 || reference.line_from > reference.line_to {
        return Err(format!(
            "{label}: lines are 1-based and inclusive, with from <= to"
        ));
    }
    if metadata.len() > MAX_REF_FILE_BYTES {
        return Err(format!(
            "{label}: the file is larger than {} MiB, too big to be cited",
            MAX_REF_FILE_BYTES / (1024 * 1024)
        ));
    }
    let wanted = u64::from(reference.line_to);
    let total = lines_up_to(File::open(&path).map_err(unreadable)?, wanted).map_err(unreadable)?;
    if total < wanted {
        return Err(format!(
            "{label}: line_to is past the end of the file, which has {total} lines"
        ));
    }
    Ok(())
}

/// The lines of `source` (a last line without a newline counts), or any
/// number `>= wanted` as soon as that many are known, so a citation near the
/// top of a file never reads the rest of it.
fn lines_up_to(mut source: impl Read, wanted: u64) -> std::io::Result<u64> {
    let mut buffer = [0u8; 64 * 1024];
    let mut newlines = 0u64;
    let mut last = b'\n';
    loop {
        let read = match source.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        newlines += buffer[..read].iter().filter(|byte| **byte == b'\n').count() as u64;
        last = buffer[read - 1];
        if newlines >= wanted {
            return Ok(newlines);
        }
    }
    Ok(newlines + u64::from(last != b'\n'))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use uuid::Uuid;

    use super::*;

    /// A throwaway directory: `a.rs` (3 lines, the last without a newline),
    /// `src/b.rs` (2 lines), `empty.rs` and a sibling `outside/secret.txt`.
    struct Tree {
        base: PathBuf,
    }

    impl Tree {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!("eval-code-{}", Uuid::new_v4().simple()));
            fs::create_dir_all(base.join("root/src")).unwrap();
            fs::create_dir_all(base.join("outside")).unwrap();
            fs::write(base.join("root/a.rs"), "fn a() {}\nfn a2() {}\nfn a3() {}").unwrap();
            fs::write(base.join("root/src/b.rs"), "fn b() {}\n// two\n").unwrap();
            fs::write(base.join("root/empty.rs"), "").unwrap();
            fs::write(base.join("outside/secret.txt"), "one\ntwo\n").unwrap();
            Self { base }
        }

        fn root(&self) -> String {
            self.base.join("root").to_string_lossy().into_owned()
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn reference(path: &str, from: u32, to: u32) -> CodeRefV1 {
        CodeRefV1 {
            path: path.into(),
            line_from: from,
            line_to: to,
        }
    }

    /// The single reason a lone reference is rejected for.
    fn rejected(root: &str, reference: CodeRefV1) -> String {
        let reasons = validate_refs(Some(root), &[reference]);
        assert_eq!(reasons.len(), 1, "{reasons:?}");
        reasons[0].clone()
    }

    #[test]
    fn the_directory_must_be_an_absolute_path_to_an_existing_directory() {
        let tree = Tree::new();
        assert_eq!(validate_directory(&tree.root()), Ok(()));
        assert_eq!(validate_directory("/"), Ok(()));
        let relative = validate_directory("workers").unwrap_err();
        assert!(relative.contains("absolute path"), "{relative}");
        let missing = validate_directory(&format!("{}/nope", tree.root())).unwrap_err();
        assert!(missing.contains("not readable"), "{missing}");
        let file = validate_directory(&format!("{}/a.rs", tree.root())).unwrap_err();
        assert!(file.contains("not a directory"), "{file}");
    }

    #[test]
    fn references_to_real_lines_inside_the_directory_pass() {
        let tree = Tree::new();
        let root = tree.root();
        assert!(validate_refs(
            Some(&root),
            &[
                reference("a.rs", 1, 3),
                reference("src/b.rs", 2, 2),
                reference("./src/b.rs", 1, 2),
            ]
        )
        .is_empty());
        assert!(validate_refs(Some(&root), &[]).is_empty());
        assert!(validate_refs(None, &[]).is_empty());
        let limit: Vec<_> = (0..MAX_CODE_REFS)
            .map(|_| reference("a.rs", 1, 1))
            .collect();
        assert!(validate_refs(Some(&root), &limit).is_empty());
    }

    #[test]
    fn references_that_do_not_resolve_to_a_file_are_rejected_with_a_reason() {
        let tree = Tree::new();
        let root = tree.root();
        assert!(rejected(&root, reference("missing.rs", 1, 1)).contains("no such file"));
        assert!(rejected(&root, reference("src", 1, 1)).contains("not a file"));
        assert!(rejected(&root, reference("", 1, 1)).contains("not a file"));
        assert!(rejected(&root, reference("../outside/secret.txt", 1, 1)).contains("`..`"));
        assert!(rejected(&root, reference("src/../a.rs", 1, 1)).contains("`..`"));
        assert!(rejected(&root, reference("src/../../outside/secret.txt", 1, 1)).contains("`..`"));
        let secret = format!("{}/outside/secret.txt", tree.base.display());
        assert!(rejected(&root, reference(&secret, 1, 1)).contains("relative"));
        assert!(rejected(&root, reference("/etc/hostname", 1, 1)).contains("relative"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_may_not_lead_out_of_the_directory() {
        use std::os::unix::fs::symlink;

        let tree = Tree::new();
        let root = tree.root();
        symlink(
            tree.base.join("outside/secret.txt"),
            tree.base.join("root/leak.txt"),
        )
        .unwrap();
        symlink(tree.base.join("outside"), tree.base.join("root/src/out")).unwrap();
        symlink("a.rs", tree.base.join("root/alias.rs")).unwrap();
        assert!(rejected(&root, reference("leak.txt", 1, 2)).contains("outside the code directory"));
        assert!(rejected(&root, reference("src/out/secret.txt", 1, 2))
            .contains("outside the code directory"));
        // A link that stays inside the directory is just another name.
        assert!(validate_refs(Some(&root), &[reference("alias.rs", 1, 3)]).is_empty());
        // A directory reached through a link is judged by where it resolves.
        let through_root_link = tree.base.join("link-to-root");
        symlink(&root, &through_root_link).unwrap();
        let through_root_link = through_root_link.to_string_lossy().into_owned();
        assert!(validate_refs(Some(&through_root_link), &[reference("a.rs", 1, 1)]).is_empty());
    }

    #[test]
    fn impossible_ranges_are_rejected() {
        let tree = Tree::new();
        let root = tree.root();
        assert!(rejected(&root, reference("a.rs", 0, 2)).contains("1-based"));
        assert!(rejected(&root, reference("a.rs", 3, 2)).contains("from <= to"));
        assert!(rejected(&root, reference("a.rs", 1, 4)).contains("has 3 lines"));
        assert!(rejected(&root, reference("src/b.rs", 1, 3)).contains("has 2 lines"));
        assert!(rejected(&root, reference("empty.rs", 1, 1)).contains("has 0 lines"));
        assert!(rejected(&root, reference("a.rs", 1, u32::MAX)).contains("has 3 lines"));
    }

    #[test]
    fn a_file_above_the_size_limit_is_rejected_without_being_read() {
        let tree = Tree::new();
        let root = tree.root();
        // Sparse: nothing is written, so a read of it would be the only cost.
        let big = fs::File::create(tree.base.join("root/big.sqlite3")).unwrap();
        big.set_len(MAX_REF_FILE_BYTES + 1).unwrap();
        assert!(rejected(&root, reference("big.sqlite3", 1, 1)).contains("larger than 16 MiB"));
        let limit = fs::File::create(tree.base.join("root/limit.log")).unwrap();
        limit.set_len(MAX_REF_FILE_BYTES).unwrap();
        // At the limit it is a file like any other: all zeros, one unterminated line.
        assert!(validate_refs(Some(&root), &[reference("limit.log", 1, 1)]).is_empty());
        assert!(rejected(&root, reference("limit.log", 1, 2)).contains("has 1 lines"));
    }

    #[test]
    fn lines_are_counted_like_an_editor_and_only_as_far_as_wanted() {
        let count = |text: &str| lines_up_to(text.as_bytes(), u64::MAX).unwrap();
        assert_eq!(count(""), 0);
        assert_eq!(count("a"), 1);
        assert_eq!(count("a\n"), 1);
        assert_eq!(count("a\nb"), 2);
        assert_eq!(count("\n\n"), 2);
        // A source that never ends is fine as long as the wanted line is near.
        struct Endless;
        impl Read for Endless {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                buffer.fill(b'\n');
                Ok(buffer.len())
            }
        }
        assert!(lines_up_to(Endless, 3).unwrap() >= 3);
        // An interrupted read is retried, a failing one is reported.
        struct Flaky(u8);
        impl Read for Flaky {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.0 += 1;
                match self.0 {
                    1 => Err(ErrorKind::Interrupted.into()),
                    2 => {
                        buffer[0] = b'x';
                        Ok(1)
                    }
                    _ => Err(ErrorKind::PermissionDenied.into()),
                }
            }
        }
        assert_eq!(
            lines_up_to(Flaky(0), 1).unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn references_need_code_access_and_stay_within_the_limit() {
        let tree = Tree::new();
        let root = tree.root();
        let off = validate_refs(None, &[reference("a.rs", 1, 1)]);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("code access was off"), "{off:?}");
        let many: Vec<_> = (0..=MAX_CODE_REFS)
            .map(|_| reference("a.rs", 1, 1))
            .collect();
        let over = validate_refs(Some(&root), &many);
        assert_eq!(over.len(), 1);
        assert!(over[0].contains("exceed the limit"), "{over:?}");
        // Every bad reference is reported, the good ones are not.
        let mixed = validate_refs(
            Some(&root),
            &[
                reference("a.rs", 1, 1),
                reference("nope.rs", 1, 1),
                reference("a.rs", 9, 9),
            ],
        );
        assert_eq!(mixed.len(), 2, "{mixed:?}");
        let gone = validate_refs(
            Some(&format!("{root}/vanished")),
            &[reference("a.rs", 1, 1)],
        );
        assert!(gone[0].contains("not readable"), "{gone:?}");
    }
}
