#[allow(dead_code)]
mod common;
mod replay_support;

use std::collections::BTreeMap;
use std::path::Path;

use context_manager::functions::assemble::AssembleOptions;
use context_manager::types::Model;
use replay_support::{
    compare_decay_four, compare_directory, parse_session_lines, prefix_edits, read_directory,
    ReplayHistory,
};
use serde_json::{json, Value};

#[test]
fn keeps_the_last_revision_in_first_occurrence_order() {
    let lines = [
        r#"{"type":"entry","entry":{"kind":"message","id":"first","revision":0,"message":{"role":"user","content":[{"type":"text","text":"old"}],"timestamp":1}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"second","revision":0,"message":{"role":"user","content":[{"type":"text","text":"second"}],"timestamp":2}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"first","revision":1,"message":{"role":"user","content":[{"type":"text","text":"new"}],"timestamp":3}}}"#,
    ];

    let history = parse_session_lines("synthetic.jsonl", lines).expect("valid session records");

    assert_eq!(history.ids(), ["first", "second"]);
    assert_eq!(history.revisions(), [1, 0]);
    assert_eq!(history.text_at(0), Some("new"));
}

#[test]
fn finds_only_actual_user_turn_endpoints() {
    let lines = [
        r#"{"type":"entry","entry":{"kind":"message","id":"one","revision":0,"message":{"role":"user","content":[{"type":"text","text":"first"}],"timestamp":1}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"inline","revision":0,"message":{"role":"user","content":[{"type":"function_result","function_call_id":"call","content":[{"type":"text","text":"result"}]}],"timestamp":2}}}"#,
        r#"{"type":"entry","entry":{"kind":"custom","id":"notice","revision":0,"data":{}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"assistant","revision":0,"message":{"role":"assistant","content":[],"stop_reason":"end","model":"test","provider":"test","timestamp":3}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"two","revision":0,"message":{"role":"user","content":[{"type":"text","text":"second"}],"timestamp":4}}}"#,
    ];

    let history = parse_session_lines("synthetic.jsonl", lines).expect("valid session records");

    assert_eq!(history.user_turn_endpoints(), [0, 3]);
}

#[test]
fn rejects_malformed_records_without_echoing_their_contents() {
    let result = parse_session_lines(
        "synthetic.jsonl",
        [
            r#"{"type":"entry","entry":{"kind":"message","id":"one","revision":0,"message":{"role":"user","content":"private transcript text"}}}"#,
        ],
    );
    let Err(error) = result else {
        panic!("invalid message shape must be rejected");
    };

    assert_eq!(error, "synthetic.jsonl:1: message entry message is invalid");
    assert!(!error.contains("private transcript text"));
}

/// Decay is a prune rule, and prune runs only over budget: under this
/// generous budget the history goes out verbatim, decay or not, so the
/// provider prompt cache keeps every earlier request's prefix.
#[tokio::test]
async fn decay_leaves_an_under_budget_history_verbatim() {
    let mut lines = Vec::new();
    for turn in 0..130 {
        lines.push(
            json!({
                "type": "entry",
                "entry": {
                    "kind": "message",
                    "id": format!("user-{turn}"),
                    "revision": 0,
                    "message": {
                        "role": "user",
                        "content": [{ "type": "text", "text": "continue" }],
                        "timestamp": turn * 2
                    }
                }
            })
            .to_string(),
        );
        if turn < 129 {
            lines.push(
                json!({
                    "type": "entry",
                    "entry": {
                        "kind": "message",
                        "id": format!("result-{turn}"),
                        "revision": 0,
                        "message": {
                            "role": "function_result",
                            "function_call_id": format!("call-{turn}"),
                            "function_id": "read_file",
                            "content": [{ "type": "text", "text": "x".repeat(1_999) }],
                            "details": {},
                            "is_error": false,
                            "timestamp": turn * 2 + 1
                        }
                    }
                })
                .to_string(),
            );
        }
    }
    let history = parse_session_lines("synthetic-long.jsonl", lines.iter().map(String::as_str))
        .expect("valid session records");

    let comparison = compare_decay_four(&history)
        .await
        .expect("generous inline budget avoids emergency reduction");

    assert_eq!(comparison.turn_count(), 130);
    assert_eq!(
        comparison.final_decay_tokens(),
        comparison.final_baseline_tokens()
    );
}

#[tokio::test]
async fn compares_an_empty_history_without_dividing_by_zero() {
    let history = parse_session_lines("empty.jsonl", std::iter::empty())
        .expect("an empty JSONL history is valid");

    let comparison = compare_decay_four(&history)
        .await
        .expect("empty history needs no assembly calls");

    assert_eq!(comparison.turn_count(), 0);
    assert_eq!(comparison.final_baseline_tokens(), 0);
    assert_eq!(comparison.final_decay_tokens(), 0);
}

#[tokio::test]
#[ignore = "reads the explicitly selected session-manager corpus"]
async fn replays_an_explicit_session_manager_directory() {
    let directory = std::env::var("CONTEXT_REPLAY_DIR")
        .expect("set CONTEXT_REPLAY_DIR to the session-manager JSONL directory");
    let report = compare_directory(std::path::Path::new(&directory))
        .await
        .expect("corpus records and production assembly must succeed");

    print!("{}", report.render());
}

fn opus(context_window: u64, max_output_tokens: u64) -> Model {
    Model {
        id: "claude-opus-5-5".to_string(),
        provider: "anthropic".to_string(),
        context_window,
        max_output_tokens,
        supports_vision: Some(true),
        ..Model::default()
    }
}

fn user(text: String) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }], "timestamp": 0 })
}

fn assistant(content: Value) -> Value {
    json!({
        "role": "assistant", "content": content, "stop_reason": "end",
        "model": "claude-opus-5-5", "provider": "anthropic", "timestamp": 0
    })
}

fn call(id: &str) -> Value {
    assistant(json!([{ "type": "function_call", "id": id, "function_id": "read_file" }]))
}

fn reply() -> Value {
    assistant(json!([{ "type": "text", "text": "ok" }]))
}

fn result(id: &str, content: Value) -> Value {
    json!({
        "role": "function_result", "function_call_id": id, "function_id": "read_file",
        "content": content, "details": {}, "is_error": false, "timestamp": 0
    })
}

fn text_result(id: &str, chars: usize) -> Value {
    result(id, json!([{ "type": "text", "text": "x".repeat(chars) }]))
}

fn history(messages: Vec<Value>) -> ReplayHistory {
    let lines: Vec<String> = messages
        .into_iter()
        .enumerate()
        .map(|(i, message)| {
            json!({
                "type": "entry",
                "entry": { "kind": "message", "id": format!("m{i}"), "revision": 0, "message": message }
            })
            .to_string()
        })
        .collect();
    parse_session_lines("synthetic.jsonl", lines.iter().map(String::as_str))
        .expect("valid session records")
}

fn no_prune() -> AssembleOptions {
    AssembleOptions {
        allow_prune: Some(false),
        ..AssembleOptions::default()
    }
}

/// Six user turns, each reading one ~18k-token result (under the cap).
fn six_large_results() -> Vec<Value> {
    (0..6)
        .flat_map(|turn| {
            let id = format!("c{turn}");
            [
                user(format!("turn {turn}")),
                call(&id),
                text_result(&id, 72_000),
                reply(),
            ]
        })
        .collect()
}

/// Every step of `context::assemble` that rewrites a message the previous
/// request already sent breaks Opus 5.5 / Fable 5.1 preserved thinking
/// (MOT-4845) and the provider prompt cache from that message on. Each row pins a site as it behaves today, so the follow-up
/// that fixes it flips exactly its row.
#[tokio::test]
async fn bound_prefix_edits_by_site() {
    let image = json!([
        { "type": "text", "text": "caption" },
        { "type": "image", "mime": "image/png", "data": "AAAA" }
    ]);
    type Row = (
        &'static str,
        Vec<Value>,
        Model,
        fn() -> AssembleOptions,
        Vec<(usize, &'static str, &'static str)>,
    );
    let rows: Vec<Row> = vec![
        // residual: media aging (follow-up) — the tool image the model
        // already saw becomes a marker once a later assistant exists.
        (
            "media aging",
            vec![
                user("look".into()),
                call("c1"),
                result("c1", image.clone()),
                reply(),
                user("next".into()),
            ],
            opus(1_000_000, 128_000),
            AssembleOptions::default,
            vec![(2, "messages[2]", "media aging")],
        ),
        // residual: media aging (follow-up) — a user image becomes a marker
        // once the user speaks again after a reply.
        (
            "user image aging",
            vec![
                json!({ "role": "user", "content": image, "timestamp": 0 }),
                reply(),
                user("next".into()),
            ],
            opus(1_000_000, 128_000),
            AssembleOptions::default,
            vec![(1, "messages[0]", "media aging")],
        ),
        // residual: cap × media aging (the media-aging follow-up) — the cap
        // counts the image (~4k tokens) and drops it; once the image ages to
        // a marker the result fits and is sent uncapped, even with
        // allow_prune: false.
        (
            "cap × media aging",
            vec![
                user("read".into()),
                call("c1"),
                result(
                    "c1",
                    json!([{ "type": "text", "text": "x".repeat(68_000) }, image[1]]),
                ),
                reply(),
                user("next".into()),
            ],
            opus(1_000_000, 128_000),
            no_prune,
            vec![(2, "messages[2]", "uncap")],
        ),
        // not an edit for text-only results: the Step 0 cap is
        // deterministic, so such a result is capped identically from the
        // first request that carries it.
        (
            "step 0 cap",
            vec![
                user("read".into()),
                call("c1"),
                text_result("c1", 120_000),
                reply(),
                user("next".into()),
                reply(),
            ],
            opus(1_000_000, 128_000),
            AssembleOptions::default,
            vec![],
        ),
        // fixed: Step 1 prune runs only over budget. It used to rewrite
        // turns 1-2's results at turn 6's first request here (36k freed >=
        // min_free_tokens) with the 1M window a tenth full.
        (
            "step 1 prune under budget",
            six_large_results(),
            opus(1_000_000, 128_000),
            AssembleOptions::default,
            vec![],
        ),
        // Over budget it still runs first, and here frees enough that
        // compaction never runs: the 120k window overflows at request 11.
        (
            "step 1 prune over budget",
            six_large_results(),
            opus(120_000, 8_000),
            AssembleOptions::default,
            vec![(11, "messages[2]", "prune")],
        ),
        // fixed by this branch: the harness sends allow_prune: false for
        // models that bind thinking to the prefix.
        (
            "step 1 allow_prune=false",
            six_large_results(),
            opus(1_000_000, 128_000),
            no_prune,
            vec![],
        ),
        // residual: Step 3 emergency reduction (follow-up) — the result
        // already sent shrinks when the next one arrives. allow_compaction:
        // false stands in for a busy lease or a failed summariser.
        (
            "step 3 emergency",
            vec![
                user("read".into()),
                call("c1"),
                text_result("c1", 28_000),
                reply(),
                user("again".into()),
                call("c2"),
                text_result("c2", 8_000),
            ],
            opus(10_000, 1_000),
            || AssembleOptions {
                allow_compaction: Some(false),
                ..AssembleOptions::default()
            },
            vec![(3, "messages[2]", "emergency")],
        ),
        // residual: keep-tail compaction (follow-up) — the summary is
        // rendered into the system prompt. The next request round-trips it
        // (previous_summary + verbatim tail) and extends the new prefix.
        (
            "keep-tail compaction",
            (0..6)
                .flat_map(|_| [user("x".repeat(6_000)), reply()])
                .chain([user("next".into())])
                .collect(),
            opus(10_000, 1_000),
            AssembleOptions::default,
            vec![(5, "system", "compaction")],
        ),
    ];
    for (name, messages, model, options, expected) in rows {
        let edits = prefix_edits(&history(messages), model, options)
            .await
            .expect("assemble");
        let edits: Vec<_> = edits
            .iter()
            .map(|(request, at, site)| (*request, at.as_str(), *site))
            .collect();
        assert_eq!(edits, expected, "{name}");
    }
}

/// How often each site edits the bound prefix in real transcripts. Every
/// request replays as Opus 5.5 (1M window, vision), whatever model served it.
#[tokio::test]
#[ignore = "reads the explicitly selected session-manager corpus"]
async fn prefix_edits_in_explicit_corpus() {
    let directory = std::env::var("CONTEXT_REPLAY_DIR")
        .expect("set CONTEXT_REPLAY_DIR to the session-manager JSONL directory");
    let by_site = |edits: &[(usize, String, &'static str)]| {
        edits
            .iter()
            .fold(BTreeMap::new(), |mut sites, (_, _, site)| {
                *sites.entry(*site).or_insert(0usize) += 1;
                sites
            })
    };
    let model = || opus(1_000_000, 128_000);
    println!("file\trequests\tdefault edits by site\tallow_prune=false edits by site");
    println!("(site compaction = system prompt edit; every other site = first edited messages[i])");
    for session in read_directory(Path::new(&directory)).expect("corpus directory lists") {
        let (name, history) = session.expect("corpus records parse");
        let default = prefix_edits(&history, model(), AssembleOptions::default)
            .await
            .expect("assemble");
        let guarded = prefix_edits(&history, model(), no_prune)
            .await
            .expect("assemble");
        let requests = history.request_ends().len();
        println!(
            "{name}\t{requests}\t{} {:?}\t{} {:?}",
            default.len(),
            by_site(&default),
            guarded.len(),
            by_site(&guarded)
        );
        println!("  default: {default:?}\n  allow_prune=false: {guarded:?}");
    }
}
