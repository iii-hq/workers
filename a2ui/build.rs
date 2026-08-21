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
    let outputs = [ui_dir.join("dist/page.js"), ui_dir.join("dist/styles.css")];
    if outputs
        .iter()
        .all(|output| output.exists() && subtree_older_than(&ui_dir, modified(output)))
    {
        return;
    }
    if std::env::var_os("SKIP_UI_BUILD").is_some() {
        if outputs.iter().any(|output| !output.exists()) {
            panic!("SKIP_UI_BUILD set but A2UI UI assets are missing");
        }
        return;
    }

    let pnpm = locate_pnpm();
    run(&pnpm, &["install"], &ui_dir);
    run(&pnpm, &["build"], &ui_dir);
    if outputs.iter().any(|output| !output.exists()) {
        panic!("A2UI UI build finished without expected dist assets");
    }
}

fn modified(path: &Path) -> SystemTime {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn subtree_older_than(root: &Path, ceiling: SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.components().any(|part| part.as_os_str() == "dist") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            return false;
        };
        if metadata.is_dir() {
            if !subtree_older_than(&path, ceiling) {
                return false;
            }
        } else if metadata.modified().map_or(true, |time| time > ceiling) {
            return false;
        }
    }
    true
}

fn run(program: &Path, args: &[&str], cwd: &Path) {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", program.display()));
    if !status.success() {
        panic!(
            "{} {} exited with {status}",
            program.display(),
            args.join(" ")
        );
    }
}

fn locate_pnpm() -> PathBuf {
    if let Ok(path) = std::env::var("PNPM") {
        return PathBuf::from(path);
    }
    let names = if cfg!(windows) {
        &["pnpm.cmd", "pnpm.exe", "pnpm"][..]
    } else {
        &["pnpm"][..]
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
