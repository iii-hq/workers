use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").expect("TARGET is set by Cargo")
    );

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

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.join("ui");
    let outputs = [ui_dir.join("dist/page.js"), ui_dir.join("dist/styles.css")];

    if outputs
        .iter()
        .all(|output| output.exists() && dist_is_fresh(output, &ui_dir))
    {
        return;
    }

    if std::env::var_os("SKIP_UI_BUILD").is_some() {
        for output in &outputs {
            assert!(
                output.exists(),
                "SKIP_UI_BUILD is set but {} is missing",
                output.display()
            );
        }
        return;
    }

    let pnpm = locate_pnpm();
    run(&pnpm, &["install"], &ui_dir);
    run(&pnpm, &["build"], &ui_dir);

    for output in &outputs {
        assert!(
            output.exists(),
            "UI build completed but {} is missing",
            output.display()
        );
    }
}

fn run(program: &Path, args: &[&str], cwd: &Path) {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", program.display()));
    assert!(status.success(), "{} {:?} failed", program.display(), args);
}

fn dist_is_fresh(output: &Path, ui_dir: &Path) -> bool {
    let Ok(output_mtime) = output.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };
    let inputs = [
        ui_dir.join("page.tsx"),
        ui_dir.join("styles.css"),
        ui_dir.join("build.mjs"),
        ui_dir.join("package.json"),
        ui_dir.join("tsconfig.json"),
        ui_dir.join("../../pnpm-lock.yaml"),
    ];
    inputs.into_iter().filter(|path| path.exists()).all(|path| {
        path.metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|mtime| mtime <= output_mtime)
    })
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
    panic!("pnpm was not found on PATH; install pnpm or prebuild ui/dist and set SKIP_UI_BUILD=1");
}
