use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// CPython 3.14.7 built for wasm32-wasip1 with WASI SDK 24, from the CPython
/// WASI platform maintainer. WASI SDK 26/27 are known-bad upstream (CPython
/// hang) — never bump the SDK half of this pin casually.
const URL: &str = "https://github.com/brettcannon/cpython-wasi-build/releases/download/v3.14.7/python-3.14.7-wasi_sdk-24.zip";
const SHA256: &str = "2e064d3fb8172471d39d741348efa722349c40b96301f69968dff714999c584b";

fn sha256_hex(path: &PathBuf) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let bytes = fs::read(path).ok()?;
    // digest 0.11's `Output` is a `hybrid_array::Array`, which doesn't impl
    // `LowerHex` — format byte-by-byte instead of `format!("{:x}", ..)`.
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest.iter() {
        write!(hex, "{b:02x}").ok()?;
    }
    Some(hex)
}

fn main() {
    println!("cargo:rerun-if-env-changed=III_PYTHON_WASI_ZIP");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("python-wasi.zip");
    // Sidecar digest file that artifact.rs embeds via include_str!. Writing
    // it here (rather than duplicating the SHA256 literal in artifact.rs)
    // means the cache key artifact.rs derives from ZIP_SHA256 can never
    // drift from the value build.rs actually verified against.
    let digest_path = out_dir.join("python-wasi.sha256");

    if sha256_hex(&dest).as_deref() == Some(SHA256) {
        // Already fetched and verified. Recreate the digest sidecar if it
        // was removed independently of the zip (e.g. a manual `rm`) so a
        // stale OUT_DIR can't leave artifact.rs's include_str! dangling.
        if !digest_path.is_file() {
            fs::write(&digest_path, SHA256).expect("writing digest sidecar file");
        }
        return;
    }
    if let Ok(local) = env::var("III_PYTHON_WASI_ZIP") {
        fs::copy(&local, &dest).unwrap_or_else(|e| panic!("copying {local}: {e}"));
    } else {
        let status = Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(&dest)
            .arg(URL)
            .status()
            .expect("curl must be on PATH (or set III_PYTHON_WASI_ZIP to a local copy)");
        assert!(status.success(), "downloading {URL} failed");
    }
    let got = sha256_hex(&dest).unwrap_or_default();
    assert_eq!(
        got, SHA256,
        "python-wasi.zip digest mismatch — refusing to embed an unverified interpreter"
    );
    fs::write(&digest_path, SHA256).expect("writing digest sidecar file");
}
