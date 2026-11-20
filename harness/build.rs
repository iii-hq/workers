//! Build script for the `harness` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Exports the commit being built as `env!("GIT_SHA")` (`git_sha` below);
//!    `turn_loop.rs` stamps it on every assistant step.
//! 3. Builds the injected console UI that `src/ui.rs` embeds (`ui_build`).

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rustc-env=GIT_SHA={}", git_sha(&manifest_dir));

    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}

/// Compiled into the binary from outside `harness/`: the path dependencies of
/// Cargo.toml, and the UI package bundled into `ui/dist`.
const OUTSIDE_SOURCES: [&str; 4] = [
    "../crates/judge-contract",
    "../crates/console-ui",
    "../crates/worker-paths",
    "../packages/console-ui",
];

/// The commit this binary is built from: `$GIT_SHA` when the build environment
/// sets it, else `git rev-parse HEAD` of the checkout, suffixed `-dirty` when
/// `harness/` or `OUTSIDE_SOURCES` has uncommitted changes (a local build never
/// claims a clean SHA). `unknown` when neither answers.
fn git_sha(crate_dir: &Path) -> String {
    println!("cargo:rerun-if-env-changed=GIT_SHA");
    if let Some(sha) = std::env::var("GIT_SHA")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return sha;
    }
    // Whatever is compiled in can make the tree dirty (or let a git that
    // failed before answer now), so editing it must refresh the value too.
    for source in ["src", "prompts", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={source}");
    }
    // `packages/console-ui` is not listed (cargo would walk its node_modules):
    // `ui_build` watches its sources, so its edits rerun this script too.
    for source in &OUTSIDE_SOURCES[..3] {
        println!("cargo:rerun-if-changed={source}");
    }
    let Some(head) = git(crate_dir, &["rev-parse", "HEAD"]) else {
        return "unknown".to_string();
    };

    // HEAD moving (commit, checkout, pull) must refresh the value. A worktree's
    // `.git` is a file, so ask git where HEAD and the branch ref live; a
    // missing path would make cargo rerun this script on every build. A commit
    // on a branch that only lives in `packed-refs` changes neither HEAD nor
    // packed-refs, but it always appends to the HEAD reflog.
    let branch = git(crate_dir, &["symbolic-ref", "-q", "HEAD"]);
    let names = ["HEAD", "packed-refs", "logs/HEAD"];
    for name in names.into_iter().chain(branch.as_deref()) {
        if let Some(path) = git(crate_dir, &["rev-parse", "--git-path", name]) {
            let path = crate_dir.join(path);
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    // A failing `git status` counts as dirty: never claim a clean SHA unchecked.
    let mut status = vec!["--no-optional-locks", "status", "--porcelain", "--", "."];
    status.extend(OUTSIDE_SOURCES);
    let dirty = git(crate_dir, &status).is_none_or(|changes| !changes.is_empty());
    if dirty { format!("{head}-dirty") } else { head }
}

/// Trimmed stdout of `git <args>` run in `dir`; `None` when git is missing or
/// exits non-zero (no repository, no commits, unsafe directory).
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
