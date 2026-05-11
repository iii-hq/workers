use std::path::{Path, PathBuf};

pub fn session_inbox_executable() -> PathBuf {
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_session_inbox") {
        return PathBuf::from(p);
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for profile in ["debug", "release"] {
        let candidate = dir.join("target").join(profile).join("session-inbox");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "session-inbox binary not found under target/{{debug,release}}/; run `cargo build --bin session-inbox` first"
    );
}
