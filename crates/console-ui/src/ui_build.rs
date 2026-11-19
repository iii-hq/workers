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
//! The inputs are the project's top-level files and `src/` tree, the same for
//! each bundled workspace package, the root `pnpm-lock.yaml` and
//! `pnpm-workspace.yaml`, and any extra input. A successful build writes a
//! marker, `.ui-build` in the project, listing those inputs and dated to the
//! start of the build. The assets are fresh when they all exist, the marker
//! lists the current inputs and no input is newer than it; otherwise
//! `pnpm install && pnpm build` runs again. The marker says "building" while
//! pnpm runs, so a failed or interrupted build is never fresh. Without any
//! marker (a `dist/` built by plain `pnpm build`, as CI pre-builds it) the
//! assets are fresh when no input is newer than they are. The assets are
//! never deleted: an explicit skip embeds whatever `dist/` was last written,
//! even by a build whose checks failed.

#![allow(dead_code)] // also compiled into build scripts that use only part of it

use std::ffi::OsStr;
use std::fs::File;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

pub struct Frontend<'a> {
    /// The pnpm project, relative to the crate: "ui", "native-ui", "web".
    pub dir: &'a str,
    /// The embedded outputs, relative to `dir`.
    pub assets: &'a [&'a str],
    /// Bundled workspace packages, relative to the repo root.
    pub packages: &'a [&'a str],
    /// Other inputs, relative to the crate; a directory is walked.
    pub extra: &'a [&'a str],
    /// Set and non-empty: use the existing assets as they are, without the
    /// marker (release jobs restore `dist/` built elsewhere).
    pub skip_env: &'a str,
}

/// In the project, outside `dist/` (vite empties that); gitignored.
const MARKER: &str = ".ui-build";
/// The marker's content while pnpm runs; never a listing, which has no
/// trailing newline.
const BUILDING: &str = "building\n";
/// At the repo root; gitignored.
const LOCK: &str = ".ui-build.lock";

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
    let skip = skip_set(std::env::var(frontend.skip_env).ok().as_deref());
    // Held until this build is checked or done: another cargo process on the
    // checkout (another target dir) waits, then finds it fresh, instead of
    // overwriting the assets this one validated.
    let _lock = if skip {
        None
    } else {
        lock(crate_dir.parent().unwrap_or(&crate_dir))
    };
    let inputs = inputs(&crate_dir, frontend);

    match needs_build(skip, &dir.join(MARKER), &assets, &inputs) {
        Err(missing) => panic!(
            "{} is set but {} is missing: build it (cd {} && pnpm install && pnpm build) \
             or unset {}",
            frontend.skip_env,
            missing.display(),
            dir.display(),
            frontend.skip_env
        ),
        Ok(false) => {}
        Ok(true) => {
            if let Some(e) = &inputs.unreadable {
                println!("cargo:warning=rebuilding {}: {e}", dir.display());
            }
            if let Err(e) = rebuild(&locate_pnpm(frontend.skip_env), &crate_dir, frontend) {
                panic!("{e}");
            }
        }
    }
    // Printed once every asset exists: deleting one then reruns the script
    // instead of failing in include_str!. A rebuild dates the assets before
    // this run, so writing them does not rerun it.
    let mut watched = inputs.watched;
    watched.extend(assets);
    for line in directives(&watched, frontend.skip_env) {
        println!("{line}");
    }
}

/// Whether to run pnpm; `Err(missing asset)` when skipping without it.
fn needs_build(
    skip: bool,
    marker: &Path,
    assets: &[PathBuf],
    inputs: &Inputs,
) -> Result<bool, PathBuf> {
    if skip {
        return match assets.iter().find(|a| !a.is_file()) {
            Some(missing) => Err(missing.clone()),
            None => Ok(false),
        };
    }
    Ok(!is_fresh(marker, assets, inputs))
}

/// What the assets are built from. `src` is the only directory entered in a
/// project, so `node_modules`, `dist` and `.tailwind` are never walked.
struct Inputs {
    /// The repo root; the marker lists inputs relative to it.
    root: PathBuf,
    /// Every input file, for the freshness check.
    files: Vec<PathBuf>,
    /// The same inputs for cargo, with each walked directory (a `src/`, an
    /// `extra` directory) in place of its files: cargo scans a directory
    /// recursively, so adding or removing a file there reruns the script.
    watched: Vec<PathBuf>,
    /// The first directory that could not be read: `files` is incomplete, so
    /// the assets are stale.
    unreadable: Option<String>,
}

impl Inputs {
    fn note(&mut self, path: &Path, e: std::io::Error) {
        if self.unreadable.is_none() {
            self.unreadable = Some(format!("cannot read {}: {e}", path.display()));
        }
    }
}

fn inputs(crate_dir: &Path, frontend: &Frontend) -> Inputs {
    let root = crate_dir.parent().unwrap_or(crate_dir).to_path_buf();
    let manifests = vec![
        root.join("pnpm-lock.yaml"),
        root.join("pnpm-workspace.yaml"),
    ];
    let mut out = Inputs {
        files: manifests.clone(),
        watched: manifests,
        root,
        unreadable: None,
    };
    project_files(&crate_dir.join(frontend.dir), &mut out);
    for package in frontend.packages {
        project_files(&out.root.join(package), &mut out);
    }
    for extra in frontend.extra {
        let path = crate_dir.join(extra);
        if path.is_dir() {
            walk(&path, &mut out);
        } else {
            out.files.push(path.clone());
        }
        out.watched.push(path);
    }
    out
}

/// The entries of `dir`; none for a directory that was never there, while
/// any other error marks the inputs unreadable.
fn entries(dir: &Path, out: &mut Inputs) -> Vec<std::fs::DirEntry> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) => {
            if e.kind() != ErrorKind::NotFound {
                out.note(dir, e);
            }
            return Vec::new();
        }
    };
    let mut list = Vec::new();
    for entry in read {
        match entry {
            Ok(entry) => list.push(entry),
            Err(e) => out.note(dir, e),
        }
    }
    list
}

fn project_files(dir: &Path, out: &mut Inputs) {
    for entry in entries(dir, out) {
        let path = entry.path();
        let name = entry.file_name();
        // tsc -b writes *.tsbuildinfo next to the sources: output, not input.
        if hidden(&name) || path.extension() == Some(OsStr::new("tsbuildinfo")) {
            continue;
        }
        if name == "src" && path.is_dir() {
            walk(&path, out);
            out.watched.push(path);
        } else if path.is_file() {
            out.files.push(path.clone());
            out.watched.push(path);
        }
    }
}

/// The `cargo:` lines for `watched`.
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

fn walk(dir: &Path, out: &mut Inputs) {
    for entry in entries(dir, out) {
        let path = entry.path();
        if hidden(&entry.file_name()) {
            continue;
        }
        // file_type does not follow symlinks, so a link cycle is never walked.
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out),
            Ok(_) => {
                if path.is_file() {
                    out.files.push(path);
                }
            }
            Err(e) => out.note(&path, e),
        }
    }
}

fn hidden(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

/// The marker's content: the existing inputs relative to the repo root,
/// sorted, so an input added or deleted since the build makes it stale.
fn listing(inputs: &Inputs) -> String {
    let mut lines: Vec<String> = inputs
        .files
        .iter()
        .filter(|f| f.exists())
        .map(|f| {
            f.strip_prefix(&inputs.root)
                .unwrap_or(f)
                .display()
                .to_string()
        })
        .collect();
    lines.sort();
    lines.join("\n")
}

fn mtime(path: &Path) -> Option<SystemTime> {
    path.metadata().and_then(|m| m.modified()).ok()
}

/// `true` when every asset exists, the marker lists the current inputs and
/// no input is newer than it; without a marker, no input may be newer than
/// the oldest asset. A missing input is ignored; any other I/O failure
/// counts as stale.
fn is_fresh(marker: &Path, assets: &[PathBuf], inputs: &Inputs) -> bool {
    if inputs.unreadable.is_some() || !assets.iter().all(|a| a.is_file()) {
        return false;
    }
    let built = match std::fs::read_to_string(marker) {
        Ok(listed) if listed == listing(inputs) => mtime(marker),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            assets.iter().map(|a| mtime(a)).min().flatten()
        }
        _ => None,
    };
    let Some(built) = built else {
        return false;
    };
    inputs
        .files
        .iter()
        .all(|input| match input.metadata().and_then(|m| m.modified()) {
            Ok(changed) => changed <= built,
            Err(e) => e.kind() == ErrorKind::NotFound,
        })
}

fn skip_set(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

/// Waits for, then holds, the checkout's build lock; builds without it, with
/// a warning, when the lock file cannot be used.
fn lock(root: &Path) -> Option<File> {
    let path = root.join(LOCK);
    match File::create(&path).and_then(|f| f.lock().map(|()| f)) {
        Ok(file) => Some(file),
        Err(e) => {
            println!("cargo:warning=building without {}: {e}", path.display());
            None
        }
    }
}

/// Marks the build as running, runs `pnpm install` then `pnpm build` in the
/// project and writes the listing once every asset exists. A failed build
/// leaves the assets as they are and the marker saying "building", so they
/// are never fresh.
fn rebuild(pnpm: &Path, crate_dir: &Path, frontend: &Frontend) -> Result<(), String> {
    let dir = crate_dir.join(frontend.dir);
    let marker = dir.join(MARKER);
    // Earlier than this script run, unless an input changed while it waited
    // for the lock.
    let before_run = inputs(crate_dir, frontend)
        .files
        .iter()
        .filter_map(|f| mtime(f))
        .max();
    std::fs::write(&marker, BUILDING)
        .map_err(|e| format!("cannot write {}: {e}", marker.display()))?;
    let started = SystemTime::now();
    for step in ["install", "build"] {
        let status = Command::new(pnpm)
            .arg(step)
            .current_dir(&dir)
            .status()
            .map_err(|e| {
                format!(
                    "failed to spawn `{} {step}` in {}: {e}",
                    pnpm.display(),
                    dir.display()
                )
            })?;
        if !status.success() {
            return Err(format!(
                "`pnpm {step}` in {} exited with {status}, see the log above",
                dir.display()
            ));
        }
    }
    let missing = frontend
        .assets
        .iter()
        .map(|a| dir.join(a))
        .find(|a| !a.is_file());
    if let Some(missing) = missing {
        return Err(format!(
            "`pnpm build` finished but {} is missing, check the build output above",
            missing.display()
        ));
    }
    // Listed after the build, which can create inputs (ade's web/public/vendor),
    // and dated to its start: an input saved while pnpm ran is newer, so the
    // next cargo build runs pnpm again.
    File::create(&marker)
        .and_then(|mut f| {
            f.write_all(listing(&inputs(crate_dir, frontend)).as_bytes())?;
            f.set_modified(started)
        })
        .map_err(|e| format!("cannot write {}: {e}", marker.display()))?;
    // Cargo reruns the script for a watched file newer than its start; best
    // effort, a failure only costs that one extra run.
    if let Some(before_run) = before_run {
        for asset in frontend.assets {
            let _ = File::options()
                .write(true)
                .open(dir.join(asset))
                .and_then(|f| f.set_modified(before_run));
        }
    }
    Ok(())
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
            age(&path, age_secs);
            path
        }

        fn crate_dir(&self) -> PathBuf {
            self.0.join("work")
        }

        fn inputs(&self) -> Inputs {
            inputs(&self.crate_dir(), &frontend())
        }

        fn marker(&self) -> PathBuf {
            self.crate_dir().join("ui").join(MARKER)
        }

        fn asset(&self) -> PathBuf {
            self.crate_dir().join("ui/dist/page.js")
        }

        fn fresh(&self) -> bool {
            is_fresh(&self.marker(), &[self.asset()], &self.inputs())
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn age(path: &Path, secs: u64) {
        let when = SystemTime::now() - Duration::from_secs(secs);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    fn frontend() -> Frontend<'static> {
        Frontend {
            dir: "ui",
            assets: &["dist/page.js"],
            packages: &["packages/console-ui"],
            extra: &["assets/icon.svg", "public", ".env.production"],
            skip_env: "SKIP_UI_BUILD",
        }
    }

    /// Old inputs of every kind, then `ui/dist/page.js` and a marker 50 s ago.
    fn built_repo(name: &str) -> Repo {
        let repo = Repo::new(name);
        for rel in [
            "pnpm-lock.yaml",
            "pnpm-workspace.yaml",
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
        repo.file("work/ui/dist/page.js", 50);
        fs::write(repo.marker(), listing(&repo.inputs())).unwrap();
        age(&repo.marker(), 50);
        repo
    }

    #[test]
    fn a_newer_input_of_any_kind_makes_the_asset_stale() {
        for touched in [
            "work/ui/tailwind.mjs",
            "work/ui/src/lib/format.ts",
            "pnpm-lock.yaml",
            "pnpm-workspace.yaml",
            "packages/console-ui/format.mjs",
            "packages/console-ui/src/hooks.ts",
            "work/assets/icon.svg",
            "work/public/logo.svg",
        ] {
            let repo = built_repo("stale");
            assert!(repo.fresh());
            repo.file(touched, 0);
            assert!(!repo.fresh(), "{touched}");
        }
    }

    #[test]
    fn an_added_or_removed_input_makes_the_asset_stale() {
        let repo = built_repo("added");
        repo.file("work/ui/src/lib/new.ts", 100);
        assert!(!repo.fresh());

        let repo = built_repo("removed");
        fs::remove_file(repo.0.join("packages/console-ui/format.mjs")).unwrap();
        assert!(!repo.fresh());
    }

    #[test]
    fn build_output_dependencies_and_dotfiles_are_not_inputs() {
        let repo = built_repo("ignored");
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
        assert!(repo.fresh());
    }

    #[test]
    fn a_missing_asset_or_a_running_build_is_not_fresh() {
        let repo = built_repo("missing");
        fs::remove_file(repo.asset()).unwrap();
        assert!(!repo.fresh());

        let repo = built_repo("building");
        fs::write(repo.marker(), BUILDING).unwrap();
        age(&repo.marker(), 50);
        assert!(!repo.fresh());
    }

    #[test]
    fn without_a_marker_the_assets_must_be_newer_than_the_inputs() {
        // A dist/ built by plain `pnpm build`, as CI pre-builds it.
        let repo = built_repo("unmarked");
        fs::remove_file(repo.marker()).unwrap();
        assert!(repo.fresh());
        repo.file("work/ui/src/lib/format.ts", 0);
        assert!(!repo.fresh());
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_is_stale() {
        use std::os::unix::fs::PermissionsExt;
        let repo = built_repo("unreadable");
        let lib = repo.crate_dir().join("ui/src/lib");
        fs::set_permissions(&lib, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::read_dir(&lib).is_ok(); // root reads it anyway
        let inputs = repo.inputs();
        fs::set_permissions(&lib, fs::Permissions::from_mode(0o755)).unwrap();
        if readable {
            return;
        }
        assert!(inputs.unreadable.is_some());
        assert!(!is_fresh(&repo.marker(), &[repo.asset()], &inputs));
        assert!(repo.fresh());
    }

    #[test]
    fn skip_uses_the_assets_of_a_failed_build() {
        let repo = built_repo("skip");
        fs::write(repo.marker(), BUILDING).unwrap();
        let (marker, assets, inputs) = (repo.marker(), [repo.asset()], repo.inputs());
        assert_eq!(needs_build(true, &marker, &assets, &inputs), Ok(false));
        assert_eq!(needs_build(false, &marker, &assets, &inputs), Ok(true));
        fs::remove_file(repo.asset()).unwrap();
        assert_eq!(
            needs_build(true, &marker, &assets, &inputs),
            Err(repo.asset())
        );
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
        let mut watched = repo.inputs().watched;
        watched.push(repo.asset());
        directives(&watched, "SKIP_UI_BUILD")
            .into_iter()
            .filter_map(|l| l.strip_prefix("cargo:rerun-if-changed=").map(String::from))
            .collect()
    }

    #[test]
    fn walked_directories_manifests_and_assets_are_rerun_triggers() {
        let repo = built_repo("dirs");
        let lines = rerun_lines(&repo);
        let crate_dir = repo.crate_dir();
        for path in [
            crate_dir.join("ui/src"),
            repo.0.join("packages/console-ui/src"),
            crate_dir.join("public"),
            repo.0.join("pnpm-lock.yaml"),
            repo.0.join("pnpm-workspace.yaml"),
            repo.asset(),
        ] {
            assert!(
                lines.contains(&path.display().to_string()),
                "{}",
                path.display()
            );
        }
        // A file under a watched directory is covered by the directory.
        assert!(!lines.iter().any(|l| l.ends_with("format.ts")), "{lines:?}");
        assert!(lines.contains(&crate_dir.join("ui/tailwind.mjs").display().to_string()));
    }

    #[test]
    fn no_project_root_or_build_directory_is_a_rerun_trigger() {
        let repo = built_repo("roots");
        repo.file("work/ui/node_modules/react/index.js", 0);
        let lines = rerun_lines(&repo);
        let crate_dir = repo.crate_dir();
        for root in [
            crate_dir.clone(),
            crate_dir.join("ui"),
            crate_dir.join("ui/dist"),
            repo.0.join("packages/console-ui"),
        ] {
            assert!(
                !lines.contains(&root.display().to_string()),
                "{}",
                root.display()
            );
        }
        assert!(
            !lines.iter().any(|l| l.contains("node_modules")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_missing_input_is_not_a_rerun_trigger() {
        let repo = built_repo("absent");
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

    /// A built repo where `/bin/sh` stands in for pnpm: `sh install` and
    /// `sh build` run the scripts of those names in `ui/`.
    #[cfg(unix)]
    fn scripted(name: &str, build: &str) -> Repo {
        let repo = built_repo(name);
        fs::write(repo.crate_dir().join("ui/install"), "exit 0\n").unwrap();
        fs::write(repo.crate_dir().join("ui/build"), build).unwrap();
        repo
    }

    #[cfg(unix)]
    fn run(repo: &Repo) -> Result<(), String> {
        rebuild(Path::new("/bin/sh"), &repo.crate_dir(), &frontend())
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_build_keeps_the_assets_but_is_stale() {
        let repo = scripted("failed", "echo new > dist/page.js; exit 1\n");
        let err = run(&repo).unwrap_err();
        assert!(err.contains("pnpm build"), "{err}");
        assert_eq!(fs::read_to_string(repo.asset()).unwrap(), "new\n");
        assert!(!repo.fresh());
    }

    #[cfg(unix)]
    #[test]
    fn an_interrupted_build_is_stale() {
        // Killed after writing a newer asset, before its checks finished.
        let repo = scripted("interrupted", "echo new > dist/page.js; kill -9 $$\n");
        assert!(run(&repo).is_err());
        assert!(repo.asset().exists());
        assert_eq!(fs::read_to_string(repo.marker()).unwrap(), BUILDING);
        assert!(!repo.fresh());
    }

    #[cfg(unix)]
    #[test]
    fn an_input_saved_while_the_build_runs_is_stale() {
        // esbuild read the old source; the save lands before the checks end.
        let repo = scripted(
            "saved",
            "echo new > dist/page.js; touch src/lib/format.ts\n",
        );
        run(&repo).unwrap();
        assert!(!repo.fresh());
    }

    #[cfg(unix)]
    #[test]
    fn a_rebuild_dates_its_assets_before_the_run() {
        // Cargo compares watched files with the script's start.
        let repo = scripted("dated", "echo new > dist/page.js\n");
        let start = SystemTime::now();
        run(&repo).unwrap();
        assert!(mtime(&repo.asset()).unwrap() < start);
        assert!(repo.fresh());
    }

    #[cfg(unix)]
    #[test]
    fn a_successful_build_writes_the_marker_and_is_fresh() {
        let repo = scripted("ok", "echo new > dist/page.js\n");
        repo.file("work/ui/src/lib/format.ts", 0);
        assert!(!repo.fresh());
        run(&repo).unwrap();
        assert!(repo.fresh());
    }

    #[cfg(unix)]
    #[test]
    fn a_build_that_skips_an_asset_names_it() {
        let repo = scripted("partial", "rm dist/page.js\n");
        let err = run(&repo).unwrap_err();
        assert!(err.contains("page.js"), "{err}");
        assert!(!repo.fresh());
    }
}
