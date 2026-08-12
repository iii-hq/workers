use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use wasmtime::{Config, Engine, Module};

/// The pinned CPython-WASI release zip, verified against ZIP_SHA256 by build.rs.
pub const PYTHON_WASI_ZIP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/python-wasi.zip"));
/// The digest build.rs verified the embedded zip against, embedded from the
/// sidecar file it wrote — so this can never drift from what was actually
/// checked at build time (see build.rs's `SHA256` pin).
pub const ZIP_SHA256: &str = include_str!(concat!(env!("OUT_DIR"), "/python-wasi.sha256"));
pub const WASMTIME_VERSION: &str = "47.0.3";

/// How often the epoch ticker fires; deadlines are expressed in these ticks.
pub const EPOCH_TICK_MS: u64 = 10;

/// The Engine every runtime shares: epoch interruption on, everything else default.
pub fn sandbox_engine() -> Result<Engine> {
    let mut cfg = Config::new();
    cfg.epoch_interruption(true);
    // wasmtime::Error deliberately doesn't impl std::error::Error (see its
    // docs), so anyhow's `Context` trait doesn't apply directly; convert via
    // the crate's own `From<wasmtime::Error> for anyhow::Error` first.
    Engine::new(&cfg)
        .map_err(anyhow::Error::from)
        .context("building wasmtime engine")
}

fn cache_root() -> PathBuf {
    // Containers routinely run with neither XDG_CACHE_HOME nor HOME set;
    // this sits on the worker boot path, so that case must degrade to the
    // temp dir (losing cache reuse across restarts), not panic the process.
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("iii").join("python-engine").join(ZIP_SHA256)
}

/// Extract the embedded bundle into the versioned cache dir if it isn't
/// there yet. Atomic against concurrent extractors — separate processes, or
/// separate threads in the same process (as `cargo test` uses by default):
/// extract into a `tempfile`-unique temp sibling, then rename; a loser of
/// the rename race drops its own temp dir and uses the winner's tree.
pub fn ensure_extracted() -> Result<PathBuf> {
    let root = cache_root();
    if root.join("python.wasm").is_file() {
        return Ok(root);
    }
    let parent = root.parent().expect("cache root has a parent");
    fs::create_dir_all(parent)?;
    // A pid-only tmp name would collide between threads of the same test
    // binary; `tempfile` gives every call its own unique directory instead.
    let tmp = tempfile::Builder::new()
        .prefix("extract-tmp-")
        .tempdir_in(parent)
        .context("creating extraction tempdir")?;

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(PYTHON_WASI_ZIP))
        .context("embedded zip is unreadable")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            anyhow::bail!("zip entry {} has an unsafe path", entry.name());
        };
        let dest = tmp.path().join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&dest)?;
        } else {
            if let Some(dir) = dest.parent() {
                fs::create_dir_all(dir)?;
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;
            fs::write(&dest, bytes)?;
        }
    }
    match fs::rename(tmp.path(), &root) {
        Ok(()) => {
            let _ = tmp.keep(); // renamed away; nothing left to clean up on drop
        }
        Err(_) if root.join("python.wasm").is_file() => {
            // Another instance won the race; `tmp` still owns its (unrenamed)
            // directory and removes it when dropped at the end of scope.
        }
        Err(e) => return Err(e).context("publishing extracted artifact"),
    }
    Ok(root)
}

/// Compile python.wasm, caching the serialized module next to the artifact,
/// keyed by wasmtime version. Cold: ~0.3 s. Warm: mmap, milliseconds.
pub fn load_module(engine: &Engine, root: &Path) -> Result<Module> {
    let cwasm = root.join(format!("python-wasmtime-{WASMTIME_VERSION}.cwasm"));
    if cwasm.is_file() {
        // SAFETY: we serialized this file ourselves into a digest-keyed
        // directory; the version key prevents cross-wasmtime loading.
        match unsafe { Module::deserialize_file(engine, &cwasm) } {
            Ok(m) => return Ok(m),
            Err(e) => tracing::warn!(error = %e, "stale cwasm; recompiling"),
        }
    }
    let wasm = fs::read(root.join("python.wasm")).context("reading python.wasm")?;
    let module = Module::new(engine, &wasm)
        .map_err(anyhow::Error::from)
        .context("compiling python.wasm")?;
    // Same collision this function's sibling (`ensure_extracted`) had: a
    // pid-only tmp name is shared by every thread of one process, so two
    // concurrent cold compiles could tear each other's `fs::write` to the
    // same path, and that file is only ever read back through `unsafe`
    // deserialize below. `tempfile_in` gives each writer its own private
    // file, so there's nothing to tear; the final `persist` rename either
    // installs a complete cwasm or is atomically superseded by another one.
    use std::io::Write as _;
    let mut tmp = tempfile::Builder::new()
        .prefix("cwasm-tmp-")
        .tempfile_in(root)
        .context("creating cwasm tempfile")?;
    tmp.write_all(&module.serialize()?)?;
    let _ = tmp.persist(&cwasm); // best-effort cache; a losing persist is harmless
    Ok(module)
}
