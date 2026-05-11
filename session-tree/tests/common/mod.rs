use std::path::{Path, PathBuf};

pub fn session_tree_executable() -> PathBuf {
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_iii-session-tree") {
        return PathBuf::from(p);
    }
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_session_tree") {
        return PathBuf::from(p);
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for profile in ["debug", "release"] {
        let candidate = dir.join("target").join(profile).join("iii-session-tree");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "iii-session-tree binary not found under target/{{debug,release}}/; run `cargo build --bin iii-session-tree` first"
    );
}
