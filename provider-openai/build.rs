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
        "ui/src",
        "ui/styles.css",
        "ui/build.mjs",
        "ui/package.json",
        "ui/tsconfig.json",
        "../pnpm-lock.yaml",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=SKIP_UI_BUILD");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
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
            panic!("SKIP_UI_BUILD set but provider-openai UI assets are missing");
        }
        return;
    }

    let pnpm = locate_pnpm();
    let install = Command::new(&pnpm)
        .arg("install")
        .arg("--frozen-lockfile")
        .current_dir(&ui_dir)
        .status()
        .expect("failed to run pnpm install for provider-openai UI");
    if !install.success() {
        panic!("pnpm install failed for provider-openai UI");
    }
    let build = Command::new(&pnpm)
        .arg("build")
        .current_dir(&ui_dir)
        .status()
        .expect("failed to build provider-openai UI");
    if !build.success() {
        panic!("pnpm build failed for provider-openai UI");
    }
}

fn dist_is_fresh(asset: &Path, ui_dir: &Path) -> bool {
    let Ok(dist_mtime) = asset.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };
    for source in [
        ui_dir.join("page.tsx"),
        ui_dir.join("src"),
        ui_dir.join("styles.css"),
        ui_dir.join("build.mjs"),
        ui_dir.join("package.json"),
        ui_dir.join("tsconfig.json"),
        ui_dir.join("../../pnpm-lock.yaml"),
    ] {
        if source_is_newer(&source, dist_mtime) {
            return false;
        }
    }
    true
}

fn source_is_newer(source: &Path, dist_mtime: std::time::SystemTime) -> bool {
    // A deleted or unreadable source must rebuild, not reuse stale assets.
    let Ok(metadata) = source.metadata() else {
        return true;
    };
    if metadata.modified().is_ok_and(|mtime| mtime > dist_mtime) {
        return true;
    }
    if metadata.is_dir() {
        let Ok(mut entries) = source.read_dir() else {
            return true;
        };
        return entries
            .any(|entry| entry.map_or(true, |entry| source_is_newer(&entry.path(), dist_mtime)));
    }
    false
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
    panic!("pnpm is required to build provider-openai UI");
}

#[cfg(test)]
mod tests {
    use super::dist_is_fresh;
    use std::fs::{self, File};
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    fn touch(path: &Path, mtime: SystemTime) {
        File::create(path).unwrap().set_modified(mtime).unwrap();
    }

    #[test]
    fn source_edits_and_deletions_invalidate_built_assets() {
        let root = std::env::temp_dir().join(format!("openai-ui-freshness-{}", std::process::id()));
        let ui = root.join("provider/ui");
        fs::create_dir_all(ui.join("src/login")).unwrap();
        let asset = ui.join("page.js");
        let source = ui.join("src/login/session.ts");
        let before = SystemTime::now() - Duration::from_secs(60);
        let built = before + Duration::from_secs(20);
        touch(&source, before);
        for name in [
            "page.tsx",
            "styles.css",
            "build.mjs",
            "package.json",
            "tsconfig.json",
        ] {
            touch(&ui.join(name), before);
        }
        touch(&root.join("pnpm-lock.yaml"), before);
        for dir in ["src/login", "src"] {
            File::open(ui.join(dir))
                .unwrap()
                .set_modified(before)
                .unwrap();
        }
        touch(&asset, built);
        assert!(dist_is_fresh(&asset, &ui));

        File::open(&source)
            .unwrap()
            .set_modified(built + Duration::from_secs(1))
            .unwrap();
        assert!(!dist_is_fresh(&asset, &ui));

        File::open(&source).unwrap().set_modified(before).unwrap();
        assert!(dist_is_fresh(&asset, &ui));
        fs::remove_file(ui.join("styles.css")).unwrap();
        assert!(!dist_is_fresh(&asset, &ui));
        fs::remove_dir_all(root).unwrap();
    }
}
