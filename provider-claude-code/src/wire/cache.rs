//! Anthropic prompt-cache markers. Three anchor points: the system prompt,
//! the tools array tail, and the last *stable* assistant turn in messages
//! (one whose tool_uses all have downstream tool_results — an unstable
//! anchor would be invalidated next turn).
use serde_json::{json, Value};

/// Below this many chars a prefix isn't worth a cache write.
pub const CACHE_MIN_CHARS: usize = 4096;
const CACHE_FLAG_ENV: &str = "PROVIDER_CLAUDE_CODE_CACHE";

/// Kill switch: unset or anything but 0/false/FALSE/False = enabled.
pub fn cache_enabled() -> bool {
    match std::env::var(CACHE_FLAG_ENV) {
        Ok(v) => !matches!(v.as_str(), "0" | "false" | "FALSE" | "False"),
        Err(_) => true,
    }
}

fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

/// The identity line the subscription OAuth backend requires as the first
/// system block — requests must resemble the official Claude Code CLI or the
/// token is rejected. This is a wire artifact only; agents never see it (the
/// router-facing identity prompt is the second block).
pub const CLAUDE_CODE_SYSTEM: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// System prompt → wire `system` field. Always an array: block 0 is the Claude
/// Code identity line, block 1 the router-supplied prompt when non-empty. The
/// cache-control anchor lands on the last block once the combined prompt is big
/// enough to be worth a cache write.
pub fn build_system_field(prompt: &str, enabled: bool) -> Value {
    let mut blocks = vec![json!({ "type": "text", "text": CLAUDE_CODE_SYSTEM })];
    if !prompt.is_empty() {
        blocks.push(json!({ "type": "text", "text": prompt }));
    }
    if enabled && (CLAUDE_CODE_SYSTEM.len() + prompt.len()) >= CACHE_MIN_CHARS {
        if let Some(obj) = blocks.last_mut().and_then(Value::as_object_mut) {
            obj.insert("cache_control".into(), ephemeral());
        }
    }
    Value::Array(blocks)
}

/// Mark the last tool when the serialized tools array clears the minimum.
pub fn apply_tools_cache_control(tools: &mut [Value], enabled: bool) {
    if !enabled || tools.is_empty() {
        return;
    }
    let size: usize = tools.iter().map(|t| t.to_string().len()).sum();
    if size < CACHE_MIN_CHARS {
        return;
    }
    if let Some(obj) = tools.last_mut().and_then(Value::as_object_mut) {
        obj.insert("cache_control".into(), ephemeral());
    }
}

/// Anchor on the last stable assistant turn, on its last block that accepts
/// cache_control — Anthropic rejects it on thinking/redacted_thinking blocks,
/// which can trail a turn under interleaved thinking.
pub fn apply_messages_cache_anchor(wire: &mut [Value], enabled: bool) {
    if !enabled || wire.is_empty() {
        return;
    }
    let Some(last_stable) = (0..wire.len())
        .rev()
        .find(|&i| is_stable_assistant(wire, i))
    else {
        return;
    };
    let Some(content) = wire[last_stable]
        .get_mut("content")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for block in content.iter_mut().rev() {
        let ty = block.get("type").and_then(Value::as_str).unwrap_or("");
        if ty == "thinking" || ty == "redacted_thinking" {
            continue;
        }
        if let Some(obj) = block.as_object_mut() {
            obj.insert("cache_control".into(), ephemeral());
        }
        return;
    }
}

fn is_stable_assistant(wire: &[Value], idx: usize) -> bool {
    let msg = &wire[idx];
    if msg.get("role").and_then(Value::as_str) != Some("assistant") {
        return false;
    }
    let Some(content) = msg.get("content").and_then(Value::as_array) else {
        return true;
    };
    content
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|b| b.get("id").and_then(Value::as_str))
        .all(|id| has_downstream_tool_result(&wire[idx + 1..], id))
}

fn has_downstream_tool_result(later: &[Value], id: &str) -> bool {
    later.iter().any(|m| {
        m.get("role").and_then(Value::as_str) == Some("user")
            && m.get("content")
                .and_then(Value::as_array)
                .is_some_and(|content| {
                    content.iter().any(|b| {
                        b.get("type").and_then(Value::as_str) == Some("tool_result")
                            && b.get("tool_use_id").and_then(Value::as_str) == Some(id)
                    })
                })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_field_forms() {
        // empty prompt: only the spoof block, no cache marker
        let v = build_system_field("", true);
        let blocks = v.as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["text"], CLAUDE_CODE_SYSTEM);
        assert!(blocks[0].get("cache_control").is_none());

        // short prompt: spoof block + prompt block, no cache marker
        let v = build_system_field("short", true);
        let blocks = v.as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["text"], CLAUDE_CODE_SYSTEM);
        assert_eq!(blocks[1]["text"], "short");
        assert!(blocks[1].get("cache_control").is_none());

        // long prompt: cache anchor on the last (prompt) block
        let long = "x".repeat(CACHE_MIN_CHARS);
        let v = build_system_field(&long, true);
        assert_eq!(v[1]["cache_control"]["type"], "ephemeral");
        // disabled: no cache marker no matter the size
        let v = build_system_field(&long, false);
        assert!(v[1].get("cache_control").is_none());
    }

    #[test]
    fn tools_marker_only_past_threshold() {
        let mut small = vec![json!({ "name": "a", "input_schema": {} })];
        apply_tools_cache_control(&mut small, true);
        assert!(small[0].get("cache_control").is_none());

        let big_schema = json!({ "description": "y".repeat(CACHE_MIN_CHARS) });
        let mut big = vec![
            json!({ "name": "a" }),
            json!({ "name": "b", "input_schema": big_schema }),
        ];
        apply_tools_cache_control(&mut big, true);
        assert!(
            big[0].get("cache_control").is_none(),
            "only the last tool is marked"
        );
        assert_eq!(big[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn anchor_lands_on_last_stable_assistant_skipping_thinking() {
        let mut wire = vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": "q" }] }),
            json!({ "role": "assistant", "content": [
                { "type": "text", "text": "a" },
                { "type": "thinking", "thinking": "t", "signature": "s" },
            ] }),
            json!({ "role": "user", "content": [{ "type": "text", "text": "q2" }] }),
        ];
        apply_messages_cache_anchor(&mut wire, true);
        let content = wire[1]["content"].as_array().unwrap();
        assert_eq!(
            content[0]["cache_control"]["type"], "ephemeral",
            "text block marked"
        );
        assert!(
            content[1].get("cache_control").is_none(),
            "thinking never marked"
        );
    }

    #[test]
    fn unstable_assistant_with_orphan_tool_use_not_anchored() {
        let mut wire = vec![
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "old" }] }),
            json!({ "role": "assistant", "content": [
                { "type": "tool_use", "id": "t1", "name": "f", "input": {} },
            ] }),
        ];
        apply_messages_cache_anchor(&mut wire, true);
        // anchor falls back to the earlier stable assistant
        assert_eq!(wire[0]["content"][0]["cache_control"]["type"], "ephemeral");
        assert!(wire[1]["content"][0].get("cache_control").is_none());
    }

    #[test]
    fn resolved_tool_use_is_stable() {
        let mut wire = vec![
            json!({ "role": "assistant", "content": [
                { "type": "tool_use", "id": "t1", "name": "f", "input": {} },
            ] }),
            json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "ok" },
            ] }),
        ];
        apply_messages_cache_anchor(&mut wire, true);
        assert_eq!(wire[0]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn disabled_flag_is_a_no_op() {
        let mut wire =
            vec![json!({ "role": "assistant", "content": [{ "type": "text", "text": "a" }] })];
        apply_messages_cache_anchor(&mut wire, false);
        assert!(wire[0]["content"][0].get("cache_control").is_none());
    }

    #[test]
    fn partially_resolved_assistant_is_unstable() {
        // two tool_uses, only one resolved downstream → not a stable anchor
        let mut wire = vec![
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "old" }] }),
            json!({ "role": "assistant", "content": [
                { "type": "tool_use", "id": "t1", "name": "f", "input": {} },
                { "type": "tool_use", "id": "t2", "name": "g", "input": {} },
            ] }),
            json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "ok" },
            ] }),
        ];
        apply_messages_cache_anchor(&mut wire, true);
        // falls back to the earlier fully-stable assistant
        assert_eq!(wire[0]["content"][0]["cache_control"]["type"], "ephemeral");
        assert!(wire[1]["content"][0].get("cache_control").is_none());
        assert!(wire[1]["content"][1].get("cache_control").is_none());
    }

    #[test]
    fn all_thinking_assistant_gets_no_marker() {
        // a stable assistant whose only blocks are thinking → no eligible block
        let mut wire = vec![json!({ "role": "assistant", "content": [
            { "type": "thinking", "thinking": "t", "signature": "s" },
            { "type": "redacted_thinking" },
        ] })];
        apply_messages_cache_anchor(&mut wire, true);
        assert!(wire[0]["content"][0].get("cache_control").is_none());
        assert!(wire[0]["content"][1].get("cache_control").is_none());
    }
}
