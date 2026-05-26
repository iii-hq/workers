//! Security-specific fixture steps. Most of the path-jail invariants
//! (dot-dot escape, absolute paths) need no fixture — the dispatcher
//! and `the call failed with code "..."` Then in [`common`](super::common)
//! are enough. This module adds the symlink-escape fixture, which
//! requires Unix `symlink` syscalls; the matching scenario is tagged
//! `@unix` so non-Unix runs can filter it out.

use cucumber::given;

use crate::common::world::CoderWorld;

#[given(regex = r#"^a symlink at "([^"]+)" pointing to a path outside base$"#)]
async fn given_symlink_escape(world: &mut CoderWorld, rel: String) {
    if world.iii.is_none() {
        return;
    }
    let Some(base) = world.base_path.as_ref() else {
        return;
    };
    create_escape_symlink(base, &rel);
}

#[cfg(unix)]
fn create_escape_symlink(base: &std::path::Path, rel: &str) {
    let outside = tempfile::tempdir().expect("escape tempdir").keep();
    let target = outside.join("secret.txt");
    std::fs::write(&target, b"x").expect("write escape target");
    std::os::unix::fs::symlink(&target, base.join(rel)).expect("create symlink");
}

#[cfg(not(unix))]
fn create_escape_symlink(_base: &std::path::Path, _rel: &str) {
    panic!(
        "symlink-escape scenarios are Unix-only; tag with @unix and \
         filter on non-Unix hosts (cargo test --test bdd -- --tags \
         \"not @unix\")"
    );
}
