use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    for path in [
        "ui/page.tsx",
        "ui/styles.css",
        "ui/src",
        "ui/build.mjs",
        "ui/package.json",
        "ui/tsconfig.json",
        "../pnpm-lock.yaml",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

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
        for asset in &assets {
            assert!(
                asset.exists(),
                "SKIP_UI_BUILD set but {} is missing",
                asset.display()
            );
        }
        return;
    }

    let pnpm = locate_pnpm();
    run(&pnpm, &["install"], &ui_dir);
    run(&pnpm, &["build"], &ui_dir);
    for asset in &assets {
        assert!(
            asset.exists(),
            "UI build finished but {} is missing",
            asset.display()
        );
    }
}

fn run(program: &Path, args: &[&str], directory: &Path) {
    let status = Command::new(program)
        .args(args)
        .current_dir(directory)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "failed to run {} in {}: {error}",
                program.display(),
                directory.display()
            )
        });
    assert!(
        status.success(),
        "{} exited with {status}",
        program.display()
    );
}

fn dist_is_fresh(asset: &Path, ui_dir: &Path) -> bool {
    let Ok(asset_time) = asset.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };
    for path in [
        ui_dir.join("page.tsx"),
        ui_dir.join("styles.css"),
        ui_dir.join("build.mjs"),
        ui_dir.join("package.json"),
        ui_dir.join("tsconfig.json"),
        ui_dir.join("../../pnpm-lock.yaml"),
    ] {
        if path.exists() {
            match path.metadata().and_then(|metadata| metadata.modified()) {
                Ok(modified) if modified <= asset_time => {}
                _ => return false,
            }
        }
    }
    subtree_is_older(&ui_dir.join("src"), asset_time)
}

fn subtree_is_older(root: &Path, ceiling: SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            return false;
        };
        if metadata.is_dir() {
            if !subtree_is_older(&path, ceiling) {
                return false;
            }
        } else {
            match metadata.modified() {
                Ok(modified) if modified <= ceiling => {}
                _ => return false,
            }
        }
    }
    true
}

fn locate_pnpm() -> PathBuf {
    if let Ok(explicit) = std::env::var("PNPM") {
        return PathBuf::from(explicit);
    }
    let names: &[&str] = if cfg!(windows) {
        &["pnpm.cmd", "pnpm.exe", "pnpm"]
    } else {
        &["pnpm"]
    };
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!("pnpm not found on PATH; build eval/ui first or set PNPM");
}
