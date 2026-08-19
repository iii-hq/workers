use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const UNWINDER_SYMBOLS: &[&str] = &[
    "_Unwind_DeleteException",
    "_Unwind_ForcedUnwind",
    "_Unwind_GetGR",
    "_Unwind_GetIP",
    "_Unwind_GetLanguageSpecificData",
    "_Unwind_GetRegionStart",
    "_Unwind_RaiseException",
    "_Unwind_Resume",
    "_Unwind_SetGR",
    "_Unwind_SetIP",
    "__unw_add_dynamic_eh_frame_section",
    "__unw_add_dynamic_fde",
    "__unw_get_fpreg",
    "__unw_get_proc_info",
    "__unw_get_proc_name",
    "__unw_get_reg",
    "__unw_getcontext",
    "__unw_init_local",
    "__unw_is_fpreg",
    "__unw_is_signal_frame",
    "__unw_iterate_dwarf_unwind_cache",
    "__unw_regname",
    "__unw_remove_dynamic_eh_frame_section",
    "__unw_remove_dynamic_fde",
    "__unw_resume",
    "__unw_set_fpreg",
    "__unw_set_reg",
    "__unw_step",
    "__unw_step_stage2",
];

#[derive(Debug)]
struct Artifact<'a> {
    target: &'a str,
    archive: &'a str,
    bytes: u64,
    sha256: &'a str,
}

fn artifact_for_target<'a>(manifest: &'a str, target: &str) -> Option<Artifact<'a>> {
    manifest
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split('|');
            let artifact = Artifact {
                target: fields.next()?,
                archive: fields.next()?,
                bytes: fields.next()?.parse().ok()?,
                sha256: fields.next()?,
            };
            (artifact.target == target).then_some(artifact)
        })
}

fn sha256(path: &Path) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .map_err(|error| format!("cannot run sha256sum: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "sha256sum failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("sha256sum returned non-UTF-8 output: {error}"))?
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| "sha256sum returned empty output".to_string())
}

fn certified_artifact_dir(manifest_dir: &Path, target: &str) -> PathBuf {
    env::var_os("CURL_IMPERSONATE_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("artifacts"))
        .join(target)
}

fn main() {
    println!("cargo:rustc-check-cfg=cfg(curl_impersonate_linked)");
    println!("cargo:rerun-if-changed=artifacts.manifest");
    println!("cargo:rerun-if-env-changed=CURL_IMPERSONATE_ARTIFACT_DIR");

    // Safe/default builds compile the bindings and their tests without a
    // native artifact. Certified compat builds explicitly opt into linking.
    if env::var_os("CARGO_FEATURE_CERTIFIED").is_none() {
        return;
    }

    let target = env::var("TARGET").expect("Cargo always sets TARGET");
    let manifest = include_str!("artifacts.manifest");
    let artifact = artifact_for_target(manifest, &target).unwrap_or_else(|| {
        panic!(
            "curl-impersonate certified mode is unsupported for target {target}; \
             supported targets are x86_64-unknown-linux-gnu and aarch64-unknown-linux-gnu"
        )
    });
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let artifact_dir = certified_artifact_dir(&manifest_dir, &target);
    let archive = artifact_dir.join(artifact.archive);
    let root = artifact_dir.join("root");
    let library = root.join("libcurl-impersonate.a");
    let header = root.join("include/curl/curl.h");

    let metadata = fs::metadata(&archive).unwrap_or_else(|_| {
        panic!(
            "certified curl-impersonate artifact is absent: {}; run scripts/fetch_curl_impersonate_artifacts.sh {target}",
            archive.display()
        )
    });
    assert_eq!(
        metadata.len(),
        artifact.bytes,
        "certified curl-impersonate artifact has wrong byte length: {}",
        archive.display()
    );
    let actual = sha256(&archive).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        actual,
        artifact.sha256,
        "certified curl-impersonate artifact checksum mismatch: {}",
        archive.display()
    );
    assert!(
        library.is_file() && header.is_file(),
        "certified artifact was verified but not extracted under {}; run the artifact fetch script",
        root.display()
    );

    // The official monolithic static archive includes LLVM libunwind. Leaving
    // those symbols global replaces Rust's process unwinder and makes error
    // backtraces segfault. Keep curl's internal references local while letting
    // the binary use its normal libgcc unwinder.
    let link_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("native");
    fs::create_dir_all(&link_dir).expect("create native link directory");
    let sanitized = link_dir.join("libcurl-impersonate.a");
    fs::copy(&library, &sanitized).expect("copy verified curl archive");
    let mut objcopy = Command::new("objcopy");
    for symbol in UNWINDER_SYMBOLS {
        objcopy.arg(format!("--localize-symbol={symbol}"));
    }
    let output = objcopy
        .arg(&sanitized)
        .output()
        .expect("certified builds require GNU objcopy");
    assert!(
        output.status.success(),
        "objcopy failed to isolate curl's bundled unwinder: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );

    println!("cargo:rustc-link-search=native={}", link_dir.display());
    println!("cargo:rustc-link-lib=static=curl-impersonate");
    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-cfg=curl_impersonate_linked");
}
