//! @pure steps asserting on the FsStore's on-disk JSONL files and
//! simulating worker restarts (fresh store over the same directory).

use cucumber::{then, when};

use session_manager::store::encode_session_id;

use crate::common::world::SessionWorld;

fn jsonl_files(world: &SessionWorld) -> Vec<String> {
    let mut names = Vec::new();
    for dent in std::fs::read_dir(&world.fs_dir).expect("read scenario data_dir") {
        let name = dent.expect("dir entry").file_name();
        if let Some(name) = name.to_str() {
            if name.ends_with(".jsonl") {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names
}

#[then(regex = r"^the session store directory contains (\d+) session files?$")]
async fn dir_contains(world: &mut SessionWorld, expected: usize) {
    let files = jsonl_files(world);
    assert_eq!(
        files.len(),
        expected,
        "expected {expected} session files, found {}: {files:?}",
        files.len()
    );
}

#[then(regex = r#"^a session file exists for "([^"]+)"$"#)]
async fn file_exists(world: &mut SessionWorld, session_id: String) {
    let session_id = world.substitute(&session_id);
    let path = world
        .fs_dir
        .join(format!("{}.jsonl", encode_session_id(&session_id)));
    assert!(
        path.exists(),
        "expected a session file for `{session_id}` at {}",
        path.display()
    );
}

#[then(regex = r#"^no session file exists for "([^"]+)"$"#)]
async fn file_absent(world: &mut SessionWorld, session_id: String) {
    let session_id = world.substitute(&session_id);
    let path = world
        .fs_dir
        .join(format!("{}.jsonl", encode_session_id(&session_id)));
    assert!(
        !path.exists(),
        "expected no session file for `{session_id}`, but {} exists",
        path.display()
    );
}

#[when("the worker restarts with the same data directory")]
async fn reopen_store(world: &mut SessionWorld) {
    world.reopen_fs();
}
