use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    if let Ok(target) = std::env::var("TARGET") {
        println!("cargo:rustc-env=TARGET={target}");
    }

    println!("cargo:rerun-if-changed=ui/page.tsx");
    println!("cargo:rerun-if-changed=ui/styles.css");
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/build.mjs");
    println!("cargo:rerun-if-changed=ui/package.json");
    println!("cargo:rerun-if-changed=ui/tsconfig.json");
    println!("cargo:rerun-if-changed=../pnpm-lock.yaml");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.join("ui");
    let assets = [
        ui_dir.join("dist").join("page.js"),
        ui_dir.join("dist").join("styles.css"),
    ];

    if assets
        .iter()
        .all(|asset| asset.exists() && dist_is_fresh(asset, &ui_dir))
    {
        return;
    }

    if std::env::var_os("SKIP_UI_BUILD").is_some() {
        for asset in &assets {
            if !asset.exists() {
                panic!(
                    "SKIP_UI_BUILD set but {} is missing; build the UI first",
                    asset.display()
                );
            }
        }
        return;
    }

    let pnpm = locate_pnpm();
    run(&pnpm, &["install"], &ui_dir);
    run(&pnpm, &["build"], &ui_dir);

    for asset in &assets {
        if !asset.exists() {
            panic!("UI build finished but {} is still missing", asset.display());
        }
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
    if !status.success() {
        panic!("{} exited with {status}", program.display());
    }
}

fn dist_is_fresh(asset: &Path, ui_dir: &Path) -> bool {
    let Ok(asset_time) = asset.metadata().and_then(|metadata| metadata.modified()) else {
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
        if !source.exists() {
            continue;
        }
        let Ok(source_time) = source.metadata().and_then(|metadata| metadata.modified()) else {
            return false;
        };
        if source_time > asset_time {
            return false;
        }
    }

    subtree_older_than(&ui_dir.join("src"), asset_time)
}

fn subtree_older_than(root: &Path, ceiling: SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            return false;
        };
        if metadata.is_dir() {
            if !subtree_older_than(&path, ceiling) {
                return false;
            }
        } else {
            let Ok(modified) = metadata.modified() else {
                return false;
            };
            if modified > ceiling {
                return false;
            }
        }
    }
    true
}

fn locate_pnpm() -> PathBuf {
    if let Ok(explicit) = std::env::var("PNPM") {
        return PathBuf::from(explicit);
    }
    let names = if cfg!(windows) {
        ["pnpm.cmd", "pnpm.exe", "pnpm"].as_slice()
    } else {
        ["pnpm"].as_slice()
    };
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!("pnpm not found on PATH");
}
