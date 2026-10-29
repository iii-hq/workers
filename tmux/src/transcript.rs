//! A sub-agent's transcript for a reader: the task it was given, what it
//! wrote and thought, the tools it called and what they returned. Claude
//! Code writes `subagents/agent-<id>.jsonl` next to the session transcript;
//! Codex writes one `rollout-…-<thread id>.jsonl` per thread. A transcript
//! is first read from its last [`WINDOW`] bytes, then only what was added
//! since, so a viewer that asks every couple of seconds costs little.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Deserialize;
use serde_json::Value;

use crate::activity::Recorder;
use crate::model::{TranscriptItem, TranscriptItemKind};

/// How far back the first read of a transcript starts, and the most read
/// at once after that.
const WINDOW: u64 = 4 * 1024 * 1024;
/// The most items kept per transcript.
pub const MAX_ITEMS: usize = 1000;
/// How far into a transcript its task is looked for when the window starts
/// later (a Codex rollout opens with a multi-megabyte `session_meta` line).
const HEAD_LIMIT: u64 = 16 * 1024 * 1024;
/// An item's text is cut after this many characters.
const TEXT_LIMIT: usize = 4_000;
/// The task is cut after this many characters.
const PROMPT_LIMIT: usize = 16_000;
/// Transcripts followed at once; the one read longest ago is dropped.
const FOLLOWED: usize = 16;
/// Tool call ids remembered to name the results that follow them.
const TOOL_NAMES: usize = 4_096;

/// The part of a transcript a reader is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptView {
    pub prompt: Option<String>,
    pub items: Vec<TranscriptItem>,
    pub truncated: bool,
}

/// The transcripts being followed, by path.
#[derive(Debug, Default)]
pub struct Transcripts {
    tails: HashMap<PathBuf, Tail>,
}

impl Transcripts {
    /// The task and the last `limit` items of the transcript at `path`.
    /// Blocking.
    pub fn read(
        &mut self,
        recorder: Recorder,
        path: &Path,
        limit: usize,
    ) -> io::Result<TranscriptView> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        // A shorter file is another file under the same name.
        if self.tails.get(path).is_some_and(|tail| tail.read_to > len) {
            self.tails.remove(path);
        }
        if !self.tails.contains_key(path) {
            if self.tails.len() >= FOLLOWED {
                let oldest = self
                    .tails
                    .iter()
                    .min_by_key(|(_, tail)| tail.used)
                    .map(|(path, _)| path.clone());
                if let Some(oldest) = oldest {
                    self.tails.remove(&oldest);
                }
            }
            self.tails
                .insert(path.to_path_buf(), Tail::open(recorder, &mut file, len));
        }
        let tail = self.tails.get_mut(path).expect("inserted above");
        tail.follow(recorder, &mut file, len)?;
        tail.used = Instant::now();
        let skip = tail.items.len().saturating_sub(limit);
        Ok(TranscriptView {
            prompt: tail.prompt.clone(),
            items: tail.items.iter().skip(skip).cloned().collect(),
            truncated: tail.truncated || skip > 0,
        })
    }
}

#[derive(Debug)]
struct Tail {
    /// Where the next read starts.
    read_to: u64,
    /// `read_to` is inside a line, whose rest is skipped.
    mid_line: bool,
    items: VecDeque<TranscriptItem>,
    truncated: bool,
    prompt: Option<String>,
    /// The task was already found, or looked for from the file's start.
    prompt_known: bool,
    /// Just past the task's line: lines before it are not items.
    prompt_end: u64,
    context: LineContext,
    used: Instant,
}

impl Tail {
    fn open(recorder: Recorder, file: &mut File, len: u64) -> Self {
        let mut tail = Self {
            read_to: 0,
            mid_line: false,
            items: VecDeque::new(),
            truncated: false,
            prompt: None,
            prompt_known: false,
            prompt_end: 0,
            context: LineContext::default(),
            used: Instant::now(),
        };
        if len > WINDOW {
            tail.skip_to(recorder, file, len - WINDOW);
        }
        tail
    }

    /// Starts reading at `offset`, inside a line, leaving out what comes
    /// before; the task is looked for from the file's start.
    fn skip_to(&mut self, recorder: Recorder, file: &File, offset: u64) {
        self.items.clear();
        self.truncated = true;
        self.read_to = offset;
        self.mid_line = true;
        if !self.prompt_known {
            self.prompt_known = true;
            if let Some((prompt, end)) = head_prompt(recorder, file) {
                self.prompt = Some(prompt);
                self.prompt_end = end;
            }
        }
    }

    /// Reads the complete lines added since the last read.
    fn follow(&mut self, recorder: Recorder, file: &mut File, len: u64) -> io::Result<()> {
        if len <= self.read_to {
            return Ok(());
        }
        if len - self.read_to > WINDOW {
            // Too far behind: start over from the last window.
            self.skip_to(recorder, file, len - WINDOW);
        }
        file.seek(SeekFrom::Start(self.read_to))?;
        let mut buf = vec![0; (len - self.read_to) as usize];
        file.read_exact(&mut buf)?;
        // A line still being written is read again once it is complete.
        let Some(end) = buf.iter().rposition(|b| *b == b'\n') else {
            return Ok(());
        };
        let mut at = 0;
        if self.mid_line {
            at = buf.iter().position(|b| *b == b'\n').unwrap_or(end) + 1;
        }
        while at <= end {
            let stop = at
                + buf[at..]
                    .iter()
                    .position(|b| *b == b'\n')
                    .unwrap_or(end - at);
            if self.read_to + at as u64 >= self.prompt_end {
                self.line(recorder, &buf[at..stop]);
            }
            at = stop + 1;
        }
        self.read_to += end as u64 + 1;
        self.mid_line = false;
        Ok(())
    }

    fn line(&mut self, recorder: Recorder, line: &[u8]) {
        let mut items = Vec::new();
        parse_line(recorder, line, &mut self.context, &mut items);
        for Parsed { item, task } in items {
            if !self.prompt_known {
                if task {
                    self.prompt_known = true;
                    self.prompt = Some(cut(item.text, PROMPT_LIMIT));
                    continue;
                }
                // A Codex sub-agent is a fork: its rollout opens with the
                // history of the agent that spawned it.
                if recorder == Recorder::Codex {
                    continue;
                }
            }
            if self.items.len() == MAX_ITEMS {
                self.items.pop_front();
                self.truncated = true;
            }
            self.items.push_back(TranscriptItem {
                text: cut(item.text, TEXT_LIMIT),
                ..item
            });
        }
    }
}

/// The first message to the sub-agent, looked for in the first
/// [`HEAD_LIMIT`] bytes, and where its line ends.
fn head_prompt(recorder: Recorder, file: &File) -> Option<(String, u64)> {
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(0)).ok()?;
    let mut reader = reader.take(HEAD_LIMIT);
    let mut context = LineContext::default();
    let mut line = Vec::new();
    let mut end = 0;
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).ok()?;
        if read == 0 || !line.ends_with(b"\n") {
            return None;
        }
        end += read as u64;
        let mut items = Vec::new();
        parse_line(recorder, &line, &mut context, &mut items);
        if let Some(Parsed { item, .. }) = items.into_iter().find(|parsed| parsed.task) {
            return Some((cut(item.text, PROMPT_LIMIT), end));
        }
    }
}

/// What one line says depends on the lines before it.
#[derive(Debug, Default)]
struct LineContext {
    /// Tool names by call id, to name the results that follow.
    tools: HashMap<String, String>,
    /// Codex: the thread's own `session_meta` (the first one) was read.
    meta_read: bool,
    /// Codex: the thread's path (`/root/review`), to which its task is sent.
    agent_path: Option<String>,
}

struct Parsed {
    item: TranscriptItem,
    /// The item is the sub-agent's task: Claude Code's first message,
    /// Codex's first message from another agent to this thread.
    task: bool,
}

/// Only the `type` of a line, so that the many lines a reader is not shown
/// are skipped without building them.
#[derive(Deserialize)]
struct LineType {
    #[serde(rename = "type")]
    kind: Option<String>,
}

/// A Codex `session_meta` line without its instructions.
#[derive(Deserialize)]
struct CodexMeta {
    payload: CodexMetaPayload,
}

#[derive(Deserialize)]
struct CodexMetaPayload {
    #[serde(default)]
    source: Value,
}

fn parse_line(recorder: Recorder, line: &[u8], context: &mut LineContext, out: &mut Vec<Parsed>) {
    let Ok(LineType { kind: Some(kind) }) = serde_json::from_slice::<LineType>(line) else {
        return;
    };
    if recorder == Recorder::Codex && kind == "session_meta" && !context.meta_read {
        context.meta_read = true;
        context.agent_path = serde_json::from_slice::<CodexMeta>(line)
            .ok()
            .and_then(|meta| {
                meta.payload
                    .source
                    .pointer("/subagent/thread_spawn/agent_path")?
                    .as_str()
                    .map(str::to_string)
            });
        return;
    }
    let wanted = match recorder {
        Recorder::Claude => kind == "user" || kind == "assistant",
        Recorder::Codex => kind == "response_item",
    };
    if !wanted {
        return;
    }
    let Ok(entry) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    let at_ms = entry
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(rfc3339_ms);
    let LineContext {
        tools, agent_path, ..
    } = context;
    let mut items = Items { out, at_ms };
    match recorder {
        Recorder::Claude => claude_items(&entry, tools, &mut items),
        Recorder::Codex => codex_items(&entry, tools, agent_path.as_deref(), &mut items),
    }
    if tools.len() > TOOL_NAMES {
        tools.clear();
    }
}

/// The items of one line.
struct Items<'a> {
    out: &'a mut Vec<Parsed>,
    at_ms: Option<u64>,
}

impl Items<'_> {
    fn push(&mut self, kind: TranscriptItemKind, text: String, tool: Option<String>, error: bool) {
        self.add(kind, text, tool, error, false);
    }

    /// A message that is the sub-agent's task if no other came before it.
    fn task(&mut self, text: String) {
        self.add(TranscriptItemKind::User, text, None, false, true);
    }

    fn add(
        &mut self,
        kind: TranscriptItemKind,
        text: String,
        tool: Option<String>,
        error: bool,
        task: bool,
    ) {
        self.out.push(Parsed {
            item: TranscriptItem {
                kind,
                at_ms: self.at_ms,
                text: text.trim().to_string(),
                tool,
                error,
            },
            task,
        });
    }
}

/// A Claude Code entry: `user` entries carry the task, messages from the
/// main agent and tool results; `assistant` entries one block each of
/// text, thinking or a tool call.
fn claude_items(entry: &Value, tools: &mut HashMap<String, String>, items: &mut Items) {
    let content = entry.pointer("/message/content");
    match entry.get("type").and_then(Value::as_str) {
        Some("user") => {
            // Meta entries are reminders the agent adds, except those with an
            // origin: messages the main agent sent while the sub-agent worked.
            let meta = entry.get("isMeta").and_then(Value::as_bool) == Some(true);
            if meta && !entry.get("origin").is_some_and(Value::is_object) {
                return;
            }
            match content {
                Some(Value::String(text)) => items.task(text.clone()),
                Some(Value::Array(blocks)) => {
                    for block in blocks {
                        match block.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                let text = str_at(block, "text");
                                if !text.starts_with("<system-reminder>") {
                                    items.task(text.to_string());
                                }
                            }
                            Some("tool_result") => items.push(
                                TranscriptItemKind::ToolResult,
                                content_text(block.get("content")),
                                tools.get(str_at(block, "tool_use_id")).cloned(),
                                block.get("is_error").and_then(Value::as_bool) == Some(true),
                            ),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Some("assistant") => {
            let Some(Value::Array(blocks)) = content else {
                return;
            };
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => items.push(
                        TranscriptItemKind::Text,
                        str_at(block, "text").to_string(),
                        None,
                        false,
                    ),
                    // Empty when the agent keeps its thinking to itself.
                    Some("thinking") if !str_at(block, "thinking").trim().is_empty() => items.push(
                        TranscriptItemKind::Thinking,
                        str_at(block, "thinking").to_string(),
                        None,
                        false,
                    ),
                    Some("tool_use") => {
                        let name = str_at(block, "name").to_string();
                        tools.insert(str_at(block, "id").to_string(), name.clone());
                        items.push(
                            TranscriptItemKind::ToolCall,
                            call_summary(block.get("input").unwrap_or(&Value::Null)),
                            Some(name),
                            false,
                        );
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// A Codex `response_item`: messages, the task another agent sent, reasoning
/// summaries, and tool calls with their outputs. `agent_path` is the
/// thread's own: its task is the first message sent to it.
fn codex_items(
    entry: &Value,
    tools: &mut HashMap<String, String>,
    agent_path: Option<&str>,
    items: &mut Items,
) {
    let Some(payload) = entry.get("payload") else {
        return;
    };
    let call_id = str_at(payload, "call_id");
    match payload.get("type").and_then(Value::as_str) {
        Some("message") => {
            let text = content_text(payload.get("content"));
            match payload.get("role").and_then(Value::as_str) {
                Some("assistant") => items.push(TranscriptItemKind::Text, text, None, false),
                // What Codex adds as user turns (`<environment_context>`,
                // instructions) is not a message.
                Some("user") if !text.trim_start().starts_with('<') => match agent_path {
                    None => items.task(text),
                    Some(_) => items.push(TranscriptItemKind::User, text, None, false),
                },
                _ => {}
            }
        }
        // Another agent's message: this thread's task, a later message, or,
        // in the history a forked thread starts with, one to another thread.
        Some("agent_message") => {
            let text = content_text(payload.get("content"));
            let recipient = payload.get("recipient").and_then(Value::as_str);
            if agent_path.is_none() || recipient.is_none() || recipient == agent_path {
                items.task(text);
            } else {
                items.push(TranscriptItemKind::User, text, None, false);
            }
        }
        Some("reasoning") => {
            let text = content_text(payload.get("summary"));
            if !text.trim().is_empty() {
                items.push(TranscriptItemKind::Thinking, text, None, false);
            }
        }
        Some("function_call") => {
            let name = str_at(payload, "name").to_string();
            tools.insert(call_id.to_string(), name.clone());
            let arguments = str_at(payload, "arguments");
            let summary = serde_json::from_str::<Value>(arguments)
                .map(|input| call_summary(&input))
                .unwrap_or_else(|_| arguments.to_string());
            items.push(TranscriptItemKind::ToolCall, summary, Some(name), false);
        }
        Some("custom_tool_call") => {
            let name = str_at(payload, "name").to_string();
            tools.insert(call_id.to_string(), name.clone());
            items.push(
                TranscriptItemKind::ToolCall,
                str_at(payload, "input").to_string(),
                Some(name),
                false,
            );
        }
        Some("local_shell_call") => {
            tools.insert(call_id.to_string(), "shell".to_string());
            items.push(
                TranscriptItemKind::ToolCall,
                call_summary(payload.get("action").unwrap_or(&Value::Null)),
                Some("shell".to_string()),
                false,
            );
        }
        Some("web_search_call") => items.push(
            TranscriptItemKind::ToolCall,
            call_summary(payload.get("action").unwrap_or(&Value::Null)),
            Some("web_search".to_string()),
            false,
        ),
        Some("function_call_output" | "custom_tool_call_output") => {
            let output = payload.get("output");
            let failed = output
                .and_then(|output| output.get("success"))
                .and_then(Value::as_bool)
                == Some(false);
            items.push(
                TranscriptItemKind::ToolResult,
                content_text(output),
                tools.get(call_id).cloned(),
                failed,
            );
        }
        _ => {}
    }
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Text from a string, from a list of parts (`text`, `input_text`,
/// `output_text`, `summary_text`), or from an object's `content`.
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.trim_end().to_string()),
                Value::Object(_) => match part.get("type").and_then(Value::as_str) {
                    Some("image" | "input_image") => Some("[image]".to_string()),
                    Some("encrypted_content") => Some("[encrypted]".to_string()),
                    _ => part
                        .get("text")
                        .and_then(Value::as_str)
                        .map(|text| text.trim_end().to_string()),
                },
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Object(object)) => content_text(object.get("content")),
        _ => String::new(),
    }
}

/// The argument of a tool call that says what it does, else its input as
/// JSON.
fn call_summary(input: &Value) -> String {
    const KEYS: [&str; 10] = [
        "command",
        "cmd",
        "file_path",
        "pattern",
        "path",
        "url",
        "query",
        "description",
        "prompt",
        "skill",
    ];
    match input {
        Value::String(text) => text.clone(),
        Value::Object(object) => KEYS
            .iter()
            .find_map(|key| match object.get(*key)? {
                Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
                Value::Array(parts) if parts.iter().all(Value::is_string) && !parts.is_empty() => {
                    Some(
                        parts
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                }
                _ => None,
            })
            .unwrap_or_else(|| input.to_string()),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `text` cut after `limit` characters, ending in `…` when it was.
fn cut(mut text: String, limit: usize) -> String {
    if let Some((index, _)) = text.char_indices().nth(limit) {
        text.truncate(index);
        text.push('…');
    }
    text
}

/// Milliseconds since the epoch of an RFC 3339 time such as
/// `2026-10-22T21:01:15.502Z` or `…+02:00`.
fn rfc3339_ms(text: &str) -> Option<u64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let mut rest = text.get(19..)?;
    let mut millis = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        millis = fraction[..digits]
            .chars()
            .chain(std::iter::repeat('0'))
            .take(3)
            .collect::<String>()
            .parse::<i64>()
            .ok()?;
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let hours = rest.get(1..3)?.parse::<i64>().ok()?;
            let minutes = rest.get(4..6)?.parse::<i64>().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    u64::try_from(seconds * 1000 + millis).ok()
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use TranscriptItemKind::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "tmux-transcript-{name}-{}-{}",
                std::process::id(),
                Instant::now().elapsed().as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str, lines: &[String]) -> PathBuf {
            let path = self.0.join(name);
            let mut text = lines.join("\n");
            text.push('\n');
            std::fs::write(&path, text).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn append(path: &Path, text: &str) {
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn kinds(view: &TranscriptView) -> Vec<(TranscriptItemKind, &str, Option<&str>)> {
        view.items
            .iter()
            .map(|item| (item.kind, item.text.as_str(), item.tool.as_deref()))
            .collect()
    }

    fn claude(kind: &str, content: Value, extra: Value) -> String {
        let mut entry = serde_json::json!({
            "parentUuid": null,
            "isSidechain": true,
            "type": kind,
            "message": {"role": kind, "content": content},
            "timestamp": "2026-10-22T21:01:15.502Z",
        });
        if let (Value::Object(entry), Value::Object(extra)) = (&mut entry, extra) {
            entry.extend(extra);
        }
        entry.to_string()
    }

    fn claude_transcript() -> Vec<String> {
        vec![
            claude("user", "Review the signing keys".into(), Value::Null),
            r#"{"type":"attachment","attachment":{"type":"skill_listing"}}"#.to_string(),
            r#"{"type":"api-request-blob","message":{"role":"user","content":[{"type":"text","text":"not shown"}]}}"#
                .to_string(),
            claude(
                "assistant",
                serde_json::json!([{"type": "thinking", "thinking": ""}]),
                Value::Null,
            ),
            claude(
                "assistant",
                serde_json::json!([{"type": "thinking", "thinking": "Start with the key store."}]),
                Value::Null,
            ),
            claude(
                "assistant",
                serde_json::json!([{"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls keys/", "description": "List keys"}}]),
                Value::Null,
            ),
            claude(
                "user",
                serde_json::json!([{"type": "tool_result", "tool_use_id": "toolu_1", "content": "a.pem\nb.pem\n"}]),
                Value::Null,
            ),
            claude(
                "assistant",
                serde_json::json!([{"type": "tool_use", "id": "toolu_2", "name": "Read", "input": {"file_path": "/k/a.pem"}}]),
                Value::Null,
            ),
            claude(
                "user",
                serde_json::json!([{"type": "tool_result", "tool_use_id": "toolu_2", "is_error": true, "content": [{"type": "text", "text": "EACCES"}]}]),
                Value::Null,
            ),
            claude(
                "user",
                "<system-reminder>internal</system-reminder>".into(),
                serde_json::json!({"isMeta": true}),
            ),
            claude(
                "user",
                "The coordinator sent a message while you were working:\nskip b.pem".into(),
                serde_json::json!({"isMeta": true, "origin": {"kind": "coordinator"}}),
            ),
            claude(
                "assistant",
                serde_json::json!([{"type": "text", "text": "a.pem is unreadable."}]),
                serde_json::json!({"message": {"role": "assistant", "stop_reason": "end_turn", "content": [{"type": "text", "text": "a.pem is unreadable."}]}}),
            ),
        ]
    }

    #[test]
    fn claude_transcripts_read_as_items() {
        let scratch = Scratch::new("claude");
        let path = scratch.file("agent-a1.jsonl", &claude_transcript());
        let view = Transcripts::default()
            .read(Recorder::Claude, &path, 100)
            .unwrap();
        assert_eq!(view.prompt.as_deref(), Some("Review the signing keys"));
        assert!(!view.truncated);
        assert_eq!(
            kinds(&view),
            vec![
                (Thinking, "Start with the key store.", None),
                (ToolCall, "ls keys/", Some("Bash")),
                (ToolResult, "a.pem\nb.pem", Some("Bash")),
                (ToolCall, "/k/a.pem", Some("Read")),
                (ToolResult, "EACCES", Some("Read")),
                (
                    User,
                    "The coordinator sent a message while you were working:\nskip b.pem",
                    None
                ),
                (Text, "a.pem is unreadable.", None),
            ]
        );
        assert!(view.items[4].error);
        assert!(!view.items[2].error);
        assert_eq!(view.items[0].at_ms, Some(1_792_702_875_502));
    }

    #[test]
    fn only_added_lines_are_read_and_a_half_written_line_waits() {
        let scratch = Scratch::new("follow");
        let lines = claude_transcript();
        let path = scratch.file("agent-a2.jsonl", &lines[..6]);
        let mut transcripts = Transcripts::default();
        let first = transcripts.read(Recorder::Claude, &path, 100).unwrap();
        assert_eq!(first.items.len(), 2);

        let (head, rest) = lines[6].split_at(20);
        append(&path, head);
        let same = transcripts.read(Recorder::Claude, &path, 100).unwrap();
        assert_eq!(same, first);

        append(&path, &format!("{rest}\n{}\n", lines[7]));
        let grown = transcripts.read(Recorder::Claude, &path, 100).unwrap();
        assert_eq!(
            kinds(&grown)[2..],
            [
                (ToolResult, "a.pem\nb.pem", Some("Bash")),
                (ToolCall, "/k/a.pem", Some("Read")),
            ]
        );

        let latest = transcripts.read(Recorder::Claude, &path, 1).unwrap();
        assert_eq!(kinds(&latest), vec![(ToolCall, "/k/a.pem", Some("Read"))]);
        assert!(latest.truncated);

        // A rewritten, shorter file is read from its start again.
        std::fs::write(&path, format!("{}\n", lines[0])).unwrap();
        let again = transcripts.read(Recorder::Claude, &path, 100).unwrap();
        assert_eq!(again.prompt.as_deref(), Some("Review the signing keys"));
        assert!(again.items.is_empty());
    }

    #[test]
    fn a_long_transcript_keeps_its_task_and_its_last_window() {
        let scratch = Scratch::new("long");
        let filler = claude(
            "user",
            serde_json::json!([{"type": "tool_result", "tool_use_id": "t", "content": "x".repeat(64 * 1024)}]),
            Value::Null,
        );
        let mut lines = vec![claude("user", "The task".into(), Value::Null)];
        lines.extend(std::iter::repeat_n(
            filler,
            (WINDOW / (64 * 1024)) as usize + 8,
        ));
        lines.push(claude(
            "assistant",
            serde_json::json!([{"type": "text", "text": "done"}]),
            Value::Null,
        ));
        let path = scratch.file("agent-a3.jsonl", &lines);
        let view = Transcripts::default()
            .read(Recorder::Claude, &path, MAX_ITEMS)
            .unwrap();
        assert_eq!(view.prompt.as_deref(), Some("The task"));
        assert!(view.truncated);
        assert_eq!(view.items.last().unwrap().text, "done");
        let result = &view.items[0];
        assert_eq!(result.kind, ToolResult);
        assert_eq!(result.text.chars().count(), TEXT_LIMIT + 1);
        assert!(result.text.ends_with('…'));
    }

    fn codex(payload: Value) -> String {
        serde_json::json!({"timestamp": "2026-10-22T19:51:55.346Z", "type": "response_item", "payload": payload})
            .to_string()
    }

    #[test]
    fn codex_rollouts_read_as_items() {
        let scratch = Scratch::new("codex");
        let lines = vec![
            format!(
                r#"{{"timestamp":"2026-10-22T19:51:49.354Z","type":"session_meta","payload":{{"id":"t1","base_instructions":{{"text":"{}"}}}}}}"#,
                "i".repeat(1024)
            ),
            r#"{"timestamp":"2026-10-22T19:51:49.354Z","type":"event_msg","payload":{"type":"task_started"}}"#
                .to_string(),
            codex(serde_json::json!({"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "<permissions>"}]})),
            codex(serde_json::json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>\n</environment_context>"}]})),
            codex(serde_json::json!({"type": "agent_message", "author": "/root", "content": [
                {"type": "input_text", "text": "Message Type: NEW_TASK\nTask name: /root/review\nSender: /root\nPayload:\n"},
                {"type": "encrypted_content", "encrypted_content": "gAAAA"}
            ]})),
            codex(serde_json::json!({"type": "reasoning", "summary": [{"type": "summary_text", "text": "Check the tests first."}], "encrypted_content": "gAAAA"})),
            codex(serde_json::json!({"type": "reasoning", "summary": [], "encrypted_content": "gAAAA"})),
            codex(serde_json::json!({"type": "custom_tool_call", "call_id": "c1", "name": "exec", "input": "text(await tools.exec_command({cmd:\"sleep 40\"}));\n"})),
            codex(serde_json::json!({"type": "custom_tool_call_output", "call_id": "c1", "output": [{"type": "input_text", "text": "Script completed\n"}, {"type": "input_text", "text": "{\"exit_code\":0}"}]})),
            codex(serde_json::json!({"type": "function_call", "call_id": "c2", "name": "shell", "arguments": "{\"command\":[\"bash\",\"-lc\",\"cargo test\"]}"})),
            codex(serde_json::json!({"type": "function_call_output", "call_id": "c2", "output": {"content": "1 failed", "success": false}})),
            codex(serde_json::json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "done"}]})),
        ];
        let path = scratch.file("rollout-2026-10-22T16-51-49-t1.jsonl", &lines);
        let view = Transcripts::default()
            .read(Recorder::Codex, &path, 100)
            .unwrap();
        assert_eq!(
            view.prompt.as_deref(),
            Some("Message Type: NEW_TASK\nTask name: /root/review\nSender: /root\nPayload:\n[encrypted]")
        );
        assert_eq!(
            kinds(&view),
            vec![
                (Thinking, "Check the tests first.", None),
                (
                    ToolCall,
                    "text(await tools.exec_command({cmd:\"sleep 40\"}));",
                    Some("exec")
                ),
                (
                    ToolResult,
                    "Script completed\n{\"exit_code\":0}",
                    Some("exec")
                ),
                (ToolCall, "bash -lc cargo test", Some("shell")),
                (ToolResult, "1 failed", Some("shell")),
                (Text, "done", None),
            ]
        );
        assert!(view.items[4].error);
    }

    #[test]
    fn the_task_is_found_past_a_long_first_line() {
        let scratch = Scratch::new("codex-head");
        let meta = format!(
            r#"{{"timestamp":"2026-10-22T19:51:49.354Z","type":"session_meta","payload":{{"base_instructions":{{"text":"{}"}}}}}}"#,
            "i".repeat(WINDOW as usize + 1024)
        );
        let task = codex(
            serde_json::json!({"type": "agent_message", "content": [{"type": "input_text", "text": "Task name: /root/review"}]}),
        );
        let answer = codex(
            serde_json::json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "ok"}]}),
        );
        let mut lines = vec![meta, task];
        lines.extend(std::iter::repeat_n(answer, 3));
        let path = scratch.file("rollout-x-t2.jsonl", &lines);
        let view = Transcripts::default()
            .read(Recorder::Codex, &path, 100)
            .unwrap();
        assert_eq!(view.prompt.as_deref(), Some("Task name: /root/review"));
        assert!(view.truncated);
        assert_eq!(kinds(&view), vec![(Text, "ok", None); 3]);
    }

    #[test]
    fn a_forked_codex_thread_starts_at_its_own_task() {
        let scratch = Scratch::new("codex-fork");
        let meta = |source: Value| {
            serde_json::json!({"timestamp": "2026-10-23T12:06:38.000Z", "type": "session_meta", "payload": {"source": source, "base_instructions": {"text": "…"}}})
                .to_string()
        };
        let message = |author: &str, recipient: &str, text: &str| {
            codex(
                serde_json::json!({"type": "agent_message", "author": author, "recipient": recipient, "content": [{"type": "input_text", "text": text}]}),
            )
        };
        let lines = vec![
            meta(
                serde_json::json!({"subagent": {"thread_spawn": {"depth": 2, "agent_path": "/root/review/tests"}}}),
            ),
            meta(
                serde_json::json!({"subagent": {"thread_spawn": {"depth": 1, "agent_path": "/root/review"}}}),
            ),
            // The history of /root/review, which spawned this thread.
            codex(
                serde_json::json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Review the PR"}]}),
            ),
            message("/root", "/root/review", "Task name: /root/review"),
            codex(
                serde_json::json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Spawning a tester."}]}),
            ),
            // This thread's own work.
            message(
                "/root/review",
                "/root/review/tests",
                "Task name: /root/review/tests",
            ),
            codex(
                serde_json::json!({"type": "function_call", "call_id": "c1", "name": "shell", "arguments": "{\"command\":[\"cargo\",\"test\"]}"}),
            ),
            message("/root/review", "/root/review/tests", "Only the tmux crate"),
        ];
        let path = scratch.file("rollout-x-t3.jsonl", &lines);
        let view = Transcripts::default()
            .read(Recorder::Codex, &path, 100)
            .unwrap();
        assert_eq!(
            view.prompt.as_deref(),
            Some("Task name: /root/review/tests")
        );
        assert_eq!(
            kinds(&view),
            vec![
                (ToolCall, "cargo test", Some("shell")),
                (User, "Only the tmux crate", None),
            ]
        );
    }

    #[test]
    fn summaries_name_what_a_call_does() {
        let summary = |input: Value| call_summary(&input);
        assert_eq!(summary(serde_json::json!({"command": "ls"})), "ls");
        assert_eq!(
            summary(serde_json::json!({"pattern": "fn main", "path": "src"})),
            "fn main"
        );
        assert_eq!(summary(serde_json::json!({"path": "src"})), "src");
        assert_eq!(
            summary(serde_json::json!({"type": "search", "query": "tmux"})),
            "tmux"
        );
        assert_eq!(
            summary(serde_json::json!({"todos": [1]})),
            r#"{"todos":[1]}"#
        );
        assert_eq!(summary(Value::Null), "");
    }

    #[test]
    fn times_and_cuts() {
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_ms("2026-10-22T21:01:15.502Z"),
            Some(1_792_702_875_502)
        );
        assert_eq!(
            rfc3339_ms("2026-10-22T21:01:15.5Z"),
            Some(1_792_702_875_500)
        );
        assert_eq!(
            rfc3339_ms("2026-10-22T23:01:15.502123+02:00"),
            Some(1_792_702_875_502)
        );
        assert_eq!(rfc3339_ms("2024-02-29T00:00:00Z"), Some(1_709_164_800_000));
        assert_eq!(rfc3339_ms("yesterday"), None);
        assert_eq!(cut("héllo".to_string(), 2), "hé…");
        assert_eq!(cut("hé".to_string(), 2), "hé");
    }
}
