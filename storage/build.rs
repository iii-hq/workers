fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").expect("TARGET must be set by Cargo build scripts")
    );

    println!("cargo:rerun-if-changed=ui/page.tsx");
    println!("cargo:rerun-if-changed=ui/styles.css");
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/build.mjs");
    println!("cargo:rerun-if-changed=ui/package.json");
    println!("cargo:rerun-if-changed=ui/tsconfig.json");
    println!("cargo:rerun-if-changed=../pnpm-lock.yaml");

    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.join("ui");
    let assets = [ui_dir.join("dist/page.js"), ui_dir.join("dist/styles.css")];
    let sources = [
        ui_dir.join("page.tsx"),
        ui_dir.join("styles.css"),
        ui_dir.join("build.mjs"),
        ui_dir.join("package.json"),
        ui_dir.join("tsconfig.json"),
        manifest_dir.join("../pnpm-lock.yaml"),
    ];
    let fresh = assets.iter().all(|asset| {
        let Ok(asset_time) = asset.metadata().and_then(|metadata| metadata.modified()) else {
            return false;
        };
        sources.iter().all(|source| {
            source
                .metadata()
                .and_then(|metadata| metadata.modified())
                .map(|time| time <= asset_time)
                .unwrap_or(false)
        }) && subtree_older_than(&ui_dir.join("src"), asset_time)
    });
    if fresh {
        return;
    }
    if std::env::var_os("SKIP_UI_BUILD").is_some() {
        if assets.iter().any(|asset| !asset.exists()) {
            panic!("SKIP_UI_BUILD is set but storage/ui/dist assets are missing");
        }
        return;
    }
    let pnpm = std::env::var_os("PNPM").unwrap_or_else(|| "pnpm".into());
    let install = std::process::Command::new(&pnpm)
        .arg("install")
        .current_dir(&ui_dir)
        .status()
        .expect("run pnpm install for storage UI");
    if !install.success() {
        panic!("pnpm install for storage UI failed with {install}");
    }
    let build = std::process::Command::new(&pnpm)
        .arg("build")
        .current_dir(&ui_dir)
        .status()
        .expect("run pnpm build for storage UI");
    if !build.success() {
        panic!("pnpm build for storage UI failed with {build}");
    }
}

fn subtree_older_than(root: &std::path::Path, ceiling: std::time::SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().all(|entry| {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            return false;
        };
        if metadata.is_dir() {
            subtree_older_than(&path, ceiling)
        } else {
            metadata.modified().is_ok_and(|time| time <= ceiling)
        }
    })
}
