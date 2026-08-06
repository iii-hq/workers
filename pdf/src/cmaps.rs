//! CJK CMap payload, embedded in this binary.
//!
//! Why this module exists: the parser loads its built-in CJK CMaps from disk at
//! runtime, resolving them against the `CARGO_MANIFEST_DIR` recorded when the
//! parser crate itself was compiled. Consumed as a dependency and cross
//! compiled in CI, that is a path inside the build machine's cargo registry —
//! it does not exist on the machine running a released binary. The lookup then
//! finds nothing and CID fonts that carry no ToUnicode table decode to empty
//! text. Nothing crashes and nothing is logged, so a released worker would
//! quietly return an empty document for a class of Chinese, Japanese and Korean
//! PDFs while passing every test built from source.
//!
//! The fix, in three steps: `build.rs` stages the parser's CMap directory into
//! `OUT_DIR`, this module embeds that directory into the binary, and
//! [`materialize`] writes it to a cache directory on first use and points the
//! parser at it through the `PDF_INSPECTOR_BCMAPS_DIR` environment variable,
//! which the parser checks ahead of its compiled-in path.

use std::path::{Path, PathBuf};

use include_dir::{include_dir, Dir};

/// Staged by `build.rs` from the parser's own `external/bcmaps`.
static CMAPS: Dir<'_> = include_dir!("$OUT_DIR/bcmaps");

/// The environment variable the parser consults before its compiled-in path.
const CMAP_DIR_ENV: &str = "PDF_INSPECTOR_BCMAPS_DIR";

/// Write the embedded CMaps to a cache directory and point the parser at them.
///
/// Best effort by design: a read-only or full disk costs CJK fidelity, which is
/// worth a warning, not a failed boot. An operator who has already set
/// `PDF_INSPECTOR_BCMAPS_DIR` keeps their directory untouched.
pub fn materialize() {
    if let Some(existing) = std::env::var_os(CMAP_DIR_ENV) {
        tracing::info!(
            dir = %Path::new(&existing).display(),
            "{CMAP_DIR_ENV} already set; using the operator's CMap directory"
        );
        return;
    }

    let dir = cache_dir();
    match write_all(&dir) {
        Ok(written) => {
            std::env::set_var(CMAP_DIR_ENV, &dir);
            tracing::info!(
                dir = %dir.display(),
                files = written,
                "CJK CMaps materialized"
            );
        }
        Err(e) => {
            tracing::warn!(
                dir = %dir.display(),
                error = %e,
                "failed to materialize CJK CMaps; CID fonts without a ToUnicode table \
                 will extract as empty text"
            );
        }
    }
}

/// Per-version cache directory, so a worker upgrade cannot serve a previous
/// release's payload out of a warm cache.
fn cache_dir() -> PathBuf {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    root.join("iii")
        .join("pdf")
        .join(format!("bcmaps-{}", env!("CARGO_PKG_VERSION")))
}

/// Write every embedded file that is missing or the wrong size. Returns the
/// number of files written.
fn write_all(dir: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let mut written = 0usize;
    for file in CMAPS.files() {
        let name = file
            .path()
            .file_name()
            .expect("an embedded file has a file name");
        let dest = dir.join(name);
        if dest
            .metadata()
            .is_ok_and(|m| m.len() == file.contents().len() as u64)
        {
            continue;
        }
        std::fs::write(&dest, file.contents())?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the module. A staging regression shows up here rather
    /// than as silently empty CJK text in production.
    #[test]
    fn cmaps_are_embedded() {
        let count = CMAPS.files().count();
        assert!(
            count > 100,
            "expected the parser's full CMap payload, embedded {count} files"
        );
    }

    #[test]
    fn embedded_cmaps_are_nonempty() {
        for file in CMAPS.files() {
            assert!(
                !file.contents().is_empty(),
                "{} staged as an empty file",
                file.path().display()
            );
        }
    }

    #[test]
    fn known_cjk_cmaps_are_present() {
        let names: Vec<String> = CMAPS
            .files()
            .filter_map(|f| {
                f.path()
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
            })
            .collect();
        // One per CJK script: simplified Chinese, Japanese, Korean, traditional
        // Chinese. A partial staging would still pass the count check above.
        for expected in [
            "UniGB-UCS2-H.bcmap",
            "UniJIS-UCS2-H.bcmap",
            "UniKS-UCS2-H.bcmap",
            "UniCNS-UCS2-H.bcmap",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing {expected} — the CMap payload is incomplete"
            );
        }
    }

    #[test]
    fn write_all_is_idempotent() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let first = write_all(tmp.path()).expect("first write");
        assert_eq!(first, CMAPS.files().count());
        let second = write_all(tmp.path()).expect("second write");
        assert_eq!(second, 0, "a warm cache must not be rewritten");
    }

    #[test]
    fn cache_dir_is_version_scoped() {
        let dir = cache_dir();
        assert!(
            dir.ends_with(format!("bcmaps-{}", env!("CARGO_PKG_VERSION"))),
            "cache dir must be version scoped: {}",
            dir.display()
        );
    }
}
