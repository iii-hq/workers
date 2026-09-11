use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use iii_sdk::{protocol::TriggerRequest, InitOptions};
use serde_json::{json, Value};

use crate::common::world::CodeWorld;

#[given("a jailed code surface")]
async fn jailed_code_surface(world: &mut CodeWorld) {
    world.setup_direct_surface();
}

#[given("a live jailed shell code surface")]
async fn live_jailed_shell_code_surface(world: &mut CodeWorld) {
    let Ok(url) = std::env::var("III_ENGINE_WS_URL") else {
        world.soft_skip("III_ENGINE_WS_URL is not set");
        return;
    };

    let client = Arc::new(iii_sdk::register_worker(&url, InitOptions::default()));
    for _ in 0..20 {
        let probe = client.trigger(TriggerRequest {
            function_id: "coder::info".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(2_000),
        });
        match tokio::time::timeout(Duration::from_secs(3), probe).await {
            Ok(Ok(_)) => {
                world.setup_live_client(client);
                return;
            }
            Ok(Err(_)) | Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }

    world.soft_skip(format!("no live coder::* worker responded at {url}"));
}

#[given(regex = r#"^a file at "([^"]+)" with content:$"#)]
fn file_with_content(world: &mut CodeWorld, path: String, #[step] step: &Step) {
    let body = docstring(step);
    world.surface().write_file(&path, body.as_bytes());
}

#[given(regex = r#"^a file at "([^"]+)" with (\d+) bytes of content$"#)]
fn file_with_repeated_content(world: &mut CodeWorld, path: String, bytes: u64) {
    world
        .surface()
        .write_file(&path, &vec![b'x'; bytes as usize]);
}

#[given(regex = r#"^a file at "([^"]+)" with (\d+) lines of (\d+) bytes each$"#)]
fn file_with_fixed_width_lines(
    world: &mut CodeWorld,
    path: String,
    lines: u64,
    bytes_per_line: u64,
) {
    assert!(bytes_per_line > 0, "bytes_per_line must be positive");
    let line = format!(
        "{}\n",
        "x".repeat(bytes_per_line.saturating_sub(1) as usize)
    );
    let content = line.repeat(lines as usize);
    world.surface().write_file(&path, content.as_bytes());
}

#[given(regex = r#"^a binary file at "([^"]+)" with invalid UTF-8 bytes$"#)]
fn binary_file_with_invalid_utf8(world: &mut CodeWorld, path: String) {
    world
        .surface()
        .write_file(&path, &[0xff, b'a', 0xfe, b'\n']);
}

#[given(regex = r#"^a directory at "([^"]+)"$"#)]
fn directory(world: &mut CodeWorld, path: String) {
    world.surface().create_dir(&path);
}

#[given(regex = r#"^a file in the secondary root at "([^"]+)" with content:$"#)]
fn secondary_file_with_content(world: &mut CodeWorld, path: String, #[step] step: &Step) {
    let body = docstring(step);
    world.surface().write_secondary_file(&path, body.as_bytes());
}

#[given(regex = r#"^a session directory at "([^"]+)"$"#)]
fn session_directory(world: &mut CodeWorld, path: String) {
    world.surface_mut().set_session_dir(&path);
}

#[given(regex = r#"^a symlink at "([^"]+)" pointing outside the jail$"#)]
fn symlink_pointing_outside(world: &mut CodeWorld, path: String) {
    let surface = world.surface();
    let target = surface.outside_root.join("outside.txt");
    fs::write(&target, b"outside").expect("write outside symlink target");
    let link = surface.root.join(path);
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).expect("create symlink parent");
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).expect("create outside symlink");

    #[cfg(not(unix))]
    fs::write(&link, b"outside").expect("create symlink fallback file");
}

#[when(regex = r#"^I call (coder::[a-z\-]+) with payload:$"#)]
async fn call_with_payload(world: &mut CodeWorld, function_id: String, #[step] step: &Step) {
    if world.is_skipped() {
        return;
    }
    let payload = parse_payload(world, step);
    world.call_function(&function_id, payload).await;
}

#[then("the call succeeded")]
fn call_succeeded(world: &mut CodeWorld) {
    if world.is_skipped() {
        return;
    }
    world.last_ok();
}

#[then(regex = r#"^the call failed with code "([^"]+)"$"#)]
fn call_failed_with_code(world: &mut CodeWorld, code: String) {
    if world.is_skipped() {
        return;
    }
    assert_error_code(world.last_err(), &code);
}

#[then(regex = r#"^the result for "([^"]+)" succeeded$"#)]
fn result_succeeded(world: &mut CodeWorld, path: String) {
    if world.is_skipped() {
        return;
    }
    let result = find_result(world, &path);
    assert_eq!(
        result.get("success").and_then(Value::as_bool),
        Some(true),
        "expected success for {path}, got {result:#}"
    );
}

#[then(regex = r#"^the result for "([^"]+)" failed with code "([^"]+)"$"#)]
fn result_failed_with_code(world: &mut CodeWorld, path: String, code: String) {
    if world.is_skipped() {
        return;
    }
    let result = find_result(world, &path);
    assert_eq!(
        result.get("success").and_then(Value::as_bool),
        Some(false),
        "expected failure for {path}, got {result:#}"
    );
    let actual = result
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing error.code in result {result:#}"));
    assert_eq!(actual, code);
}

#[then(regex = r#"^the move from "([^"]+)" to "([^"]+)" succeeded$"#)]
fn move_succeeded(world: &mut CodeWorld, from: String, to: String) {
    if world.is_skipped() {
        return;
    }
    let result = find_move_result(world, &from, &to);
    assert_eq!(
        result.get("success").and_then(Value::as_bool),
        Some(true),
        "expected move success for {from} -> {to}, got {result:#}"
    );
}

#[then(regex = r#"^the move from "([^"]+)" to "([^"]+)" failed with code "([^"]+)"$"#)]
fn move_failed_with_code(world: &mut CodeWorld, from: String, to: String, code: String) {
    if world.is_skipped() {
        return;
    }
    let result = find_move_result(world, &from, &to);
    assert_eq!(
        result.get("success").and_then(Value::as_bool),
        Some(false),
        "expected move failure for {from} -> {to}, got {result:#}"
    );
    let actual = result
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing error.code in result {result:#}"));
    assert_eq!(actual, code);
}

#[then(regex = r#"^the file "([^"]+)" exists$"#)]
fn file_exists(world: &mut CodeWorld, path: String) {
    if world.is_skipped() {
        return;
    }
    let path = fixture_path(world, &path);
    assert!(path.exists(), "expected {} to exist", path.display());
}

#[then(regex = r#"^the file "([^"]+)" does not exist$"#)]
fn file_does_not_exist(world: &mut CodeWorld, path: String) {
    if world.is_skipped() {
        return;
    }
    let path = fixture_path(world, &path);
    assert!(!path.exists(), "expected {} not to exist", path.display());
}

#[then(regex = r#"^the file "([^"]+)" contains "([^"]*)"$"#)]
fn file_contains(world: &mut CodeWorld, path: String, needle: String) {
    if world.is_skipped() {
        return;
    }
    let path = fixture_path(world, &path);
    let actual = fs::read_to_string(&path).expect("read fixture file");
    assert!(
        actual.contains(&needle),
        "expected {} to contain {needle:?}, got {actual:?}",
        path.display()
    );
}

#[then(regex = r#"^the file "([^"]+)" equals:$"#)]
fn file_equals(world: &mut CodeWorld, path: String, #[step] step: &Step) {
    if world.is_skipped() {
        return;
    }
    let path = fixture_path(world, &path);
    let actual = fs::read_to_string(&path).expect("read fixture file");
    assert_eq!(actual, docstring(step));
}

#[then(regex = r#"^the read content equals:$"#)]
fn read_content_equals(world: &mut CodeWorld, #[step] step: &Step) {
    if world.is_skipped() {
        return;
    }
    assert_eq!(
        world.last_ok().get("content").and_then(Value::as_str),
        Some(docstring(step).as_str())
    );
}

#[then(regex = r#"^the read content contains "([^"]*)"$"#)]
fn read_content_contains(world: &mut CodeWorld, needle: String) {
    if world.is_skipped() {
        return;
    }
    let content = world
        .last_ok()
        .get("content")
        .and_then(Value::as_str)
        .expect("response content must be a string");
    assert!(
        content.contains(&needle),
        "expected read content to contain {needle:?}, got {content:?}"
    );
}

#[then(regex = r#"^the read field "([^"]+)" is (true|false)$"#)]
fn read_field_bool(world: &mut CodeWorld, field: String, expected: String) {
    if world.is_skipped() {
        return;
    }
    let actual = world
        .last_ok()
        .get(&field)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("missing bool field {field} in {:#}", world.last_ok()));
    assert_eq!(actual, expected == "true");
}

#[then(regex = r#"^the read field "([^"]+)" equals (\d+)$"#)]
fn read_field_u64(world: &mut CodeWorld, field: String, expected: u64) {
    if world.is_skipped() {
        return;
    }
    let actual = world
        .last_ok()
        .get(&field)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("missing numeric field {field} in {:#}", world.last_ok()));
    assert_eq!(actual, expected);
}

#[then(regex = r#"^the batch read result for "([^"]+)" succeeded$"#)]
fn batch_read_result_succeeded(world: &mut CodeWorld, path: String) {
    if world.is_skipped() {
        return;
    }
    let result = find_read_result(world, &path);
    assert_eq!(
        result.get("success").and_then(Value::as_bool),
        Some(true),
        "expected batch read success for {path}, got {result:#}"
    );
}

#[then(regex = r#"^the batch read result for "([^"]+)" failed with code "([^"]+)"$"#)]
fn batch_read_result_failed(world: &mut CodeWorld, path: String, code: String) {
    if world.is_skipped() {
        return;
    }
    let result = find_read_result(world, &path);
    assert_eq!(
        result.get("success").and_then(Value::as_bool),
        Some(false),
        "expected batch read failure for {path}, got {result:#}"
    );
    let actual = result
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing error.code in result {result:#}"));
    assert_eq!(actual, code);
}

#[then(regex = r#"^the batch read result for "([^"]+)" contains "([^"]*)"$"#)]
fn batch_read_result_contains(world: &mut CodeWorld, path: String, needle: String) {
    if world.is_skipped() {
        return;
    }
    let result = find_read_result(world, &path);
    let content = result
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing content in batch read result {result:#}"));
    assert!(
        content.contains(&needle),
        "expected batch content to contain {needle:?}, got {content:?}"
    );
}

#[then(regex = r#"^the info exposes (\d+) base paths$"#)]
fn info_exposes_base_paths(world: &mut CodeWorld, count: usize) {
    if world.is_skipped() {
        return;
    }
    let actual = world
        .last_ok()
        .get("base_paths")
        .and_then(Value::as_array)
        .expect("base_paths must be an array")
        .len();
    assert_eq!(actual, count);
}

#[then(regex = r#"^the info includes non-accessible glob "([^"]+)"$"#)]
fn info_includes_non_accessible_glob(world: &mut CodeWorld, glob: String) {
    if world.is_skipped() {
        return;
    }
    assert_array_contains_str(world.last_ok(), "non_accessible_globs", &glob);
}

#[then(regex = r#"^the info field "([^"]+)" equals (\d+)$"#)]
fn info_field_u64(world: &mut CodeWorld, field: String, expected: u64) {
    if world.is_skipped() {
        return;
    }
    let actual = world
        .last_ok()
        .get(&field)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("missing numeric info field {field}"));
    assert_eq!(actual, expected);
}

#[then(regex = r#"^the listing contains "([^"]+)"$"#)]
fn listing_contains(world: &mut CodeWorld, name: String) {
    if world.is_skipped() {
        return;
    }
    find_entry(world.last_ok(), &name);
}

#[then(regex = r#"^the listing marks "([^"]+)" non-accessible$"#)]
fn listing_marks_non_accessible(world: &mut CodeWorld, name: String) {
    if world.is_skipped() {
        return;
    }
    let entry = find_entry(world.last_ok(), &name);
    assert_eq!(
        entry.get("non_accessible").and_then(Value::as_bool),
        Some(true),
        "expected listing entry {name} to be non-accessible, got {entry:#}"
    );
}

#[then(regex = r#"^the listing has more pages$"#)]
fn listing_has_more_pages(world: &mut CodeWorld) {
    if world.is_skipped() {
        return;
    }
    assert_eq!(
        world.last_ok().get("has_more").and_then(Value::as_bool),
        Some(true)
    );
}

#[then(regex = r#"^the tree contains "([^"]+)"$"#)]
fn tree_contains(world: &mut CodeWorld, name: String) {
    if world.is_skipped() {
        return;
    }
    assert!(
        find_tree_node(world.last_ok().pointer("/root").expect("tree root"), &name).is_some(),
        "expected tree to contain {name}, got {:#}",
        world.last_ok()
    );
}

#[then(regex = r#"^the tree does not contain "([^"]+)"$"#)]
fn tree_does_not_contain(world: &mut CodeWorld, name: String) {
    if world.is_skipped() {
        return;
    }
    assert!(
        find_tree_node(world.last_ok().pointer("/root").expect("tree root"), &name).is_none(),
        "expected tree not to contain {name}, got {:#}",
        world.last_ok()
    );
}

#[then(regex = r#"^the tree marks "([^"]+)" truncated for "([^"]+)"$"#)]
fn tree_marks_truncated(world: &mut CodeWorld, name: String, reason: String) {
    if world.is_skipped() {
        return;
    }
    let node = find_tree_node(world.last_ok().pointer("/root").expect("tree root"), &name)
        .unwrap_or_else(|| panic!("missing tree node {name} in {:#}", world.last_ok()));
    let actual = node
        .pointer("/truncated/reason")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing truncation reason in {node:#}"));
    assert_eq!(actual, reason);
}

#[then(regex = r#"^the search has a content match for "([^"]+)" at line (\d+)$"#)]
fn search_has_content_match(world: &mut CodeWorld, path: String, line: u64) {
    if world.is_skipped() {
        return;
    }
    let matches = world
        .last_ok()
        .get("content_matches")
        .and_then(Value::as_array)
        .expect("content_matches must be an array");
    assert!(
        matches.iter().any(|m| {
            m.get("path")
                .and_then(Value::as_str)
                .is_some_and(|actual| world.path_matches(actual, &path))
                && m.get("line").and_then(Value::as_u64) == Some(line)
        }),
        "missing content match for {path}:{line}, got {matches:#?}"
    );
}

#[then(regex = r#"^the search has a path match for "([^"]+)"$"#)]
fn search_has_path_match(world: &mut CodeWorld, path: String) {
    if world.is_skipped() {
        return;
    }
    let matches = world
        .last_ok()
        .get("path_matches")
        .and_then(Value::as_array)
        .expect("path_matches must be an array");
    assert!(
        matches.iter().any(|m| {
            m.get("path")
                .and_then(Value::as_str)
                .is_some_and(|actual| world.path_matches(actual, &path))
        }),
        "missing path match for {path}, got {matches:#?}"
    );
}

#[then(regex = r#"^the search has no path match for "([^"]+)"$"#)]
fn search_has_no_path_match(world: &mut CodeWorld, path: String) {
    if world.is_skipped() {
        return;
    }
    let matches = world
        .last_ok()
        .get("path_matches")
        .and_then(Value::as_array)
        .expect("path_matches must be an array");
    assert!(
        matches.iter().all(|m| {
            !m.get("path")
                .and_then(Value::as_str)
                .is_some_and(|actual| world.path_matches(actual, &path))
        }),
        "unexpected path match for {path}, got {matches:#?}"
    );
}

#[then(regex = r#"^the search truncated flag is (true|false)$"#)]
fn search_truncated_flag(world: &mut CodeWorld, expected: String) {
    if world.is_skipped() {
        return;
    }
    assert_eq!(
        world.last_ok().get("truncated").and_then(Value::as_bool),
        Some(expected == "true")
    );
}

fn parse_payload(world: &CodeWorld, step: &Step) -> Value {
    let expanded = world.expand(docstring(step).trim());
    serde_json::from_str(&expanded)
        .unwrap_or_else(|err| panic!("invalid JSON payload {expanded:?}: {err}"))
}

fn docstring(step: &Step) -> String {
    let raw = step
        .docstring()
        .unwrap_or_else(|| panic!("step `{}` requires a docstring", step.value))
        .to_string();
    raw.strip_prefix('\n').unwrap_or(&raw).to_string()
}

fn fixture_path(world: &CodeWorld, input: &str) -> PathBuf {
    let expanded = world.expand(input);
    let path = Path::new(&expanded);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        world.surface().root.join(path)
    }
}

fn find_result<'a>(world: &'a CodeWorld, path: &str) -> &'a Value {
    let results = world
        .last_ok()
        .get("results")
        .and_then(Value::as_array)
        .expect("response results must be an array");
    results
        .iter()
        .find(|result| {
            result
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|actual| world.path_matches(actual, path))
        })
        .unwrap_or_else(|| panic!("missing result for {path}, got {results:#?}"))
}

fn find_read_result<'a>(world: &'a CodeWorld, path: &str) -> &'a Value {
    let results = world
        .last_ok()
        .get("results")
        .and_then(Value::as_array)
        .expect("read-file batch response results must be an array");
    results
        .iter()
        .find(|result| {
            result
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|actual| world.path_matches(actual, path))
        })
        .unwrap_or_else(|| panic!("missing batch read result for {path}, got {results:#?}"))
}

fn find_move_result<'a>(world: &'a CodeWorld, from: &str, to: &str) -> &'a Value {
    let results = world
        .last_ok()
        .get("results")
        .and_then(Value::as_array)
        .expect("move response results must be an array");
    results
        .iter()
        .find(|result| {
            let from_matches = result
                .get("from")
                .and_then(Value::as_str)
                .is_some_and(|actual| world.path_matches(actual, from));
            let to_matches = result
                .get("to")
                .and_then(Value::as_str)
                .is_some_and(|actual| world.path_matches(actual, to));
            from_matches && to_matches
        })
        .unwrap_or_else(|| panic!("missing move result for {from} -> {to}, got {results:#?}"))
}

fn assert_error_code(err: &str, code: &str) {
    assert!(
        err.contains(code),
        "expected error to contain code {code}, got {err:?}"
    );
}

fn assert_array_contains_str(value: &Value, field: &str, expected: &str) {
    let values = value
        .get(field)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{field} must be an array in {value:#}"));
    assert!(
        values.iter().any(|value| value.as_str() == Some(expected)),
        "expected {field} to contain {expected:?}, got {values:#?}"
    );
}

fn find_entry<'a>(value: &'a Value, name: &str) -> &'a Value {
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .expect("entries must be an array");
    entries
        .iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
        .unwrap_or_else(|| panic!("missing listing entry {name}, got {entries:#?}"))
}

fn find_tree_node<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    if node.get("name").and_then(Value::as_str) == Some(name) {
        return Some(node);
    }

    node.get("children")
        .and_then(Value::as_array)
        .and_then(|children| {
            children
                .iter()
                .find_map(|child| find_tree_node(child, name))
        })
}
