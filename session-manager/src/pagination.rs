//! Block segmentation and elision behind the lazy transcript readers,
//! `session::messages-tail` and `session::messages-range`.
//!
//! WHY a separate reader rather than options on `session::messages`: a UI
//! opening a long session wants the newest few exchanges and nothing else,
//! and wants a tool run of 300 calls to arrive as "300 calls, here is the
//! last one" until someone asks to see them all. Both are read-time shapings
//! of the same stored entries. Nothing here touches what is on disk, and
//! `session::messages` keeps returning every entry in full.
//!
//! The unit of pagination is the *block*: one entry, or one *activity run*
//! (a maximal stretch of assistant messages that call functions, the
//! results answering them, and the trigger-wake entries that woke the agent
//! into doing so). A page never cuts inside a block, so a reader can never
//! receive a call without room for its result, or the tail half of a run
//! whose head it would have collapsed together with. Runs mirror what the
//! console collapses behind "show all", deliberately a little coarser: a
//! coarser unit can only make page boundaries safer.

use serde_json::Value;

use crate::types::{AgentMessage, ContentBlock, JsonMap, SessionEntry};

/// One pagination unit: a half-open index range into the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub start: usize,
    pub end: usize,
    pub kind: BlockKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// A plain entry: user prompt, terminal assistant prose, bookkeeping.
    Single,
    /// Function calls, their results and the wakes that led to them.
    ActivityRun,
}

impl Block {
    pub fn contains(&self, idx: usize) -> bool {
        idx >= self.start && idx < self.end
    }
}

/// Split `path` (oldest first) into blocks.
pub fn activity_blocks(path: &[&SessionEntry]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut run_start: Option<usize> = None;
    for (idx, entry) in path.iter().enumerate() {
        if is_run_member(entry) {
            run_start.get_or_insert(idx);
            continue;
        }
        if let Some(start) = run_start.take() {
            blocks.push(Block {
                start,
                end: idx,
                kind: BlockKind::ActivityRun,
            });
        }
        blocks.push(Block {
            start: idx,
            end: idx + 1,
            kind: BlockKind::Single,
        });
    }
    if let Some(start) = run_start {
        blocks.push(Block {
            start,
            end: path.len(),
            kind: BlockKind::ActivityRun,
        });
    }
    blocks
}

/// Index of the block containing the entry at path index `idx`, if any.
pub fn block_index_of(blocks: &[Block], idx: usize) -> Option<usize> {
    blocks.iter().position(|b| b.contains(idx))
}

/// The harness's trigger-wake bookkeeping. Newer writers mark it in
/// `origin`; the id prefixes cover transcripts written before they did.
/// Both halves of a wake (the user-role notification and the custom record)
/// belong with the calls they caused, never on the far side of a page cut.
const WAKE_ID_PREFIXES: &[&str] = &[
    "e_fire_",
    "e_notify_",
    "e_expire_",
    "e_stalespawn_",
    "e_claimfail_",
    "e_condfail_",
    "e_trigfired_",
    "e_trigexpired_",
    "e_trigstale_",
];
const WAKE_CUSTOM_TYPE: &str = "trigger_fired";

fn origin_flag(origin: Option<&JsonMap>, key: &str) -> bool {
    origin
        .and_then(|o| o.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn is_wake_entry(id: &str, origin: Option<&JsonMap>) -> bool {
    origin_flag(origin, "notification")
        || origin_flag(origin, "trigger_fired")
        || WAKE_ID_PREFIXES.iter().any(|p| id.starts_with(p))
}

fn has_function_call(content: &[ContentBlock]) -> bool {
    content
        .iter()
        .any(|b| matches!(b, ContentBlock::FunctionCall { .. }))
}

fn is_run_member(entry: &SessionEntry) -> bool {
    match entry {
        SessionEntry::Message {
            id,
            origin,
            message,
            ..
        } => match &**message {
            AgentMessage::Assistant { content, .. } => has_function_call(content),
            AgentMessage::FunctionResult { .. } => true,
            AgentMessage::User { .. } => is_wake_entry(id, origin.as_ref()),
            AgentMessage::Custom { custom_type, .. } => {
                custom_type == WAKE_CUSTOM_TYPE || is_wake_entry(id, origin.as_ref())
            }
        },
        SessionEntry::Custom {
            id,
            origin,
            custom_type,
            ..
        } => custom_type == WAKE_CUSTOM_TYPE || is_wake_entry(id, origin.as_ref()),
    }
}

/// Which entries of a run the collapsed view still needs whole: the last
/// assistant message that calls functions (the console shows its last call
/// while collapsed), the results answering that message's calls, and the
/// wake entries (tiny, and drawn as the phase's header). Returns one flag
/// per entry of `run`, `true` = keep whole.
pub fn kept_in_collapsed_run(run: &[&SessionEntry]) -> Vec<bool> {
    let last_call_idx = run.iter().rposition(|e| {
        matches!(
            e,
            SessionEntry::Message { message, .. }
                if matches!(&**message, AgentMessage::Assistant { content, .. } if has_function_call(content))
        )
    });
    let last_call_ids: Vec<&str> = match last_call_idx.map(|i| run[i]) {
        Some(SessionEntry::Message { message, .. }) => message
            .content()
            .iter()
            .filter_map(|b| match b {
                ContentBlock::FunctionCall { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    run.iter()
        .enumerate()
        .map(|(idx, entry)| {
            if Some(idx) == last_call_idx {
                return true;
            }
            match entry {
                SessionEntry::Message { message, .. } => match &**message {
                    AgentMessage::FunctionResult {
                        function_call_id, ..
                    } => last_call_ids.contains(&function_call_id.as_str()),
                    AgentMessage::Assistant { .. } => false,
                    // Anything else inside a run is a wake entry.
                    _ => true,
                },
                SessionEntry::Custom { .. } => true,
            }
        })
        .collect()
}

/// Strip the heavy parts of a message the collapsed view does not draw,
/// keeping everything it needs to draw the placeholder and to swap the full
/// entry in later: text (the visible phase summaries), every `function_call`
/// id and function id (the row's identity and label), every result's
/// `function_call_id` and `is_error` (pairing and status). Arguments,
/// result bodies, details and thinking are what a 300-call run is heavy
/// with, and exactly what `session::messages-range` brings back on demand.
pub fn elide_message(message: &mut AgentMessage) {
    match message {
        AgentMessage::Assistant { content, .. } => {
            content.retain(|b| !matches!(b, ContentBlock::Thinking { .. }));
            for block in content.iter_mut() {
                if let ContentBlock::FunctionCall { arguments, .. } = block {
                    *arguments = Value::Object(JsonMap::new());
                }
            }
        }
        AgentMessage::FunctionResult {
            content, details, ..
        } => {
            content.clear();
            *details = Value::Null;
        }
        AgentMessage::User { .. } | AgentMessage::Custom { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, message: Value) -> SessionEntry {
        serde_json::from_value(json!({
            "kind": "message", "id": id, "parent_id": null, "timestamp": 1, "message": message
        }))
        .unwrap()
    }

    fn user(id: &str) -> SessionEntry {
        entry(
            id,
            json!({ "role": "user", "content": [{ "type": "text", "text": "q" }], "timestamp": 1 }),
        )
    }

    fn prose(id: &str) -> SessionEntry {
        entry(
            id,
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "done" }],
                    "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 1 }),
        )
    }

    fn call(id: &str, call_id: &str, text: Option<&str>) -> SessionEntry {
        let mut content = vec![];
        if let Some(text) = text {
            content.push(json!({ "type": "text", "text": text }));
        }
        content.push(json!({ "type": "thinking", "text": "hmm" }));
        content.push(
            json!({ "type": "function_call", "id": call_id, "function_id": "shell::run",
                             "arguments": { "cmd": "ls" } }),
        );
        entry(
            id,
            json!({ "role": "assistant", "content": content, "stop_reason": "function_call",
                    "model": "m", "provider": "p", "timestamp": 1 }),
        )
    }

    fn result(id: &str, call_id: &str) -> SessionEntry {
        entry(
            id,
            json!({ "role": "function_result", "function_call_id": call_id, "function_id": "shell::run",
                    "content": [{ "type": "text", "text": "README.md" }], "details": { "code": 0 },
                    "is_error": false, "timestamp": 1 }),
        )
    }

    fn custom(id: &str, custom_type: &str) -> SessionEntry {
        serde_json::from_value(json!({
            "kind": "custom", "id": id, "parent_id": null, "timestamp": 1,
            "custom_type": custom_type, "data": {}
        }))
        .unwrap()
    }

    fn kinds(path: &[SessionEntry]) -> Vec<(usize, usize, BlockKind)> {
        let refs: Vec<&SessionEntry> = path.iter().collect();
        activity_blocks(&refs)
            .into_iter()
            .map(|b| (b.start, b.end, b.kind))
            .collect()
    }

    #[test]
    fn calls_results_and_wakes_form_one_run_between_prose() {
        let path = vec![
            user("u1"),
            call("a1", "c1", None),
            result("r1", "c1"),
            call("a2", "c2", Some("progress")),
            result("r2", "c2"),
            prose("a3"),
            user("u2"),
            // A wake pair right before the calls it caused stays with them.
            user("e_fire_sub_1_1"),
            custom("e_trigfired_sub_1_1", "trigger_fired"),
            call("a4", "c4", None),
            result("r4", "c4"),
        ];
        assert_eq!(
            kinds(&path),
            vec![
                (0, 1, BlockKind::Single),
                (1, 5, BlockKind::ActivityRun),
                (5, 6, BlockKind::Single),
                (6, 7, BlockKind::Single),
                (7, 11, BlockKind::ActivityRun),
            ]
        );
    }

    #[test]
    fn bookkeeping_entries_break_a_run() {
        // A compaction marker is a boundary the reader draws as its own row;
        // splitting there is always safe.
        let path = vec![
            call("a1", "c1", None),
            result("r1", "c1"),
            custom("k1", "compaction"),
            call("a2", "c2", None),
        ];
        assert_eq!(
            kinds(&path),
            vec![
                (0, 2, BlockKind::ActivityRun),
                (2, 3, BlockKind::Single),
                (3, 4, BlockKind::ActivityRun),
            ]
        );
        assert!(kinds(&[]).is_empty());
    }

    #[test]
    fn wake_flag_in_origin_counts_without_the_id_convention() {
        let mut wake = user("e_custom_id");
        if let SessionEntry::Message { origin, .. } = &mut wake {
            let mut o = JsonMap::new();
            o.insert("notification".into(), json!(true));
            *origin = Some(o);
        }
        let path = vec![wake, call("a1", "c1", None)];
        assert_eq!(kinds(&path), vec![(0, 2, BlockKind::ActivityRun)]);
    }

    #[test]
    fn collapsed_run_keeps_last_call_its_results_and_wakes() {
        let path = [
            user("e_fire_sub_1_1"),
            call("a1", "c1", Some("first")),
            result("r1", "c1"),
            call("a2", "c2", None),
            result("r2", "c2"),
            result("r2b", "c9"), // stray result for another call: elided
        ];
        let refs: Vec<&SessionEntry> = path.iter().collect();
        assert_eq!(
            kept_in_collapsed_run(&refs),
            vec![true, false, false, true, true, false]
        );
        // A run with no call entry at all (orphan results) keeps nothing but
        // wakes: every result is refetchable, nothing is the "last call".
        let orphans = [result("r1", "c1"), user("e_fire_x_1")];
        let refs: Vec<&SessionEntry> = orphans.iter().collect();
        assert_eq!(kept_in_collapsed_run(&refs), vec![false, true]);
    }

    #[test]
    fn elision_keeps_identity_and_text_but_drops_the_heavy_parts() {
        let SessionEntry::Message { message, .. } = call("a1", "c1", Some("summary")) else {
            unreachable!()
        };
        let mut message = *message;
        elide_message(&mut message);
        let round = serde_json::to_value(&message).unwrap();
        // Thinking gone, text kept, call keeps id + function id, arguments empty.
        assert_eq!(round["content"].as_array().unwrap().len(), 2);
        assert_eq!(round["content"][0]["text"], "summary");
        assert_eq!(round["content"][1]["id"], "c1");
        assert_eq!(round["content"][1]["function_id"], "shell::run");
        assert_eq!(round["content"][1]["arguments"], json!({}));
        assert_eq!(round["stop_reason"], "function_call");

        let SessionEntry::Message { message, .. } = result("r1", "c1") else {
            unreachable!()
        };
        let mut message = *message;
        elide_message(&mut message);
        let round = serde_json::to_value(&message).unwrap();
        assert_eq!(round["function_call_id"], "c1");
        assert_eq!(round["is_error"], false);
        assert_eq!(round["content"], json!([]));
        assert_eq!(round["details"], Value::Null);
    }
}
