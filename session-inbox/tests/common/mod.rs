use std::path::{Path, PathBuf};

pub fn session_inbox_executable() -> PathBuf {
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_iii_session_inbox") {
        return PathBuf::from(p);
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for profile in ["debug", "release"] {
        let candidate = dir.join("target").join(profile).join("iii-session-inbox");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "iii-session-inbox binary not found under target/{{debug,release}}/; run `cargo build --bin iii-session-inbox` first"
    );
}
