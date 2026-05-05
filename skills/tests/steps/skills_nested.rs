//! Step defs for tests/features/skills_nested.feature.
//!
//! Exercises the v2 URI scheme: arbitrary-depth state-backed skills
//! (`iii://a/b/c/...`), the `fn`-reserved-first-segment rule on
//! `skills::register`, the path-mapped section URI shape
//! (`iii://fn/a/b/c` → `a::b::c`), and `skill::fetch` composition
//! across depths.

use cucumber::{given, then, when};
use iii_sdk::{IIIError, RegisterFunction, TriggerRequest};
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_OK: &str = "skills_nested_last_ok";
const LAST_ERR: &str = "skills_nested_last_err";
const LAST_READ: &str = "skills_nested_last_read";
const LAST_LIST: &str = "skills_nested_last_list";
const LAST_INDEX: &str = "skills_nested_last_index";
const LAST_FETCH: &str = "skills_nested_last_fetch";
const DEEP_FN: &str = "skills_nested_deep_fn";

fn nested_id_slot(id: &str) -> String {
    format!("skills_nested_id_{id}")
}

fn seeded_full(world: &IiiSkillsWorld, base_id: &str) -> String {
    world
        .stash
        .get(&nested_id_slot(base_id))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_default()
}

/// Map a base id like `parent/sub` to its scenario-scoped form by
/// scoping ONLY the first segment so the slashed structure of the
/// remaining segments is preserved. e.g. `parent/sub` becomes
/// `parent-<unique>/sub`.
fn scoped_nested(world: &IiiSkillsWorld, base_id: &str) -> String {
    match base_id.split_once('/') {
        Some((root, rest)) => format!("{}/{}", world.scoped_id(root), rest),
        None => world.scoped_id(base_id),
    }
}

async fn trigger(
    world: &mut IiiSkillsWorld,
    function_id: &str,
    payload: Value,
    ok_slot: &str,
    err_slot: &str,
) {
    world.stash.remove(ok_slot);
    world.stash.remove(err_slot);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        Ok(v) => {
            world.stash.insert(ok_slot.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(err_slot.into(), Value::String(e.to_string()));
        }
    }
}

// ── seeding nested skills ──────────────────────────────────────────────

#[given(regex = r#"^a nested skill at "([^"]+)" with body "([^"]*)"$"#)]
async fn seed_nested(world: &mut IiiSkillsWorld, base_id: String, body: String) {
    let scoped = scoped_nested(world, &base_id);
    world
        .stash
        .insert(nested_id_slot(&base_id), Value::String(scoped.clone()));
    trigger(
        world,
        "skills::register",
        json!({ "id": scoped, "skill": body }),
        LAST_OK,
        LAST_ERR,
    )
    .await;
    if world.stash.contains_key(LAST_ERR) {
        let err = world
            .stash
            .get(LAST_ERR)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        panic!("seeding nested skill {scoped:?} failed: {err}");
    }
}

// ── reading nested skills ──────────────────────────────────────────────

#[when(regex = r#"^I read the nested skill at "([^"]+)"$"#)]
async fn read_nested(world: &mut IiiSkillsWorld, base_id: String) {
    let scoped = seeded_full(world, &base_id);
    let uri = format!("iii://{scoped}");
    trigger(
        world,
        "skills::resources-read",
        json!({ "uri": uri }),
        LAST_READ,
        LAST_ERR,
    )
    .await;
}

#[then(regex = r#"^the nested read mime is "([^"]+)"$"#)]
fn nested_read_mime(world: &mut IiiSkillsWorld, want: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_READ).expect("no read recorded");
    let mime = v["contents"][0]["mimeType"].as_str().unwrap_or("");
    assert_eq!(mime, want, "mime mismatch in: {v}");
}

#[then(regex = r#"^the nested read text contains "([^"]+)"$"#)]
fn nested_read_text_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_READ).expect("no read recorded");
    let text = v["contents"][0]["text"].as_str().unwrap_or("");
    assert!(text.contains(&needle), "missing {needle:?} in: {text}");
}

// ── skills::list (lex sort confirms parent precedes children) ──────────

#[when("I list the seeded nested ids")]
async fn list_nested(world: &mut IiiSkillsWorld) {
    trigger(world, "skills::list", json!({}), LAST_LIST, LAST_ERR).await;
}

#[then(regex = r#"^the listing is sorted with parent before children for "([^"]+)"$"#)]
fn listing_sorted_for_root(world: &mut IiiSkillsWorld, root_base: String) {
    if world.iii.is_none() {
        return;
    }
    let parent = seeded_full(world, &root_base);
    let v = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = v["skills"].as_array().expect("missing .skills array");
    let ids: Vec<&str> = arr.iter().filter_map(|e| e["id"].as_str()).collect();
    let parent_pos = ids
        .iter()
        .position(|id| *id == parent)
        .unwrap_or_else(|| panic!("parent {parent:?} not in listing: {ids:?}"));
    // Every entry whose id starts with "<parent>/" must appear AFTER parent.
    let prefix = format!("{parent}/");
    for (i, id) in ids.iter().enumerate() {
        if id.starts_with(&prefix) {
            assert!(
                i > parent_pos,
                "child {id:?} at pos {i} appears before parent {parent:?} at pos {parent_pos}; ids={ids:?}"
            );
        }
    }
    // Confirm the listing is globally sorted, which is the property
    // the index renderer relies on.
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "listing not sorted by id: {ids:?}");
}

// ── iii://skills index render ──────────────────────────────────────────

#[when("I read iii://skills via fetch")]
async fn fetch_index(world: &mut IiiSkillsWorld) {
    trigger(
        world,
        "skills::resources-read",
        json!({ "uri": "iii://skills" }),
        LAST_INDEX,
        LAST_ERR,
    )
    .await;
}

#[then(regex = r#"^the index has the seeded entry "([^"]+)" indented by (\d+) spaces$"#)]
fn index_indent(world: &mut IiiSkillsWorld, base_id: String, indent_spaces: usize) {
    if world.iii.is_none() {
        return;
    }
    let scoped = seeded_full(world, &base_id);
    let v = world.stash.get(LAST_INDEX).expect("no index recorded");
    let text = v["contents"][0]["text"].as_str().unwrap_or("");
    let needle_indent = " ".repeat(indent_spaces);
    let bullet_prefix = format!("{needle_indent}- [`{scoped}`](iii://{scoped})");
    assert!(
        text.lines().any(|l| l.starts_with(&bullet_prefix)),
        "missing line starting with {bullet_prefix:?} in:\n{text}"
    );
}

// ── reuse of skills_register's failure-path assertions ─────────────────
//
// The `fn`-reserved scenarios share the existing skills_register step
// helpers (`I register a skill with id "<id>" and body "<body>"`,
// `the skills::register call fails`, `the skills::register error
// mentions "<needle>"`) — so no new step defs are needed for them.

// ── path-mapped section URIs ───────────────────────────────────────────

#[given("a deep section function whose id has 2 scope segments")]
async fn seed_deep_2(world: &mut IiiSkillsWorld) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let fn_id = format!("bdd::deep-{}", world.unique_id);
    world
        .stash
        .insert(DEEP_FN.into(), Value::String(fn_id.clone()));
    iii.register_function(
        RegisterFunction::new_async(fn_id.clone(), |_input: Value| async move {
            Ok::<_, IIIError>(Value::String("# deep-section\nlevel 2".into()))
        })
        .description("bdd: deep section, 2 segments"),
    );
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
}

#[given("a deep section function whose id has 3 scope segments")]
async fn seed_deep_3(world: &mut IiiSkillsWorld) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    // Three `::`-separated parts. We use kebab-case so each segment
    // satisfies validate_id_segment and the URI builder produces a
    // clean `iii://fn/a/b/c` path.
    let fn_id = format!("bdd::deep::triple-{}", world.unique_id);
    world
        .stash
        .insert(DEEP_FN.into(), Value::String(fn_id.clone()));
    iii.register_function(
        RegisterFunction::new_async(fn_id.clone(), |_input: Value| async move {
            Ok::<_, IIIError>(Value::String("# triple-section\nthree levels".into()))
        })
        .description("bdd: deep section, 3 segments"),
    );
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
}

#[when("I read the section URI for the seeded deep function")]
async fn read_deep_section(world: &mut IiiSkillsWorld) {
    let fn_id = world
        .stash
        .get(DEEP_FN)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let path = fn_id.replace("::", "/");
    let uri = format!("iii://fn/{path}");
    trigger(
        world,
        "skills::resources-read",
        json!({ "uri": uri }),
        LAST_READ,
        LAST_ERR,
    )
    .await;
}

// ── skill::fetch composition ───────────────────────────────────────────

#[when("I fetch the seeded nested URIs at depths 1, 2, 3")]
async fn fetch_three_depths(world: &mut IiiSkillsWorld) {
    let one = seeded_full(world, "fetched");
    let two = seeded_full(world, "fetched/sub");
    let three = seeded_full(world, "fetched/sub/leaf");
    let uris = vec![
        format!("iii://{one}"),
        format!("iii://{two}"),
        format!("iii://{three}"),
    ];
    trigger(
        world,
        "skill::fetch",
        json!({ "uris": uris }),
        LAST_FETCH,
        LAST_ERR,
    )
    .await;
}

fn fetch_text(world: &IiiSkillsWorld) -> String {
    world
        .stash
        .get(LAST_FETCH)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

#[then(regex = r#"^the fetched text contains "([^"]+)"$"#)]
fn fetched_text_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let text = fetch_text(world);
    assert!(text.contains(&needle), "missing {needle:?} in: {text}");
}

#[then(regex = r#"^the fetched text has (\d+) sections joined by "([^"]+)"$"#)]
fn fetched_section_count(world: &mut IiiSkillsWorld, want: usize, sep_inner: String) {
    if world.iii.is_none() {
        return;
    }
    let text = fetch_text(world);
    // Sections are joined with `\n\n---\n\n`; the visible delimiter
    // inside the join is just `---`.
    let delim = format!("\n\n{sep_inner}\n\n");
    let sep_count = text.matches(delim.as_str()).count();
    let got = if text.is_empty() { 0 } else { sep_count + 1 };
    assert_eq!(
        got, want,
        "expected {want} sections joined by {sep_inner:?}; got {got}. text={text:?}"
    );
}
