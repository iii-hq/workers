//! Build-script helper that builds a worker's frontend before `include_str!`
//! embeds its output. Std only: each UI-building `build.rs` compiles this
//! file directly, without a build-dependency,
//!
//! ```rust,ignore
//! #[path = "../crates/console-ui/src/ui_build.rs"]
//! mod ui_build;
//!
//! fn main() {
//!     ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
//! }
//! ```
//!
//! The assets are rebuilt with `pnpm install && pnpm build` when one is
//! missing or older than any input: the project's top-level files and `src/`
//! tree, the same for each bundled workspace package, the root
//! `pnpm-lock.yaml` and any extra input. A failed build deletes the assets, so
//! a retry never accepts what a failed check left behind.

#![allow(dead_code)] // also compiled into build scripts that use only part of it

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Frontend<'a> {
    /// The pnpm project, relative to the crate: "ui", "native-ui", "web".
    pub dir: &'a str,
    /// The embedded outputs, relative to `dir`.
    pub assets: &'a [&'a str],
    /// Bundled workspace packages, relative to the repo root.
    pub packages: &'a [&'a str],
    /// Other inputs, relative to the crate; a directory is walked.
    pub extra: &'a [&'a str],
    /// Set and non-empty: use the existing assets as they are.
    pub skip_env: &'a str,
}

/// `ui/`, bundling packages/console-ui, skipped by `SKIP_UI_BUILD`.
pub fn ensure_console_ui(assets: &[&str]) {
    ensure_built(&Frontend {
        dir: "ui",
        assets,
        packages: &["packages/console-ui"],
        extra: &[],
        skip_env: "SKIP_UI_BUILD",
    });
}

pub fn ensure_built(frontend: &Frontend) {
    let crate_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let dir = crate_dir.join(frontend.dir);
    let assets: Vec<PathBuf> = frontend.assets.iter().map(|a| dir.join(a)).collect();
    let inputs = inputs(&crate_dir, frontend);
    for line in directives(&inputs.watched, frontend.skip_env) {
        println!("{line}");
    }

    if assets.iter().all(|a| is_fresh(a, &inputs.files)) {
        return;
    }
    if skip_set(std::env::var(frontend.skip_env).ok().as_deref()) {
        if let Some(missing) = assets.iter().find(|a| !a.exists()) {
            panic!(
                "{} is set but {} is missing: build it (cd {} && pnpm install && pnpm build) \
                 or unset {}",
                frontend.skip_env,
                missing.display(),
                dir.display(),
                frontend.skip_env
            );
        }
        return;
    }
    if let Err(e) = run_build(&locate_pnpm(frontend.skip_env), &dir, &assets) {
        panic!("{e}");
    }
}

/// What the assets are built from. `src` is the only directory entered in a
/// project, so `node_modules`, `dist` and `.tailwind` are never walked.
struct Inputs {
    /// Every input file, for the freshness check.
    files: Vec<PathBuf>,
    /// The same inputs for cargo, with each walked directory (a `src/`, an
    /// `extra` directory) in place of its files: cargo scans a directory
    /// recursively, so adding or removing a file there reruns the script.
    watched: Vec<PathBuf>,
}

fn inputs(crate_dir: &Path, frontend: &Frontend) -> Inputs {
    let root = crate_dir.join("..");
    let lock = root.join("pnpm-lock.yaml");
    let mut out = Inputs {
        files: vec![lock.clone()],
        watched: vec![lock],
    };
    project_files(&crate_dir.join(frontend.dir), &mut out);
    for package in frontend.packages {
        project_files(&root.join(package), &mut out);
    }
    for extra in frontend.extra {
        let path = crate_dir.join(extra);
        if path.is_dir() {
            walk(&path, &mut out.files);
        } else {
            out.files.push(path.clone());
        }
        out.watched.push(path);
    }
    out
}

fn project_files(dir: &Path, out: &mut Inputs) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        // tsc -b writes *.tsbuildinfo next to the sources: output, not input.
        if hidden(&name) || path.extension() == Some(OsStr::new("tsbuildinfo")) {
            continue;
        }
        if name == "src" && path.is_dir() {
            walk(&path, &mut out.files);
            out.watched.push(path);
        } else if path.is_file() {
            out.files.push(path.clone());
            out.watched.push(path);
        }
    }
}

/// The `cargo:` lines for `watched`. The assets are not listed: include_str!
/// tracks them, and listing them would rerun the script on its own output.
/// A missing path is left out: cargo counts it as always changed and would
/// rerun the script on every build.
/// shortcut: a missing input created later (ade's gitignored
/// web/.env.production) only counts once another input reruns the script;
/// watch its parent instead if that ever matters, unless that parent is a
/// project root holding node_modules or dist.
fn directives(watched: &[PathBuf], skip_env: &str) -> Vec<String> {
    let mut lines: Vec<String> = watched
        .iter()
        .filter(|path| path.exists())
        .map(|path| format!("cargo:rerun-if-changed={}", path.display()))
        .collect();
    lines.push(format!("cargo:rerun-if-env-changed={skip_env}"));
    lines.push("cargo:rerun-if-env-changed=PNPM".to_string());
    lines
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if hidden(&entry.file_name()) {
            continue;
        }
        // file_type does not follow symlinks, so a link cycle is never walked.
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            walk(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn hidden(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

/// `true` when `asset` exists and no input is newer. A missing input is
/// ignored; any other I/O failure counts as stale.
fn is_fresh(asset: &Path, inputs: &[PathBuf]) -> bool {
    let Ok(built) = asset.metadata().and_then(|m| m.modified()) else {
        return false;
    };
    inputs
        .iter()
        .all(|input| match input.metadata().and_then(|m| m.modified()) {
            Ok(changed) => changed <= built,
            Err(e) => e.kind() == ErrorKind::NotFound,
        })
}

fn skip_set(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

/// `pnpm install` then `pnpm build` in `dir`. A failed build deletes the
/// assets: the shared builder checks its output after esbuild wrote it.
fn run_build(pnpm: &Path, dir: &Path, assets: &[PathBuf]) -> Result<(), String> {
    for step in ["install", "build"] {
        let status = Command::new(pnpm)
            .arg(step)
            .current_dir(dir)
            .status()
            .map_err(|e| {
                format!(
                    "failed to spawn `{} {step}` in {}: {e}",
                    pnpm.display(),
                    dir.display()
                )
            })?;
        if !status.success() {
            if step == "build" {
                for asset in assets {
                    let _ = std::fs::remove_file(asset);
                }
            }
            return Err(format!(
                "`pnpm {step}` in {} exited with {status}, see the log above",
                dir.display()
            ));
        }
    }
    match assets.iter().find(|a| !a.exists()) {
        Some(missing) => Err(format!(
            "`pnpm build` finished but {} is missing, check the build output above",
            missing.display()
        )),
        None => Ok(()),
    }
}

/// `PNPM` when set; empty counts as unset, as for the skip variable.
fn explicit_pnpm(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn locate_pnpm(skip_env: &str) -> PathBuf {
    if let Some(explicit) = explicit_pnpm(std::env::var_os("PNPM")) {
        return explicit;
    }
    let names: &[&str] = if cfg!(windows) {
        &["pnpm.cmd", "pnpm.exe", "pnpm"]
    } else {
        &["pnpm"]
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home.map(PathBuf::from) {
        for candidate in [
            home.join(".local/share/pnpm/pnpm"),
            home.join(".local/bin/pnpm"),
        ] {
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!(
        "pnpm not found on PATH: install Node and pnpm, set PNPM to its path, or build the UI \
         yourself and set {skip_env}=1"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::time::{Duration, SystemTime};

    /// A repo root holding `work/ui` and `packages/console-ui`, removed on drop.
    struct Repo(PathBuf);

    impl Repo {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("ui-build-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Repo(root)
        }

        /// Writes `rel` with an mtime `age_secs` in the past.
        fn file(&self, rel: &str, age_secs: u64) -> PathBuf {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, rel).unwrap();
            let when = SystemTime::now() - Duration::from_secs(age_secs);
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
            path
        }

        fn crate_dir(&self) -> PathBuf {
            self.0.join("work")
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const ASSETS: &[&str] = &["dist/page.js"];
    const EXTRA: &[&str] = &["assets/icon.svg", "public", ".env.production"];

    fn frontend() -> Frontend<'static> {
        Frontend {
            dir: "ui",
            assets: ASSETS,
            packages: &["packages/console-ui"],
            extra: EXTRA,
            skip_env: "SKIP_UI_BUILD",
        }
    }

    /// Old inputs of every kind and a newer `ui/dist/page.js`.
    fn built_repo(name: &str) -> (Repo, PathBuf) {
        let repo = Repo::new(name);
        for rel in [
            "pnpm-lock.yaml",
            "work/ui/page.tsx",
            "work/ui/tailwind.mjs",
            "work/ui/src/lib/format.ts",
            "packages/console-ui/format.mjs",
            "packages/console-ui/src/hooks.ts",
            "work/assets/icon.svg",
            "work/public/logo.svg",
        ] {
            repo.file(rel, 100);
        }
        let asset = repo.file("work/ui/dist/page.js", 50);
        (repo, asset)
    }

    fn fresh(repo: &Repo, asset: &Path) -> bool {
        is_fresh(asset, &inputs(&repo.crate_dir(), &frontend()).files)
    }

    #[test]
    fn a_newer_input_of_any_kind_makes_the_asset_stale() {
        for touched in [
            "work/ui/tailwind.mjs",
            "work/ui/src/lib/format.ts",
            "pnpm-lock.yaml",
            "packages/console-ui/format.mjs",
            "packages/console-ui/src/hooks.ts",
            "work/assets/icon.svg",
            "work/public/logo.svg",
        ] {
            let (repo, asset) = built_repo("stale");
            assert!(fresh(&repo, &asset));
            repo.file(touched, 0);
            assert!(!fresh(&repo, &asset), "{touched}");
        }
    }

    #[test]
    fn build_output_dependencies_and_dotfiles_are_not_inputs() {
        let (repo, asset) = built_repo("ignored");
        for rel in [
            "work/ui/node_modules/react/index.js",
            "work/ui/dist/chunk.js",
            "work/ui/.tailwind/out.css",
            "work/ui/.DS_Store",
            "work/ui/tsconfig.tsbuildinfo",
            "packages/console-ui/node_modules/esbuild/lib/main.js",
        ] {
            repo.file(rel, 0);
        }
        assert!(fresh(&repo, &asset));
    }

    #[test]
    fn a_missing_asset_is_not_fresh() {
        let (repo, asset) = built_repo("missing");
        fs::remove_file(&asset).unwrap();
        assert!(!fresh(&repo, &asset));
    }

    #[test]
    fn skip_needs_a_non_empty_value() {
        assert!(!skip_set(None));
        assert!(!skip_set(Some("")));
        assert!(skip_set(Some("1")));
    }

    #[test]
    fn an_empty_pnpm_counts_as_unset() {
        assert_eq!(explicit_pnpm(None), None);
        assert_eq!(explicit_pnpm(Some("".into())), None);
        assert_eq!(
            explicit_pnpm(Some("/opt/pnpm".into())),
            Some(PathBuf::from("/opt/pnpm"))
        );
    }

    fn rerun_lines(repo: &Repo) -> Vec<String> {
        let watched = inputs(&repo.crate_dir(), &frontend()).watched;
        directives(&watched, "SKIP_UI_BUILD")
            .into_iter()
            .filter_map(|l| l.strip_prefix("cargo:rerun-if-changed=").map(String::from))
            .collect()
    }

    #[test]
    fn walked_directories_are_rerun_triggers_themselves() {
        let (repo, _asset) = built_repo("dirs");
        let lines = rerun_lines(&repo);
        let crate_dir = repo.crate_dir();
        for dir in [
            crate_dir.join("ui/src"),
            crate_dir.join("../packages/console-ui/src"),
            crate_dir.join("public"),
        ] {
            assert!(
                lines.contains(&dir.display().to_string()),
                "{}",
                dir.display()
            );
        }
        // A file under a watched directory is covered by the directory.
        assert!(!lines.iter().any(|l| l.ends_with("format.ts")), "{lines:?}");
        assert!(lines.contains(&crate_dir.join("ui/tailwind.mjs").display().to_string()));
    }

    #[test]
    fn no_project_root_or_build_output_is_a_rerun_trigger() {
        let (repo, _asset) = built_repo("roots");
        repo.file("work/ui/node_modules/react/index.js", 0);
        let lines = rerun_lines(&repo);
        let crate_dir = repo.crate_dir();
        for root in [
            crate_dir.clone(),
            crate_dir.join("ui"),
            crate_dir.join("../packages/console-ui"),
        ] {
            assert!(
                !lines.contains(&root.display().to_string()),
                "{}",
                root.display()
            );
        }
        assert!(
            !lines.iter().any(|l| Path::new(l)
                .components()
                .any(|c| c.as_os_str() == "node_modules" || c.as_os_str() == "dist")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_missing_input_is_not_a_rerun_trigger() {
        let (repo, _asset) = built_repo("absent");
        let lines = rerun_lines(&repo);
        assert!(
            !lines.iter().any(|l| l.ends_with(".env.production")),
            "{lines:?}"
        );
        repo.file("work/.env.production", 100);
        let lines = rerun_lines(&repo);
        assert!(
            lines.iter().any(|l| l.ends_with(".env.production")),
            "{lines:?}"
        );
    }

    /// `/bin/sh` stands in for pnpm: `sh install` and `sh build` run the
    /// scripts of those names in the project dir.
    #[cfg(unix)]
    fn scripted(name: &str, build: &str) -> (Repo, PathBuf, Vec<PathBuf>) {
        let repo = Repo::new(name);
        let dir = repo.crate_dir().join("ui");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("install"), "exit 0\n").unwrap();
        fs::write(dir.join("build"), build).unwrap();
        let assets = vec![dir.join("dist/page.js"), dir.join("dist/styles.css")];
        (repo, dir, assets)
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_build_deletes_the_assets_it_wrote() {
        let (_repo, dir, assets) = scripted(
            "failed",
            "mkdir -p dist; touch dist/page.js dist/styles.css; exit 1\n",
        );
        let err = run_build(Path::new("/bin/sh"), &dir, &assets).unwrap_err();
        assert!(err.contains("pnpm build"), "{err}");
        assert!(assets.iter().all(|a| !a.exists()));
    }

    #[cfg(unix)]
    #[test]
    fn a_successful_build_keeps_the_assets() {
        let (_repo, dir, assets) =
            scripted("ok", "mkdir -p dist; touch dist/page.js dist/styles.css\n");
        run_build(Path::new("/bin/sh"), &dir, &assets).unwrap();
        assert!(assets.iter().all(|a| a.exists()));
    }

    #[cfg(unix)]
    #[test]
    fn a_build_that_skips_an_asset_names_it() {
        let (_repo, dir, assets) = scripted("partial", "mkdir -p dist; touch dist/page.js\n");
        let err = run_build(Path::new("/bin/sh"), &dir, &assets).unwrap_err();
        assert!(err.contains("styles.css"), "{err}");
    }
}
