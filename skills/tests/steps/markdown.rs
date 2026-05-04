//! Step defs for tests/features/markdown.feature.
//!
//! Pure, engine-free unit tests of the markdown / URI / validation
//! helpers that the skills module exposes. Runs under `--tags @pure`
//! on CI hosts without `iii`.

use cucumber::{given, then, when};
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;
use iii_skills::functions::prompts;
use iii_skills::functions::skills::{
    self, extract_description, extract_title, is_always_hidden, normalize_function_output,
    parse_uri, truncate_chars, validate_id, ParsedUri,
};

const BUF_INPUT: &str = "md_input";
const BUF_OUTPUT: &str = "md_output";
const BUF_ERR: &str = "md_error";
const BUF_MIME: &str = "md_mime";

// ── title / description ─────────────────────────────────────────────────

#[when(regex = r#"^I extract the title from:$"#)]
fn when_extract_title(world: &mut IiiSkillsWorld, step: &cucumber::gherkin::Step) {
    let md = step.docstring.clone().unwrap_or_default();
    let got = extract_title(&md)
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);
    world.stash.insert(BUF_OUTPUT.into(), got);
}

#[then(regex = r#"^the extracted title is "([^"]+)"$"#)]
fn then_title_is(world: &mut IiiSkillsWorld, want: String) {
    let got = world.stash.get(BUF_OUTPUT).unwrap();
    assert_eq!(got.as_str(), Some(want.as_str()));
}

#[then("there is no extracted title")]
fn then_no_title(world: &mut IiiSkillsWorld) {
    let got = world.stash.get(BUF_OUTPUT).unwrap();
    assert!(got.is_null(), "expected Null; got {got}");
}

#[when(regex = r#"^I extract the description from:$"#)]
fn when_extract_desc(world: &mut IiiSkillsWorld, step: &cucumber::gherkin::Step) {
    let md = step.docstring.clone().unwrap_or_default();
    let got = extract_description(&md)
        .map(Value::String)
        .unwrap_or(Value::Null);
    world.stash.insert(BUF_OUTPUT.into(), got);
}

#[then(regex = r#"^the extracted description is "([^"]+)"$"#)]
fn then_desc_is(world: &mut IiiSkillsWorld, want: String) {
    let got = world.stash.get(BUF_OUTPUT).unwrap();
    assert_eq!(got.as_str(), Some(want.as_str()));
}

#[then("there is no extracted description")]
fn then_no_desc(world: &mut IiiSkillsWorld) {
    let got = world.stash.get(BUF_OUTPUT).unwrap();
    assert!(got.is_null(), "expected Null; got {got}");
}

// ── truncate_chars ──────────────────────────────────────────────────────

#[given(regex = r#"^the string "([^"]*)" repeated (\d+) times$"#)]
fn given_repeated(world: &mut IiiSkillsWorld, s: String, n: usize) {
    world
        .stash
        .insert(BUF_INPUT.into(), Value::String(s.repeat(n)));
}

#[when(regex = r#"^I truncate it to (\d+) chars$"#)]
fn when_truncate(world: &mut IiiSkillsWorld, n: usize) {
    let input = world.stash.get(BUF_INPUT).and_then(|v| v.as_str()).unwrap();
    let got = truncate_chars(input, n);
    world.stash.insert(BUF_OUTPUT.into(), Value::String(got));
}

#[then(regex = r#"^the truncated string has (\d+) chars$"#)]
fn then_truncated_len(world: &mut IiiSkillsWorld, want: usize) {
    let got = world
        .stash
        .get(BUF_OUTPUT)
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(got.chars().count(), want, "{got:?}");
}

#[then("the truncated string ends with ...")]
fn then_truncated_ends_ellipsis(world: &mut IiiSkillsWorld) {
    let got = world
        .stash
        .get(BUF_OUTPUT)
        .and_then(|v| v.as_str())
        .unwrap();
    assert!(got.ends_with("..."), "{got:?}");
}

// ── parse_uri ───────────────────────────────────────────────────────────

#[when(regex = r#"^I parse the URI "([^"]*)"$"#)]
fn when_parse_uri(world: &mut IiiSkillsWorld, uri: String) {
    world.stash.remove(BUF_OUTPUT);
    world.stash.remove(BUF_ERR);
    match parse_uri(&uri) {
        Ok(ParsedUri::Index) => {
            world
                .stash
                .insert(BUF_OUTPUT.into(), json!({ "kind": "index" }));
        }
        Ok(ParsedUri::Skill(id)) => {
            world
                .stash
                .insert(BUF_OUTPUT.into(), json!({ "kind": "skill", "id": id }));
        }
        Ok(ParsedUri::Section {
            skill_id,
            function_id,
        }) => {
            world.stash.insert(
                BUF_OUTPUT.into(),
                json!({ "kind": "section", "skill_id": skill_id, "function_id": function_id }),
            );
        }
        Err(e) => {
            world.stash.insert(BUF_ERR.into(), Value::String(e));
        }
    }
}

#[then(regex = r#"^parse_uri returns the index shape$"#)]
fn then_parse_index(world: &mut IiiSkillsWorld) {
    let v = world.stash.get(BUF_OUTPUT).unwrap();
    assert_eq!(v["kind"].as_str(), Some("index"), "{v}");
}

#[then(regex = r#"^parse_uri returns a skill with id "([^"]+)"$"#)]
fn then_parse_skill(world: &mut IiiSkillsWorld, id: String) {
    let v = world.stash.get(BUF_OUTPUT).unwrap();
    assert_eq!(v["kind"].as_str(), Some("skill"));
    assert_eq!(v["id"].as_str(), Some(id.as_str()));
}

#[then(regex = r#"^parse_uri returns a section with skill "([^"]+)" and function "([^"]+)"$"#)]
fn then_parse_section(world: &mut IiiSkillsWorld, skill: String, function: String) {
    let v = world.stash.get(BUF_OUTPUT).unwrap();
    assert_eq!(v["kind"].as_str(), Some("section"));
    assert_eq!(v["skill_id"].as_str(), Some(skill.as_str()));
    assert_eq!(v["function_id"].as_str(), Some(function.as_str()));
}

#[then("parse_uri fails")]
fn then_parse_fails(world: &mut IiiSkillsWorld) {
    assert!(
        world.stash.contains_key(BUF_ERR),
        "expected error; got success"
    );
}

// ── validate_id / validate_name ─────────────────────────────────────────

#[when(regex = r#"^I validate the skill id "([^"]*)"$"#)]
fn when_validate_id(world: &mut IiiSkillsWorld, id: String) {
    world.stash.remove(BUF_ERR);
    match validate_id(&id) {
        Ok(()) => {
            world.stash.insert(BUF_OUTPUT.into(), Value::Bool(true));
        }
        Err(e) => {
            world.stash.insert(BUF_ERR.into(), Value::String(e));
        }
    }
}

#[when(regex = r#"^I validate a skill id of (\d+) lowercase letters$"#)]
fn when_validate_id_sized(world: &mut IiiSkillsWorld, n: usize) {
    let id = "a".repeat(n);
    when_validate_id(world, id)
}

#[then("the skill id validation succeeds")]
fn then_validate_id_ok(world: &mut IiiSkillsWorld) {
    assert!(
        world.stash.contains_key(BUF_OUTPUT),
        "error: {:?}",
        world.stash.get(BUF_ERR)
    );
}

#[then("the skill id validation fails")]
fn then_validate_id_err(world: &mut IiiSkillsWorld) {
    assert!(
        world.stash.contains_key(BUF_ERR),
        "expected error; got {:?}",
        world.stash.get(BUF_OUTPUT)
    );
}

#[when(regex = r#"^I validate the prompt name "([^"]*)"$"#)]
fn when_validate_prompt(world: &mut IiiSkillsWorld, name: String) {
    world.stash.remove(BUF_ERR);
    match prompts::validate_name(&name) {
        Ok(()) => {
            world.stash.insert(BUF_OUTPUT.into(), Value::Bool(true));
        }
        Err(e) => {
            world.stash.insert(BUF_ERR.into(), Value::String(e));
        }
    }
}

#[then("the prompt name validation succeeds")]
fn then_validate_prompt_ok(world: &mut IiiSkillsWorld) {
    assert!(
        world.stash.contains_key(BUF_OUTPUT),
        "error: {:?}",
        world.stash.get(BUF_ERR)
    );
}

#[then("the prompt name validation fails")]
fn then_validate_prompt_err(world: &mut IiiSkillsWorld) {
    assert!(
        world.stash.contains_key(BUF_ERR),
        "expected error; got success"
    );
}

#[when("I validate a duplicate argument pair")]
fn when_validate_arg_dup(world: &mut IiiSkillsWorld) {
    let args = vec![
        prompts::PromptArgument {
            name: "to".into(),
            description: None,
            required: true,
        },
        prompts::PromptArgument {
            name: "to".into(),
            description: None,
            required: false,
        },
    ];
    match prompts::validate_arguments(&args) {
        Ok(()) => {
            world.stash.insert(BUF_OUTPUT.into(), Value::Bool(true));
        }
        Err(e) => {
            world.stash.insert(BUF_ERR.into(), Value::String(e));
        }
    }
}

#[when("I validate an empty argument name")]
fn when_validate_arg_empty(world: &mut IiiSkillsWorld) {
    let args = vec![prompts::PromptArgument {
        name: "".into(),
        description: None,
        required: true,
    }];
    match prompts::validate_arguments(&args) {
        Ok(()) => {
            world.stash.insert(BUF_OUTPUT.into(), Value::Bool(true));
        }
        Err(e) => {
            world.stash.insert(BUF_ERR.into(), Value::String(e));
        }
    }
}

#[then("argument validation fails")]
fn then_arg_validation_fails(world: &mut IiiSkillsWorld) {
    assert!(
        world.stash.contains_key(BUF_ERR),
        "expected error; got success"
    );
}

// ── normalize_function_output ───────────────────────────────────────────

#[when("I normalize a markdown string output")]
fn when_norm_string(world: &mut IiiSkillsWorld) {
    let (text, mime) = normalize_function_output(Value::String("hello".into()));
    world.stash.insert(BUF_OUTPUT.into(), Value::String(text));
    world
        .stash
        .insert(BUF_MIME.into(), Value::String(mime.into()));
}

#[when("I normalize a {content} output")]
fn when_norm_content(world: &mut IiiSkillsWorld) {
    let (text, mime) = normalize_function_output(json!({ "content": "hi" }));
    world.stash.insert(BUF_OUTPUT.into(), Value::String(text));
    world
        .stash
        .insert(BUF_MIME.into(), Value::String(mime.into()));
}

#[when("I normalize an arbitrary JSON output")]
fn when_norm_json(world: &mut IiiSkillsWorld) {
    let (text, mime) = normalize_function_output(json!({ "x": 1 }));
    world.stash.insert(BUF_OUTPUT.into(), Value::String(text));
    world
        .stash
        .insert(BUF_MIME.into(), Value::String(mime.into()));
}

#[then(regex = r#"^the normalized mime is "([^"]+)"$"#)]
fn then_norm_mime(world: &mut IiiSkillsWorld, want: String) {
    let got = world.stash.get(BUF_MIME).and_then(|v| v.as_str()).unwrap();
    assert_eq!(got, want);
}

#[then(regex = r#"^the normalized text is "([^"]+)"$"#)]
fn then_norm_text(world: &mut IiiSkillsWorld, want: String) {
    let got = world
        .stash
        .get(BUF_OUTPUT)
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(got, want);
}

#[then("the normalized text is JSON containing \"x\"")]
fn then_norm_text_has_x(world: &mut IiiSkillsWorld) {
    let got = world
        .stash
        .get(BUF_OUTPUT)
        .and_then(|v| v.as_str())
        .unwrap();
    assert!(got.contains("\"x\""), "{got}");
}

// ── normalize_prompt_output ─────────────────────────────────────────────

#[when("I normalize a prompt output string")]
fn when_prompt_norm_string(world: &mut IiiSkillsWorld) {
    match prompts::normalize_prompt_output(Value::String("hello".into())) {
        Ok(v) => {
            world.stash.insert(BUF_OUTPUT.into(), v);
        }
        Err(e) => {
            world.stash.insert(BUF_ERR.into(), Value::String(e));
        }
    }
}

#[when("I normalize a prompt output with a messages array")]
fn when_prompt_norm_messages(world: &mut IiiSkillsWorld) {
    match prompts::normalize_prompt_output(json!({
        "messages": [
            { "role": "user", "content": { "type": "text", "text": "a" } },
            { "role": "assistant", "content": { "type": "text", "text": "b" } }
        ]
    })) {
        Ok(v) => {
            world.stash.insert(BUF_OUTPUT.into(), v);
        }
        Err(e) => {
            world.stash.insert(BUF_ERR.into(), Value::String(e));
        }
    }
}

#[when("I normalize a prompt output of unsupported shape")]
fn when_prompt_norm_bad(world: &mut IiiSkillsWorld) {
    match prompts::normalize_prompt_output(json!({ "x": 1 })) {
        Ok(v) => {
            world.stash.insert(BUF_OUTPUT.into(), v);
        }
        Err(e) => {
            world.stash.insert(BUF_ERR.into(), Value::String(e));
        }
    }
}

#[then(regex = r#"^the normalized messages length is (\d+)$"#)]
fn then_norm_messages_len(world: &mut IiiSkillsWorld, want: usize) {
    let v = world.stash.get(BUF_OUTPUT).unwrap();
    assert_eq!(v.as_array().map(|a| a.len()).unwrap_or(0), want, "{v}");
}

#[then("prompt normalization fails")]
fn then_norm_fails(world: &mut IiiSkillsWorld) {
    assert!(
        world.stash.contains_key(BUF_ERR),
        "expected error; got success"
    );
}

// ── is_always_hidden ────────────────────────────────────────────────────

#[when(regex = r#"^I check the hard floor for "([^"]*)"$"#)]
fn when_check_floor(world: &mut IiiSkillsWorld, fid: String) {
    world
        .stash
        .insert(BUF_OUTPUT.into(), Value::Bool(is_always_hidden(&fid)));
}

#[then("the function id is hard-floored")]
fn then_floored(world: &mut IiiSkillsWorld) {
    let got = world
        .stash
        .get(BUF_OUTPUT)
        .and_then(|v| v.as_bool())
        .unwrap();
    assert!(got, "expected hard-floored");
}

#[then("the function id is not hard-floored")]
fn then_not_floored(world: &mut IiiSkillsWorld) {
    let got = world
        .stash
        .get(BUF_OUTPUT)
        .and_then(|v| v.as_bool())
        .unwrap();
    assert!(!got, "expected NOT hard-floored");
}

// Used by a handful of scenarios that just need the module loaded.
#[allow(dead_code)]
fn _force_imports() {
    let _ = skills::list_templates();
}
