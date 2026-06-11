//! Anthropic prompt-cache markers. Three anchor points: the system prompt,
//! the tools array tail, and the last *stable* assistant turn in messages
//! (one whose tool_uses all have downstream tool_results — an unstable
//! anchor would be invalidated next turn).
use serde_json::{json, Value};

/// Below this many chars a prefix isn't worth a cache write.
pub const CACHE_MIN_CHARS: usize = 4096;
const CACHE_FLAG_ENV: &str = "PROVIDER_ANTHROPIC_CACHE";

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

/// System prompt → wire `system` field. None when empty (omit the field);
/// the cache-marked array form once the prompt is big enough to be worth it.
pub fn build_system_field(prompt: &str, enabled: bool) -> Option<Value> {
    if prompt.is_empty() {
        return None;
    }
    if enabled && prompt.len() >= CACHE_MIN_CHARS {
        return Some(json!([
            { "type": "text", "text": prompt, "cache_control": ephemeral() }
        ]));
    }
    Some(Value::String(prompt.to_string()))
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
        assert!(build_system_field("", true).is_none());
        assert_eq!(
            build_system_field("short", true),
            Some(Value::String("short".into()))
        );
        let long = "x".repeat(CACHE_MIN_CHARS);
        let v = build_system_field(&long, true).unwrap();
        assert_eq!(v[0]["cache_control"]["type"], "ephemeral");
        // disabled: stays a plain string no matter the size
        assert_eq!(build_system_field(&long, false), Some(Value::String(long)));
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
}
