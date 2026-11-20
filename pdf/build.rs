//! Build script for the `pdf` worker.
//!
//! Three jobs:
//!
//! 1. Forward the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Stage the parser's CJK CMap payload into `OUT_DIR` so `src/cmaps.rs` can
//!    embed it with `include_dir!`. The parser resolves those files at runtime
//!    relative to its OWN `CARGO_MANIFEST_DIR`, which for a dependency is a
//!    path inside the build machine's cargo registry — a directory that does
//!    not exist on the machine running a released binary. Without this staging
//!    step CID fonts with no ToUnicode table silently decode to nothing.
//! 3. Build the injected console UI that `src/ui.rs` embeds (`ui_build`).

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    stage_cmaps();
    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}

// ---------------------------------------------------------------------------
// CMaps
// ---------------------------------------------------------------------------

/// Copy `pdf-inspector`'s `external/bcmaps` into `$OUT_DIR/bcmaps`.
///
/// The source directory is resolved through `cargo metadata` rather than by
/// guessing at a registry path, so a vendored, patched, or path-overridden
/// dependency stages the right files.
fn stage_cmaps() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let dest = out_dir.join("bcmaps");

    let src = locate_dependency_cmaps();
    println!("cargo:rerun-if-changed={}", src.display());

    if dest.is_dir() && dir_file_count(&dest) == dir_file_count(&src) {
        return;
    }

    if dest.exists() {
        std::fs::remove_dir_all(&dest)
            .unwrap_or_else(|e| panic!("failed to clear {}: {e}", dest.display()));
    }
    std::fs::create_dir_all(&dest)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", dest.display()));

    let entries =
        std::fs::read_dir(&src).unwrap_or_else(|e| panic!("failed to read {}: {e}", src.display()));
    let mut copied = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().expect("directory entry has a file name");
        std::fs::copy(&path, dest.join(name))
            .unwrap_or_else(|e| panic!("failed to copy {}: {e}", path.display()));
        copied += 1;
    }

    assert!(
        copied > 0,
        "staged zero CMap files from {} — the parser dependency layout changed",
        src.display()
    );
}

/// `<pdf-inspector manifest dir>/external/bcmaps`, via `cargo metadata`.
fn locate_dependency_cmaps() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1"])
        .output()
        .expect("failed to run `cargo metadata`");
    assert!(
        output.status.success(),
        "`cargo metadata` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata emits json");
    let manifest = metadata["packages"]
        .as_array()
        .expect("metadata carries a packages array")
        .iter()
        .find(|p| p["name"] == "pdf-inspector")
        .and_then(|p| p["manifest_path"].as_str())
        .unwrap_or_else(|| {
            panic!("`pdf-inspector` is not in the dependency graph — cannot stage its CMaps")
        });

    let dir = Path::new(manifest)
        .parent()
        .expect("a manifest path has a parent")
        .join("external")
        .join("bcmaps");
    assert!(
        dir.is_dir(),
        "expected the parser's CMap directory at {} — the dependency layout changed",
        dir.display()
    );
    dir
}

fn dir_file_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().filter(|e| e.path().is_file()).count())
        .unwrap_or(0)
}
