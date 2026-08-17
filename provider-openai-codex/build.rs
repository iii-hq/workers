use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").expect("TARGET must be set by Cargo build scripts")
    );

    if std::env::var_os("CARGO_FEATURE_CONSOLE_UI").is_none() {
        return;
    }

    for path in [
        "ui/page.tsx",
        "ui/styles.css",
        "ui/build.mjs",
        "ui/package.json",
        "ui/tsconfig.json",
        "../pnpm-lock.yaml",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=SKIP_UI_BUILD");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.join("ui");
    let assets = [ui_dir.join("dist/page.js"), ui_dir.join("dist/styles.css")];
    if assets
        .iter()
        .all(|asset| asset.exists() && dist_is_fresh(asset, &ui_dir))
    {
        return;
    }

    if std::env::var_os("SKIP_UI_BUILD").is_some() {
        if assets.iter().any(|asset| !asset.exists()) {
            panic!("SKIP_UI_BUILD set but provider-openai-codex UI assets are missing");
        }
        return;
    }

    let pnpm = locate_pnpm();
    let install = Command::new(&pnpm)
        .arg("install")
        .current_dir(&ui_dir)
        .status()
        .expect("failed to run pnpm install for provider-openai-codex UI");
    if !install.success() {
        panic!("pnpm install failed for provider-openai-codex UI");
    }
    let build = Command::new(&pnpm)
        .arg("build")
        .current_dir(&ui_dir)
        .status()
        .expect("failed to build provider-openai-codex UI");
    if !build.success() {
        panic!("pnpm build failed for provider-openai-codex UI");
    }
}

fn dist_is_fresh(asset: &Path, ui_dir: &Path) -> bool {
    let Ok(dist_mtime) = asset.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };
    for source in [
        ui_dir.join("page.tsx"),
        ui_dir.join("styles.css"),
        ui_dir.join("build.mjs"),
        ui_dir.join("package.json"),
        ui_dir.join("tsconfig.json"),
        ui_dir.join("../../pnpm-lock.yaml"),
    ] {
        if let Ok(source_mtime) = source.metadata().and_then(|metadata| metadata.modified()) {
            if source_mtime > dist_mtime {
                return false;
            }
        }
    }
    true
}

fn locate_pnpm() -> String {
    for candidate in ["pnpm", "pnpm.cmd"] {
        if Command::new(candidate).arg("--version").output().is_ok() {
            return candidate.to_string();
        }
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if let Some(home) = home {
        for candidate in [
            home.join(".local/share/pnpm/pnpm"),
            home.join(".local/bin/pnpm"),
        ] {
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    panic!("pnpm is required to build provider-openai-codex UI");
}
