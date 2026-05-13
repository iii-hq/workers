use std::path::{Path, PathBuf};

pub fn session_executable() -> PathBuf {
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_session") {
        return PathBuf::from(p);
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for profile in ["debug", "release"] {
        let candidate = dir.join("target").join(profile).join("session");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "session binary not found under target/{{debug,release}}/; run `cargo build --bin session` first"
    );
}
