//! Generates `EXPECTED_WORKERS` from `iii.worker.yaml` so the harness has a
//! single source of truth for the runtime worker list.
//!
//! `iii.worker.yaml` is the manifest the iii engine reads to decide which
//! workers to spawn. The Rust side needs the same list at runtime to feed
//! `fanout::diff_workers` (so the workers strip in the UI can flag
//! down/missing entries), advertise it via `harness::status`, and gate the
//! first-boot bootstrap in `harness::skills::bootstrap_run`.
//!
//! Parsing is line-based on purpose: it avoids pulling `serde_yaml` into
//! build-deps and keeps the build script self-contained.

use std::{env, fs, path::PathBuf};

fn main() {
    // Forward $TARGET so `src/manifest.rs` can use `env!("TARGET")`. Cargo
    // sets it for build scripts but not for normal compilation.
    println!(
        "cargo:rustc-env=TARGET={}",
        env::var("TARGET").expect("TARGET set by cargo for build scripts")
    );

    println!("cargo:rerun-if-changed=iii.worker.yaml");
    println!("cargo:rerun-if-changed=build.rs");

    let yaml = fs::read_to_string("iii.worker.yaml")
        .expect("harness/iii.worker.yaml must exist next to Cargo.toml");

    let workers: Vec<String> = yaml
        .lines()
        .skip_while(|l| !l.starts_with("dependencies:"))
        .skip(1)
        .take_while(|l| l.is_empty() || l.starts_with(' ') || l.starts_with('\t'))
        .filter(|l| l.starts_with("  ") && l.contains(':'))
        .filter_map(|l| l.split(':').next())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();

    assert!(
        !workers.is_empty(),
        "no entries parsed from iii.worker.yaml `dependencies:` block"
    );

    let body = workers
        .iter()
        .map(|w| format!("    {:?},", w))
        .collect::<Vec<_>>()
        .join("\n");

    let generated = format!(
        "/// Runtime workers the harness assumes are on the iii bus.\n\
         ///\n\
         /// Generated at build time from `harness/iii.worker.yaml` by\n\
         /// `harness/build.rs` so the Rust side and the engine-facing\n\
         /// manifest cannot drift.\n\
         pub const EXPECTED_WORKERS: &[&str] = &[\n{body}\n];\n"
    );

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let dest: PathBuf = [out_dir.as_str(), "expected_workers.rs"].iter().collect();
    fs::write(&dest, generated).expect("write generated expected_workers.rs");
}
