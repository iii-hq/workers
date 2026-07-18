//! `fp::pipe` — a short worker-side pipeline: each step triggers a
//! function and its result lands in the next step's payload (`into`, default
//! `/value`). Pure `fp::*` transforms run inline; every other step goes
//! over the bus via `iii.trigger`. Large values thread worker→worker and
//! never round-trip through the model; the chat gets a receipt with per-step
//! sizes and a preview.
//!
//! Steps run with THIS worker's authority, not the calling agent's dispatch
//! policy — the `fp::pipe` call itself is the policy/approval surface
//! (an approver sees the full step list). `forbidden_step` statically refuses
//! the classes that must never ride on worker authority.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;

use crate::util;

pub const PIPE_ID: &str = "fp::pipe";
pub const PIPE_DESC: &str =
    "Run a short worker-side pipeline in one call: each step triggers a function and its result \
     lands in the next step's payload at `into` (default \"/value\") — large values flow \
     worker→worker and never enter the chat. fp::* transform steps \
     (get/pick/omit/take/drop/map/filter/split/join/uniq/size/compact/nth/getOr/flatten/\
     sortBy/reverse, lodash semantics) run inline and \
     thread the transformed value itself — the {value} wrapper they return when called \
     directly never appears between steps. \
     The FIRST step receives no threaded value: start with a producing function (a fetch, a \
     state::get) or seed a leading transform via payload.value. \
     Example: fetch an article and persist it trimmed: \
     through=[{function:'scrapling::fetch', payload:{url,format:'markdown'}}, \
     {function:'fp::get', payload:{path:'/content'}}, \
     {function:'fp::take', payload:{n:20000}}, \
     {function:'state::set', payload:{scope,key}}]. Returns per-step sizes and a preview, not \
     the value.";

/// ponytail: v1 ceilings — no shell/coder steps (they need the harness
/// fs_scope stamp), no nested pipes, no trigger-control or session/approval
/// steps, and a held (approval) release re-runs steps from scratch is
/// unsupported: the pipe call itself is what approvers review.
const MAX_STEPS: usize = 12;
const DEFAULT_PREVIEW_CHARS: usize = 400;
/// Hard cap on `preview_chars`: the receipt is a glimpse (or a slice to
/// reason over), never the bulk channel it exists to replace — an unbounded
/// u32 would let one call ship an entire fetched document back into the chat.
const MAX_PREVIEW_CHARS: usize = 8_000;
/// Per-step bus budget. The WHOLE pipe must also fit the caller's own
/// dispatch timeout on `fp::pipe`.
const STEP_TIMEOUT_MS: u64 = 120_000;

// NOTE: deliberately NOT `#[serde(deny_unknown_fields)]`. The engine injects
// a `_caller_worker_id` field into every dispatched call's payload (see
// shell/src/functions/types.rs for the same note), so a strict top-level
// struct would reject EVERY live call. `PipeStep` below stays strict — the
// engine never injects inside nested values, and a typo'd step field failing
// loudly is teachable. (A `//` comment on purpose: a `///` doc comment
// becomes the published schema description, leaking engine plumbing to every
// model that reads engine::functions::info.)
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PipeRequest {
    /// Steps run in order; step N+1 receives step N's threaded value.
    pub through: Vec<PipeStep>,
    /// Preview length of the final threaded value in the receipt (capped at
    /// 8000).
    #[serde(default)]
    pub preview_chars: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PipeStep {
    /// Function id to trigger (e.g. `scrapling::fetch`, `fp::get`,
    /// `state::set`).
    pub function: String,
    /// Base payload for the call.
    #[serde(default)]
    pub payload: Option<Value>,
    /// JSON pointer in THIS step's payload where the previous step's value
    /// lands (default `/value`). Object keys only (`~0`/`~1` escapes are
    /// decoded); indexing into arrays is unsupported.
    #[serde(default)]
    pub into: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StepReceipt {
    pub function: String,
    /// Size of the value threaded OUT of this step (chars of its string or
    /// serialized form).
    pub chars: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PipeResponse {
    pub steps: Vec<StepReceipt>,
    pub value_preview: Option<String>,
}

/// Step functions a pipe refuses. Steps run with THIS worker's authority, so
/// every class the agent policy hard-denies (iii-permissions.yaml) must be
/// refused here too — an allowlisted or approve-always `fp::pipe` would
/// otherwise be a standing policy bypass. `state::*` is the deliberate
/// exception: persisting the threaded value is the pipe's purpose, and the
/// pipe call itself is the approval surface an approver reviews.
fn forbidden_step(function_id: &str) -> Option<&'static str> {
    if (function_id.starts_with("shell::") && !function_id.starts_with("shell::filesystem::"))
        || function_id.starts_with("coder::")
    {
        return Some("filesystem-scoped functions (shell::*/coder::*) are not supported in a pipe — call them directly");
    }
    if function_id == PIPE_ID {
        return Some(
            "pipes do not nest — to start from a literal value, seed the FIRST transform \
             step's payload (\"value\": …) instead of wrapping the value in a pipe step",
        );
    }
    if function_id == "engine::register_trigger" || function_id == "engine::unregister_trigger" {
        return Some("trigger control calls are not supported in a pipe — call them directly");
    }
    if function_id.starts_with("session::") || function_id.starts_with("approval::") {
        return Some(
            "session/approval functions are not supported in a pipe — steps run with worker \
             authority, which would bypass the agent-level deny on them",
        );
    }
    if function_id.starts_with("configuration::") || function_id.starts_with("oauth::") {
        return Some(
            "credential/config functions are not supported in a pipe — steps run with worker \
             authority, which would bypass the agent-level deny on them",
        );
    }
    if function_id.starts_with("router::") || function_id.starts_with("provider::") {
        return Some(
            "LLM routing/provider functions are not supported in a pipe — model spend must \
             stay under the harness loop's accounting",
        );
    }
    if function_id.starts_with("harness::") || function_id.starts_with("run::") {
        return Some(
            "harness/turn-control functions are not supported in a pipe — steps run with \
             worker authority, which would bypass the agent-level deny on them",
        );
    }
    if function_id.starts_with("stream::")
        || function_id.starts_with("hook-fanout::")
        || function_id.starts_with("iii::")
    {
        return Some("bus-internal functions are not supported in a pipe");
    }
    if function_id.ends_with("::on-config-change") {
        return Some("lifecycle hooks are engine-dispatched, never pipe steps");
    }
    None
}

/// Static validation, separated from execution for testability.
pub fn validate(req: &PipeRequest) -> Result<(), String> {
    if req.through.is_empty() || req.through.len() > MAX_STEPS {
        return Err(format!("a pipe takes 1..={MAX_STEPS} steps"));
    }
    for (i, step) in req.through.iter().enumerate() {
        if let Some(reason) = forbidden_step(&step.function) {
            return Err(format!("step {} ({}): {reason}", i + 1, step.function));
        }
    }
    // Fail fast on an unseeded leading transform (live-tested trap: serde's
    // bare "missing field `value`" taught nothing). Only our own transforms
    // are statically checkable; a foreign first step just runs.
    if let Some(first) = req.through.first() {
        if util::is_transform(&first.function)
            && first
                .payload
                .as_ref()
                .and_then(|p| p.get("value"))
                .is_none()
        {
            return Err(format!(
                "step 1 ({}) receives no threaded input — seed it in the first step's payload \
                 ({{\"value\": …}}), or start the pipe with a producing function (a fetch, a \
                 state::get)",
                first.function
            ));
        }
    }
    Ok(())
}

/// Set `value` at `pointer` inside `payload`, creating intermediate objects.
fn inject(payload: Value, pointer: &str, value: Value) -> Result<Value, String> {
    // RFC 6901 token unescape (`~1` → `/`, then `~0` → `~`). Object keys
    // only — a token that lands on an array errors below instead of indexing.
    let parts: Vec<String> = pointer
        .strip_prefix('/')
        .map(|p| {
            p.split('/')
                .map(|t| t.replace("~1", "/").replace("~0", "~"))
                .collect()
        })
        .unwrap_or_default();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return Err(format!(
            "`into` must be a JSON pointer like \"/value\", got {pointer:?}"
        ));
    }
    let mut root = match payload {
        Value::Object(m) => Value::Object(m),
        Value::Null => json!({}),
        other => {
            return Err(format!(
                "step payload must be an object to receive `into`, got {}",
                util::kind_of(&other)
            ))
        }
    };
    let mut cursor = &mut root;
    for part in &parts[..parts.len() - 1] {
        let map = cursor.as_object_mut().expect("object cursor");
        cursor = map.entry(part.clone()).or_insert_with(|| json!({}));
        if !cursor.is_object() {
            return Err(format!(
                "`into` path {pointer:?} crosses a non-object at {part:?}"
            ));
        }
    }
    let map = cursor.as_object_mut().expect("object cursor");
    map.insert(parts[parts.len() - 1].clone(), value);
    Ok(root)
}

/// Requested preview length, clamped to [0, MAX_PREVIEW_CHARS].
fn preview_len(requested: Option<u32>) -> usize {
    requested
        .map(|c| c as usize)
        .unwrap_or(DEFAULT_PREVIEW_CHARS)
        .min(MAX_PREVIEW_CHARS)
}

fn chars_of(value: &Value) -> usize {
    match value {
        Value::String(s) => s.chars().count(),
        other => other.to_string().chars().count(),
    }
}

fn preview_of(value: &Value, max_chars: usize) -> String {
    let s = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => format!("{}…", &s[..byte_idx]),
        None => s,
    }
}

/// A step failure carries the receipts trail so the caller sees how far the
/// pipe got and at what sizes, without the values themselves. `i` is the
/// 0-based loop index; the message is 1-based to match the console's step
/// list (live-tested: 0-based "failed at step 2" next to a 2-step completed
/// trail read as "the 2nd step failed" when it was the 3rd).
fn step_error(i: usize, function: &str, msg: &str, receipts: &[StepReceipt]) -> String {
    let n = i + 1;
    let trail = receipts
        .iter()
        .map(|r| format!("{}→{}ch", r.function, r.chars))
        .collect::<Vec<_>>()
        .join(" · ");
    if trail.is_empty() {
        format!("pipe failed at step {n} ({function}): {msg}")
    } else {
        format!("pipe failed at step {n} ({function}): {msg} · completed: {trail}")
    }
}

/// Execute a pipe: transforms inline, everything else over the bus. The first
/// failing step stops the pipe with a teachable error carrying the trail.
pub async fn run(iii: &IIIClient, req: PipeRequest) -> Result<PipeResponse, String> {
    validate(&req)?;
    let preview_chars = preview_len(req.preview_chars);

    let mut receipts: Vec<StepReceipt> = Vec::with_capacity(req.through.len());
    let mut piped: Option<Value> = None;

    for (i, step) in req.through.iter().enumerate() {
        let mut args = step.payload.clone().unwrap_or_else(|| json!({}));
        if let Some(value) = piped.take() {
            let pointer = step.into.as_deref().unwrap_or("/value");
            args = inject(args, pointer, value)
                .map_err(|msg| step_error(i, &step.function, &msg, &receipts))?;
        }

        let value = match util::apply(&step.function, &args) {
            Some(result) => result.map_err(|msg| step_error(i, &step.function, &msg, &receipts))?,
            None => iii
                .trigger(TriggerRequest {
                    function_id: step.function.clone(),
                    payload: args,
                    action: None,
                    timeout_ms: Some(STEP_TIMEOUT_MS),
                })
                .await
                .map_err(|e| step_error(i, &step.function, &e.to_string(), &receipts))?,
        };

        receipts.push(StepReceipt {
            function: step.function.clone(),
            chars: chars_of(&value),
        });
        piped = Some(value);
    }

    let final_value = piped.unwrap_or(Value::Null);
    // Receipts + preview only, matching PipeResponse — a step's raw result
    // (e.g. state::set echoing the stored value) must never ride along, or
    // the receipt grows to the size of the value it exists to avoid.
    Ok(PipeResponse {
        value_preview: Some(preview_of(&final_value, preview_chars)),
        steps: receipts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: Value) -> Result<PipeRequest, String> {
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    #[test]
    fn tolerates_the_engine_injected_caller_worker_id() {
        // The engine stamps `_caller_worker_id` into every dispatched payload
        // (see shell/src/functions/types.rs) — this is the byte-shape of the
        // live request a stale deny_unknown_fields build rejected 4× in a row
        // (2026-07-16 research-pipeline session), forcing an 18KB
        // bulk-through-chat fallback:
        let req = parse(json!({
            "preview_chars": 300,
            "through": [
                { "function": "scrapling::fetch",
                  "payload": { "url": "https://en.wikipedia.org/wiki/Circuit_breaker_design_pattern",
                               "format": "markdown", "main_content_only": true } },
                { "function": "fp::get", "payload": { "path": "/content" } },
                { "function": "fp::take", "payload": { "n": 6000 } },
                { "function": "state::set", "into": "/value/content",
                  "payload": { "scope": "research", "key": "article",
                               "value": { "url": "https://en.wikipedia.org/wiki/Circuit_breaker_design_pattern",
                                          "title": "Circuit Breaker Design Pattern",
                                          "content": "" } } },
            ],
            "_caller_worker_id": "harness",
        }))
        .unwrap();
        assert_eq!(req.through.len(), 4);
        assert!(validate(&req).is_ok());

        // ...while STEP-level strictness stays: the engine never injects
        // inside nested values, so a typo'd step field still fails loudly.
        assert!(
            parse(json!({ "through": [{ "function": "a::b", "args": {} }] }))
                .unwrap_err()
                .contains("args")
        );
    }

    #[test]
    fn unseeded_leading_transform_fails_fast_and_teachably() {
        // The exact live shape that failed with serde's bare "missing field
        // `value`": a transform first, its seed nowhere.
        let req = parse(json!({ "through": [
            { "function": "fp::split", "payload": { "separator": "," } },
            { "function": "fp::uniq" },
        ]}))
        .unwrap();
        let err = validate(&req).unwrap_err();
        // step indices are 1-based in every message, matching the console
        assert!(err.contains("step 1") && err.contains("no threaded input"));
        assert!(err.contains("producing function"));

        // seeded leading transform passes (explicit null seed counts: the
        // transform itself decides whether null is workable)
        for seed in [json!("a,b"), Value::Null] {
            let req = parse(json!({ "through": [
                { "function": "fp::split", "payload": { "separator": ",", "value": seed } },
            ]}))
            .unwrap();
            assert!(validate(&req).is_ok());
        }

        // a producing first step is not ours to check
        let req = parse(json!({ "through": [
            { "function": "scrapling::fetch", "payload": { "url": "u" } },
            { "function": "fp::uniq" },
        ]}))
        .unwrap();
        assert!(validate(&req).is_ok());
    }

    #[test]
    fn validate_checks_shape_and_forbidden_steps() {
        // fetch → get → take → set: the motivating pipeline parses, zero `into`.
        let req = parse(json!({ "through": [
            { "function": "scrapling::fetch", "payload": { "url": "u", "format": "markdown" } },
            { "function": "fp::get", "payload": { "path": "/content" } },
            { "function": "fp::take", "payload": { "n": 20000 } },
            { "function": "state::set", "payload": { "scope": "s", "key": "k" } },
        ]}))
        .unwrap();
        assert_eq!(req.through.len(), 4);
        assert!(req.through.iter().all(|s| s.into.is_none()));
        assert!(validate(&req).is_ok());

        // unknown step fields fail loudly, not silently
        assert!(parse(json!({ "through": [
            { "function": "a::b", "pick": "/content" },
        ]}))
        .unwrap_err()
        .contains("pick"));

        // scoped / control / nested / worker-authority steps are refused with
        // a reason — mirroring every agent-policy hard-denied class
        // (iii-permissions.yaml), state::* excepted on purpose
        for (function, needle) in [
            ("shell::exec", "filesystem-scoped"),
            ("fp::pipe", "nest"),
            ("engine::register_trigger", "trigger control"),
            ("session::append", "worker"),
            ("approval::resolve", "worker"),
            ("configuration::get", "credential"),
            ("oauth::anthropic::login", "credential"),
            ("router::chat", "routing"),
            ("provider::anthropic::chat", "routing"),
            ("harness::send", "turn-control"),
            ("run::start", "turn-control"),
            ("stream::set", "bus-internal"),
            ("iii::durable::publish", "bus-internal"),
            ("storage::on-config-change", "lifecycle"),
        ] {
            let req = parse(json!({ "through": [{ "function": function }] })).unwrap();
            assert!(validate(&req).unwrap_err().contains(needle), "{function}");
        }

        let empty = parse(json!({ "through": [] })).unwrap();
        assert!(validate(&empty).is_err());
    }

    #[test]
    fn pipe_as_seed_step_redirects_to_the_seed_rule() {
        // Live-tested (haiku-4.5): five consecutive attempts used fp::pipe
        // {value} as step 1 to inject a literal starting value — the refusal
        // must teach the seed rule, not just state the nesting ban.
        let req = parse(json!({ "through": [
            { "function": "fp::pipe", "payload": { "value": [1, 2, 3] } },
            { "function": "fp::take", "payload": { "n": 3 } },
        ]}))
        .unwrap();
        let err = validate(&req).unwrap_err();
        assert!(err.contains("do not nest") && err.contains("seed the FIRST transform"));
    }

    #[test]
    fn inject_shapes_the_value() {
        // inject creates intermediate objects and rejects bad pointers
        assert_eq!(
            inject(json!({ "scope": "s" }), "/value", json!("x")).unwrap(),
            json!({ "scope": "s", "value": "x" })
        );
        assert_eq!(
            inject(json!({}), "/config/value", json!(1)).unwrap(),
            json!({ "config": { "value": 1 } })
        );
        // schema-literal models pad steps with a placeholder ({"value": {}}
        // — the published schemas mark `value` required); the threaded value
        // must overwrite it (live-tested: ornith-35b padded every join step)
        assert_eq!(
            inject(
                json!({ "separator": "-", "value": {} }),
                "/value",
                json!(["a"])
            )
            .unwrap(),
            json!({ "separator": "-", "value": ["a"] })
        );
        assert!(inject(json!({}), "value", json!(1)).is_err());
        assert!(inject(json!({ "a": 1 }), "/a/b", json!(1)).is_err());

        // RFC 6901 escapes decode per token; arrays refuse teachably
        assert_eq!(
            inject(json!({}), "/headers/x~1trace", json!(1)).unwrap(),
            json!({ "headers": { "x/trace": 1 } })
        );
        assert_eq!(
            inject(json!({}), "/a~0b", json!(1)).unwrap(),
            json!({ "a~b": 1 })
        );
        assert!(inject(json!({ "items": [] }), "/items/0", json!(1))
            .unwrap_err()
            .contains("non-object"));
    }

    #[test]
    fn previews_and_errors_stay_small() {
        assert_eq!(preview_of(&json!("abcdef"), 3), "abc…");
        assert_eq!(preview_of(&json!("ab"), 3), "ab");

        // an unbounded preview would defeat the receipt design — clamp it
        assert_eq!(preview_len(None), DEFAULT_PREVIEW_CHARS);
        assert_eq!(preview_len(Some(50)), 50);
        assert_eq!(preview_len(Some(u32::MAX)), MAX_PREVIEW_CHARS);

        let receipts = vec![
            StepReceipt {
                function: "scrapling::fetch".into(),
                chars: 184_232,
            },
            StepReceipt {
                function: "fp::get".into(),
                chars: 180_101,
            },
        ];
        // loop index 2 = the third step; the message is 1-based like the UI
        let msg = step_error(2, "state::set", "boom", &receipts);
        assert!(msg.contains("step 3 (state::set): boom"));
        assert!(msg.contains("scrapling::fetch→184232ch"));
    }

    #[test]
    fn desc_teaches_bare_value_threading() {
        // The live trap: a model that saw {value: X} from ten direct calls
        // composed `pick {paths:["value"]}` to unwrap between steps. The
        // published description must keep saying nothing threads wrapped.
        assert!(PIPE_DESC.contains("thread the transformed value itself"));
    }
}
