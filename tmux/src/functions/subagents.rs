use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::Deserialize;

use super::panes::pane_arg;
use super::{register_fn, spec, Ctx, FunctionSpec};
use crate::activity::{self, Recorder};
use crate::model::SubagentTranscript;
use crate::transcript::MAX_ITEMS;

pub const TRANSCRIPT: &str = "tmux::subagents::transcript";

const TRANSCRIPT_DESC: &str = "What one sub-agent of the agent in a pane (Claude Code or Codex) has done so far, from the transcript the agent keeps on disk: the task it was given, what it wrote and reasoned, the tools it called and what they returned, oldest first. Works while the sub-agent runs (`working`) and after it finished, as long as the agent runs. Read-only.";

/// Items returned when `limit` is absent.
const DEFAULT_LIMIT: usize = 200;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TranscriptInput {
    /// Pane id of the agent, e.g. `%5`.
    pub pane: String,
    /// The sub-agent's id, from `agent.subagents[].id` in `tmux::sessions::list`.
    pub subagent: String,
    /// The most items returned, the latest ones: 200 when absent, at most 1000.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// A sub-agent id names a file, so it is a bare name: Claude Code's are hex,
/// Codex's are UUIDs.
fn subagent_arg(raw: &str) -> Result<&str, String> {
    let id = raw.trim();
    let bare = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if bare {
        Ok(id)
    } else {
        Err(format!(
            "subagent must be an id from agent.subagents, got '{raw}'"
        ))
    }
}

pub async fn transcript(ctx: &Ctx, input: TranscriptInput) -> Result<SubagentTranscript, String> {
    let pane = pane_arg(&input.pane)?.to_string();
    let id = subagent_arg(&input.subagent)?.to_string();
    let limit = input
        .limit
        .map_or(DEFAULT_LIMIT, |limit| limit as usize)
        .clamp(1, MAX_ITEMS);
    let agent = ctx
        .collector
        .pane_agent(&pane)
        .await
        .ok_or_else(|| format!("pane {pane} runs no agent"))?;
    let recorder = Recorder::of(&agent.id)
        .ok_or_else(|| format!("{} keeps no sub-agent transcripts", agent.label))?;
    let table = match recorder {
        Recorder::Codex => Some(ctx.collector.proc_table().await),
        Recorder::Claude => None,
    };
    let transcripts = ctx.transcripts.clone();
    let (pid, file_id) = (agent.pid, id.clone());
    let view = tokio::task::spawn_blocking(move || {
        let path = match &table {
            Some(table) => activity::codex_thread_file(table, pid, &file_id),
            None => activity::claude_subagent_file(pid, &file_id),
        }
        .ok_or_else(|| format!("no transcript for sub-agent {file_id}"))?;
        transcripts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .read(recorder, &path, limit)
            .map_err(|error| format!("reading {}: {error}", path.display()))
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(SubagentTranscript {
        working: agent.subagents.iter().any(|sub| sub.id == id),
        pane_id: pane,
        agent: agent.id,
        subagent_id: id,
        prompt: view.prompt,
        items: view.items,
        truncated: view.truncated,
    })
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![spec::<TranscriptInput, SubagentTranscript>(
        TRANSCRIPT,
        TRANSCRIPT_DESC,
    )]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(
        iii,
        ctx,
        TRANSCRIPT,
        TRANSCRIPT_DESC,
        TranscriptInput,
        transcript
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_ids_are_bare_names() {
        assert_eq!(
            subagent_arg(" a09cd1aaf64a55b3b ").unwrap(),
            "a09cd1aaf64a55b3b"
        );
        assert!(subagent_arg("01a1652b-754a-7e83-9583-4e9a8d1349cd").is_ok());
        assert!(subagent_arg("").is_err());
        assert!(subagent_arg("../../etc/passwd").is_err());
        assert!(subagent_arg("a/b").is_err());
        assert!(subagent_arg(&"a".repeat(65)).is_err());
    }
}
