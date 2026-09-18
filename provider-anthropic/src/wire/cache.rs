//! Anthropic prompt-cache markers. Three anchor points: the system prompt,
//! the tools array tail, and the last *stable* assistant turn in messages
//! (one whose tool_uses all have downstream tool_results — an unstable
//! anchor would be invalidated next turn).
use llm_router::types::router::PromptSection;
use serde_json::{json, Value};

/// Below this many chars a prefix isn't worth a cache write.
pub const CACHE_MIN_CHARS: usize = 4096;
const CACHE_FLAG_ENV: &str = "PROVIDER_ANTHROPIC_CACHE";
const CACHE_TTL_ENV: &str = "PROVIDER_ANTHROPIC_CACHE_TTL";

/// Kill switch: unset or anything but 0/false/FALSE/False = enabled.
pub fn cache_enabled() -> bool {
    match std::env::var(CACHE_FLAG_ENV) {
        Ok(v) => !matches!(v.as_str(), "0" | "false" | "FALSE" | "False"),
        Err(_) => true,
    }
}

/// TTL for the shared-prefix markers (tools + the boundary block): unset or
/// `1h` = the 1-hour cache (2x base on the one write, reads unchanged, so a
/// profile stays warm across sessions an hour apart); `5m` = the default
/// 5-minute cache. The per-turn messages anchor always stays at 5 minutes:
/// it changes every turn, and longer TTLs must precede shorter ones.
pub fn cache_ttl() -> Option<&'static str> {
    match std::env::var(CACHE_TTL_ENV).as_deref() {
        Ok("5m") => None,
        _ => Some("1h"),
    }
}

fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

fn ephemeral_ttl(ttl: Option<&str>) -> Value {
    match ttl {
        Some(ttl) => json!({ "type": "ephemeral", "ttl": ttl }),
        None => ephemeral(),
    }
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

/// Sectioned system prompt → wire `system` array: one text block per non-empty
/// section, `cache_control` on every boundary block — so the frozen profile
/// prefix caches on its own and the per-session tail after it does not
/// invalidate it. No byte gate here: Anthropic applies its per-model token
/// minimum over the whole prefix (tools included) and silently skips a short
/// one, so a byte count of the system text alone would only lose eligible
/// boundaries. None when every section is empty (omit the field).
pub fn build_system_blocks(
    sections: &[PromptSection],
    enabled: bool,
    ttl: Option<&str>,
) -> Option<Value> {
    let mut blocks = Vec::new();
    let mut marked = 0usize;
    for section in sections.iter().filter(|s| !s.text.is_empty()) {
        let mut block = json!({ "type": "text", "text": section.text });
        // ponytail: cap 2 so the tools and messages anchors keep the total <= 4
        if enabled && section.cache_boundary && marked < 2 {
            block["cache_control"] = ephemeral_ttl(ttl);
            marked += 1;
        }
        blocks.push(block);
    }
    (!blocks.is_empty()).then_some(Value::Array(blocks))
}

/// Mark the last tool when the serialized tools array clears the minimum.
/// `ttl` rides along on the sectioned path only: tools precede the system
/// prefix, and a 1-hour boundary block behind a 5-minute tools marker would
/// break the longer-before-shorter rule.
pub fn apply_tools_cache_control(tools: &mut [Value], enabled: bool, ttl: Option<&str>) {
    if !enabled || tools.is_empty() {
        return;
    }
    let size: usize = tools.iter().map(|t| t.to_string().len()).sum();
    if size < CACHE_MIN_CHARS {
        return;
    }
    if let Some(obj) = tools.last_mut().and_then(Value::as_object_mut) {
        obj.insert("cache_control".into(), ephemeral_ttl(ttl));
    }
}

/// Anchor on the newest user turn's last tool_result when present; otherwise
/// fall back to the last stable assistant turn. Anthropic rejects cache_control
/// on thinking/redacted_thinking blocks, which can trail a turn under
/// interleaved thinking.
pub fn apply_messages_cache_anchor(wire: &mut [Value], enabled: bool) {
    if !enabled || wire.is_empty() {
        return;
    }
    if let Some(block) = wire
        .iter_mut()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|message| message.get_mut("content").and_then(Value::as_array_mut))
        .and_then(|content| {
            content
                .iter_mut()
                .rev()
                .find(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        })
    {
        if let Some(obj) = block.as_object_mut() {
            obj.insert("cache_control".into(), ephemeral());
            return;
        }
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
    fn system_blocks_mark_every_declared_boundary() {
        let section = |text: &str, boundary: bool| PromptSection {
            text: text.into(),
            cache_boundary: boundary,
        };
        assert!(build_system_blocks(&[section("", true)], true, None).is_none());
        // one block per section; the boundary is marked whatever its size
        // (Anthropic applies the token minimum itself), the tail stays bare
        let v = build_system_blocks(
            &[section("stable", true), section("dyn", false)],
            true,
            None,
        )
        .unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
        assert_eq!(v[0]["cache_control"]["type"], "ephemeral");
        assert!(v[1].get("cache_control").is_none());
        assert_eq!(v[1]["text"], "dyn");
        // disabled: no marker
        let long = "x".repeat(CACHE_MIN_CHARS);
        let v = build_system_blocks(&[section(&long, true)], false, None).unwrap();
        assert!(v[0].get("cache_control").is_none());
        // three boundaries: at most two markers
        let v = build_system_blocks(
            &[
                section(&long, true),
                section(&long, true),
                section(&long, true),
            ],
            true,
            None,
        )
        .unwrap();
        let marked = v
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b.get("cache_control").is_some())
            .count();
        assert_eq!(marked, 2);
    }

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
        apply_tools_cache_control(&mut small, true, None);
        assert!(small[0].get("cache_control").is_none());

        let big_schema = json!({ "description": "y".repeat(CACHE_MIN_CHARS) });
        let mut big = vec![
            json!({ "name": "a" }),
            json!({ "name": "b", "input_schema": big_schema }),
        ];
        apply_tools_cache_control(&mut big, true, None);
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
    fn newest_user_tool_result_wins_over_stable_assistant() {
        let mut wire = vec![
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "old stable" }] }),
            json!({ "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "old result" }] }),
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "latest stable" }] }),
            json!({ "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t2", "content": "fresh" }] }),
        ];
        apply_messages_cache_anchor(&mut wire, true);
        assert!(wire[0]["content"][0].get("cache_control").is_none());
        assert!(wire[1]["content"][0].get("cache_control").is_none());
        assert!(wire[2]["content"][0].get("cache_control").is_none());
        assert_eq!(wire[3]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn last_tool_result_wins_over_other_blocks_in_the_newest_user_message() {
        let mut wire = vec![
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "stable" }] }),
            json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "first" },
                { "type": "tool_result", "tool_use_id": "t2", "content": "last" },
                { "type": "text", "text": "trailing" },
            ] }),
        ];
        apply_messages_cache_anchor(&mut wire, true);
        assert!(wire[0]["content"][0].get("cache_control").is_none());
        assert!(wire[1]["content"][0].get("cache_control").is_none());
        assert_eq!(wire[1]["content"][1]["cache_control"]["type"], "ephemeral");
        assert!(wire[1]["content"][2].get("cache_control").is_none());
    }

    #[test]
    fn stable_assistant_remains_the_fallback_without_a_newest_user_tool_result() {
        let mut wire = vec![
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "stable" }] }),
            json!({ "role": "user", "content": [{ "type": "text", "text": "next" }] }),
        ];
        apply_messages_cache_anchor(&mut wire, true);
        assert_eq!(wire[0]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn disabled_cache_leaves_the_newest_user_tool_result_unmarked() {
        let mut wire = vec![json!({ "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "t1", "content": "fresh" },
        ] })];
        apply_messages_cache_anchor(&mut wire, false);
        assert!(wire[0]["content"][0].get("cache_control").is_none());
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
    fn tool_result_is_preferred_over_a_resolved_tool_use() {
        let mut wire = vec![
            json!({ "role": "assistant", "content": [
                { "type": "tool_use", "id": "t1", "name": "f", "input": {} },
            ] }),
            json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "ok" },
            ] }),
        ];
        apply_messages_cache_anchor(&mut wire, true);
        assert!(wire[0]["content"][0].get("cache_control").is_none());
        assert_eq!(wire[1]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn disabled_flag_is_a_no_op() {
        let mut wire =
            vec![json!({ "role": "assistant", "content": [{ "type": "text", "text": "a" }] })];
        apply_messages_cache_anchor(&mut wire, false);
        assert!(wire[0]["content"][0].get("cache_control").is_none());
    }

    #[test]
    fn tool_result_is_preferred_even_when_the_assistant_is_unstable() {
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
        assert!(wire[0]["content"][0].get("cache_control").is_none());
        assert!(wire[1]["content"][0].get("cache_control").is_none());
        assert!(wire[1]["content"][1].get("cache_control").is_none());
        assert_eq!(wire[2]["content"][0]["cache_control"]["type"], "ephemeral");
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

    #[test]
    fn ttl_rides_on_the_boundary_and_tools_markers_only() {
        let long = "x".repeat(CACHE_MIN_CHARS);
        let sections = [PromptSection {
            text: long.clone(),
            cache_boundary: true,
        }];
        let v = build_system_blocks(&sections, true, Some("1h")).unwrap();
        assert_eq!(
            v[0]["cache_control"],
            json!({ "type": "ephemeral", "ttl": "1h" })
        );
        let v = build_system_blocks(&sections, true, None).unwrap();
        assert_eq!(v[0]["cache_control"], json!({ "type": "ephemeral" }));
        let mut big = vec![
            json!({ "name": "a", "input_schema": { "description": "y".repeat(CACHE_MIN_CHARS) } }),
        ];
        apply_tools_cache_control(&mut big, true, Some("1h"));
        assert_eq!(big[0]["cache_control"]["ttl"], "1h");
        // the flat legacy block and the messages anchor never carry a ttl
        assert_eq!(
            build_system_field(&long, true).unwrap()[0]["cache_control"],
            json!({ "type": "ephemeral" })
        );
        assert_eq!(ephemeral_ttl(None), ephemeral());
    }
}
