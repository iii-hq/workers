use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{bail, ensure, Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const PAGE_SIZE: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Codex,
    ClaudeCode,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
        }
    }

    fn root(self) -> Result<PathBuf> {
        let variable = match self {
            Self::Codex => "CODEX_HOME",
            Self::ClaudeCode => "CLAUDE_CONFIG_DIR",
        };
        if let Some(path) = std::env::var_os(variable).filter(|p| !p.is_empty()) {
            return Ok(path.into());
        }
        let user_dir = std::env::var_os("HOME").context("User home directory is unavailable")?;
        Ok(PathBuf::from(user_dir).join(match self {
            Self::Codex => ".codex",
            Self::ClaudeCode => ".claude",
        }))
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ConversationSummary {
    pub id: String,
    pub source: Source,
    pub title: String,
    pub cwd: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExternalMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    pub timestamp: i64,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub native_stop_reason: Option<String>,
}

pub struct Transcript {
    pub conversation: ConversationSummary,
    pub messages: Vec<ExternalMessage>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct Catalog {
    pub source: Source,
    pub directory: String,
    pub available: bool,
    pub conversations: Vec<ConversationSummary>,
    pub next_cursor: Option<String>,
    pub warnings: Vec<String>,
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn timestamp(value: &Value) -> Result<i64> {
    let value = value.as_str().context("Message timestamp is missing")?;
    Ok(chrono::DateTime::parse_from_rfc3339(value)
        .context("Invalid message timestamp")?
        .timestamp_millis())
}

fn content_text(content: &Value, block_type: &str) -> String {
    if let Some(value) = content.as_str() {
        return value.to_owned();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| text(block, "type") == Some(block_type))
        .filter_map(|block| text(block, "text"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn records(mut reader: impl BufRead, mut visit: impl FnMut(Value) -> Result<()>) -> Result<bool> {
    let mut line = Vec::new();
    let mut index = 0;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(false);
        }
        index += 1;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice(&line) {
            Ok(row) => visit(row)?,
            Err(_) if !line.ends_with(b"\n") => return Ok(true),
            Err(error) => return Err(error).context(format!("Invalid history at line {index}")),
        }
    }
}

fn parse(source: Source, id: &str, reader: impl BufRead, modified_at: i64) -> Result<Transcript> {
    let mut messages = Vec::new();
    let mut cwd = None;
    let mut title = None;
    let incomplete = match source {
        Source::Codex => {
            let mut has_meta = false;
            let mut provider = None;
            let mut model = None;
            let mut seen = HashSet::new();
            let incomplete = records(reader, |row| {
                let payload = &row["payload"];
                match text(&row, "type") {
                    Some("session_meta") => {
                        ensure!(
                            text(payload, "id") == Some(id),
                            "Session identity does not match its history file"
                        );
                        ensure!(
                            payload.get("agent_path").is_none(),
                            "Subagent histories are not supported in this import"
                        );
                        cwd = text(payload, "cwd").map(str::to_owned);
                        provider = text(payload, "model_provider").map(str::to_owned);
                        has_meta = true;
                    }
                    Some("turn_context") => model = text(payload, "model").map(str::to_owned),
                    Some("event_msg") => {
                        ensure!(
                            text(payload, "type") != Some("thread_rolled_back"),
                            "Rewound Codex histories are not supported in this import"
                        );
                        if text(payload, "type") != Some("item_completed") {
                            return Ok(());
                        }
                        let item = &payload["item"];
                        let (role, block_type) = match text(item, "type") {
                            Some("UserMessage") => ("user", "text"),
                            Some("AgentMessage") => ("assistant", "Text"),
                            _ => return Ok(()),
                        };
                        let body = content_text(&item["content"], block_type);
                        if body.trim().is_empty() {
                            return Ok(());
                        }
                        let entry_id = text(item, "id").context("Codex message ID is missing")?;
                        ensure!(
                            seen.insert(entry_id.to_owned()),
                            "The history repeats a completed message ID"
                        );
                        messages.push(ExternalMessage {
                            id: entry_id.to_owned(),
                            role: role.into(),
                            text: body,
                            timestamp: payload["completed_at_ms"]
                                .as_i64()
                                .context("Codex message completion time is missing")?,
                            model: model.clone(),
                            provider: provider.clone(),
                            native_stop_reason: None,
                        });
                    }
                    _ => {}
                }
                Ok(())
            })?;
            ensure!(has_meta, "Codex session metadata is missing");
            ensure!(
                !messages.is_empty(),
                "No supported Codex messages found; completed item events are required"
            );
            incomplete
        }
        Source::ClaudeCode => {
            // Keep the parent graph and projected text, never tool output or reasoning.
            let mut nodes = HashMap::new();
            let mut leaf = None;
            let mut custom_title = None;
            let incomplete = records(reader, |row| {
                if row["isSidechain"] == true {
                    return Ok(());
                }
                if let Some(value) = text(&row, "cwd") {
                    cwd = Some(value.to_owned());
                }
                match text(&row, "type") {
                    Some("custom-title") => {
                        custom_title = text(&row, "customTitle").map(str::to_owned)
                    }
                    Some("ai-title") => title = text(&row, "aiTitle").map(str::to_owned),
                    _ => {}
                }
                if let Some(entry_id) = text(&row, "uuid") {
                    let role = text(&row, "type").filter(|r| matches!(*r, "user" | "assistant"));
                    if role.is_some() {
                        leaf = Some(entry_id.to_owned());
                    }
                    let projected = (|| -> Result<Option<ExternalMessage>> {
                        let Some(role) = role else { return Ok(None) };
                        if row["isMeta"] == true || row["isCompactSummary"] == true {
                            return Ok(None);
                        }
                        let message = &row["message"];
                        let body = content_text(&message["content"], "text");
                        if body.trim().is_empty() {
                            return Ok(None);
                        }
                        Ok(Some(ExternalMessage {
                            id: entry_id.to_owned(),
                            role: role.into(),
                            text: body,
                            timestamp: timestamp(&row["timestamp"])?,
                            model: text(message, "model").map(str::to_owned),
                            provider: Some("anthropic".into()),
                            native_stop_reason: text(message, "stop_reason").map(str::to_owned),
                        }))
                    })();
                    nodes.insert(
                        entry_id.to_owned(),
                        (text(&row, "parentUuid").map(str::to_owned), projected),
                    );
                }
                Ok(())
            })?;
            title = custom_title.or(title);
            let mut visited = HashSet::new();
            while let Some(entry_id) = leaf {
                ensure!(
                    visited.insert(entry_id.clone()),
                    "The history contains a parent cycle"
                );
                let (parent, message) = nodes
                    .remove(&entry_id)
                    .context("The history has a missing parent message")?;
                if let Some(message) = message? {
                    messages.push(message);
                }
                leaf = parent;
            }
            messages.reverse();
            ensure!(
                !messages.is_empty(),
                "No supported Claude Code text messages found"
            );
            incomplete
        }
    };
    let mut warnings = vec![
        "Only user and assistant text is imported; tools, reasoning, and attachments are omitted."
            .into(),
    ];
    if incomplete {
        warnings.push("The last record is still being written and was omitted. Wait for the source to finish before importing its complete history.".into());
    }
    let first = messages.first().context("The history has no messages")?;
    let last = messages.last().context("The history has no messages")?;
    let title = title.unwrap_or_else(|| {
        messages
            .iter()
            .find(|m| m.role == "user")
            .unwrap_or(first)
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(100)
            .collect()
    });
    Ok(Transcript {
        conversation: ConversationSummary {
            id: id.into(),
            source,
            title,
            cwd,
            created_at: first.timestamp,
            updated_at: modified_at.max(last.timestamp),
        },
        messages,
        warnings,
    })
}

fn collect_files(path: &Path, depth: usize, files: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() || path.symlink_metadata()?.file_type().is_symlink() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() && depth > 0 {
            collect_files(&entry.path(), depth - 1, files)?;
        } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "jsonl") {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn files(source: Source, root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    match source {
        Source::Codex => {
            collect_files(&root.join("sessions"), 3, &mut paths)?;
            collect_files(&root.join("archived_sessions"), 0, &mut paths)?;
        }
        Source::ClaudeCode => collect_files(&root.join("projects"), 1, &mut paths)?,
    }
    paths.sort_by_cached_key(|path| {
        std::cmp::Reverse(fs::metadata(path).and_then(|m| m.modified()).ok())
    });
    Ok(paths)
}

fn file_id(source: Source, path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let id = match source {
        Source::Codex => stem.get(stem.len().checked_sub(36)?..)?,
        Source::ClaudeCode => stem,
    };
    uuid::Uuid::parse_str(id).ok().map(|_| id)
}

fn read_file(source: Source, root: &Path, path: &Path) -> Result<Transcript> {
    ensure!(
        !path.symlink_metadata()?.file_type().is_symlink(),
        "History symlinks are not supported"
    );
    let canonical = path.canonicalize()?;
    ensure!(
        canonical.starts_with(root.canonicalize()?),
        "History is outside the source directory"
    );
    let id = file_id(source, path).context("Invalid history filename")?;
    let file = fs::File::open(canonical)?;
    let metadata = file.metadata()?;
    let modified_at = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_millis() as i64;
    // Snapshot the observed length so a live source cannot extend this read indefinitely.
    parse(
        source,
        id,
        BufReader::new(file.take(metadata.len())),
        modified_at,
    )
}

pub fn discover(source: Source, query: Option<&str>, cursor: Option<&str>) -> Result<Catalog> {
    discover_at(source, &source.root()?, query, cursor)
}

fn discover_at(
    source: Source,
    root: &Path,
    query: Option<&str>,
    cursor: Option<&str>,
) -> Result<Catalog> {
    let mut catalog = Catalog {
        source,
        directory: root.display().to_string(),
        available: root.is_dir(),
        conversations: Vec::new(),
        next_cursor: None,
        warnings: Vec::new(),
    };
    if !catalog.available {
        return Ok(catalog);
    }
    let paths = files(source, root)?;
    let offset = cursor
        .map(str::parse::<usize>)
        .transpose()
        .context("Invalid history cursor")?
        .unwrap_or(0);
    ensure!(
        offset <= paths.len(),
        "History cursor expired; refresh the list"
    );
    let query = query.unwrap_or("").trim().to_lowercase();
    let mut unavailable = 0;
    let mut seen = HashSet::new();
    for (index, path) in paths.iter().enumerate().skip(offset) {
        if file_id(source, path).is_none() {
            continue;
        }
        let transcript = match read_file(source, root, path) {
            Ok(transcript) => transcript,
            Err(_) => {
                unavailable += 1;
                continue;
            }
        };
        let c = transcript.conversation;
        if !seen.insert(c.id.clone()) {
            continue;
        }
        if !query.is_empty()
            && !c.title.to_lowercase().contains(&query)
            && !c
                .cwd
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains(&query)
        {
            continue;
        }
        catalog.conversations.push(c);
        if catalog.conversations.len() == PAGE_SIZE {
            catalog.next_cursor = (index + 1 < paths.len()).then(|| (index + 1).to_string());
            break;
        }
    }
    if unavailable > 0 {
        catalog.warnings.push(format!(
            "{unavailable} empty, unsupported, or unreadable histories were skipped."
        ));
    }
    Ok(catalog)
}

pub fn read(source: Source, id: &str) -> Result<Transcript> {
    read_at(source, &source.root()?, id)
}

fn read_at(source: Source, root: &Path, id: &str) -> Result<Transcript> {
    ensure!(
        uuid::Uuid::parse_str(id).is_ok(),
        "Invalid external conversation ID"
    );
    let matches: Vec<_> = files(source, root)?
        .into_iter()
        .filter(|path| file_id(source, path) == Some(id))
        .collect();
    match matches.as_slice() {
        [path] => read_file(source, root, path),
        [] => bail!("The source history is no longer available"),
        _ => {
            bail!("Multiple history files have this identity; resolve the duplicate at the source")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "0198ff66-9317-7000-8000-000000000001";
    fn jsonl(rows: Vec<Value>) -> Vec<u8> {
        rows.into_iter()
            .map(|r| format!("{r}\n"))
            .collect::<String>()
            .into_bytes()
    }
    fn claude(id: &str, parent: Option<&str>, role: &str, body: Value) -> Value {
        json!({"type":role,"uuid":id,"parentUuid":parent,"timestamp":"2026-09-15T12:00:00Z", "message":{"role":role,"content":body,"model":"example"}})
    }

    #[test]
    fn codex_uses_completed_items_once_and_keeps_original_timestamps() {
        let bytes = jsonl(vec![
            json!({"type":"session_meta","payload":{"id":ID,"cwd":"/project","model_provider":"openai"}}),
            json!({"type":"turn_context","payload":{"model":"example"}}),
            json!({"type":"response_item","payload":{"role":"user","content":[{"type":"text","text":"internal instructions"}]}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","completed_at_ms":1000,"item":{"type":"UserMessage","id":"u1","content":[{"type":"text","text":"hello"},{"type":"image","image_url":"private"}]}}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","completed_at_ms":2000,"item":{"type":"AgentMessage","id":"a1","content":[{"type":"Text","text":"hello back"}]}}}),
        ]);
        let result = parse(Source::Codex, ID, bytes.as_slice(), 3000).unwrap();
        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.messages[0].text, "hello");
        assert_eq!(result.messages[0].timestamp, 1000);
        assert_eq!(result.messages[1].model.as_deref(), Some("example"));
        assert_eq!(result.conversation.cwd.as_deref(), Some("/project"));
    }

    #[test]
    fn claude_follows_latest_branch_and_excludes_tool_results_and_meta() {
        let mut internal = claude("meta", Some("root"), "user", json!("system context"));
        internal["isMeta"] = json!(true);
        let mut sidechain = claude("child", None, "assistant", json!("child text"));
        sidechain["isSidechain"] = json!(true);
        let bytes = jsonl(vec![
            claude("root", None, "user", json!("repeat")),
            claude("abandoned", Some("root"), "assistant", json!("old branch")),
            internal,
            claude(
                "tool",
                Some("meta"),
                "user",
                json!([{"type":"tool_result","content":"tool output"}]),
            ),
            claude(
                "answer",
                Some("tool"),
                "assistant",
                json!([{"type":"thinking","thinking":"private"},{"type":"text","text":"answer"}]),
            ),
            claude("repeat", Some("answer"), "user", json!("repeat")),
            sidechain,
        ]);
        let result = parse(Source::ClaudeCode, ID, bytes.as_slice(), 0).unwrap();
        assert_eq!(
            result
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["root", "answer", "repeat"]
        );
        assert_eq!(result.messages[0].text, result.messages[2].text);
        assert_eq!(result.messages[0].timestamp, 1789473600000);
    }

    #[test]
    fn only_an_unfinished_final_record_is_skipped() {
        let mut bytes = jsonl(vec![claude("root", None, "user", json!("hello"))]);
        bytes.extend_from_slice(b"{\"type\":");
        let result = parse(Source::ClaudeCode, ID, bytes.as_slice(), 0).unwrap();
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.warnings.len(), 2);
        bytes.push(b'\n');
        assert!(parse(Source::ClaudeCode, ID, bytes.as_slice(), 0).is_err());
    }

    #[test]
    fn histories_above_64_mib_are_read_without_retaining_tool_output() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("ade-history-{}", uuid::Uuid::new_v4()));
        let project = root.join("projects/project");
        fs::create_dir_all(&project).unwrap();
        let path = project.join(format!("{ID}.jsonl"));
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(&jsonl(vec![claude("root", None, "user", json!("hello"))]))
            .unwrap();
        let tool = jsonl(vec![
            json!({"type":"progress","data":"x".repeat(128 * 1024)}),
        ]);
        for _ in 0..513 {
            file.write_all(&tool).unwrap();
        }
        drop(file);
        assert!(path.metadata().unwrap().len() > 64 * 1024 * 1024);
        let catalog = discover_at(Source::ClaudeCode, &root, None, None).unwrap();
        assert_eq!(catalog.conversations.len(), 1);
        let result = read_at(Source::ClaudeCode, &root, ID).unwrap();
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].text, "hello");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_pages_and_does_not_accept_arbitrary_paths() {
        let root = std::env::temp_dir().join(format!("ade-history-{}", uuid::Uuid::new_v4()));
        let project = root.join("projects/project");
        fs::create_dir_all(&project).unwrap();
        for index in 0..27 {
            let id = format!("0198ff66-9317-7000-8000-{index:012}");
            fs::write(
                project.join(format!("{id}.jsonl")),
                jsonl(vec![claude(
                    "u",
                    None,
                    "user",
                    json!(format!("conversation {index}")),
                )]),
            )
            .unwrap();
        }
        let first = discover_at(Source::ClaudeCode, &root, None, None).unwrap();
        let second = discover_at(
            Source::ClaudeCode,
            &root,
            None,
            first.next_cursor.as_deref(),
        )
        .unwrap();
        assert_eq!(first.conversations.len(), 25);
        assert_eq!(second.conversations.len(), 2);
        assert!(second.next_cursor.is_none());
        assert!(read_at(Source::ClaudeCode, &root, "../secret").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                project.join(format!("{}.jsonl", first.conversations[0].id)),
                project.join("0198ff66-9317-7000-8000-999999999999.jsonl"),
            )
            .unwrap();
            assert_eq!(files(Source::ClaudeCode, &root).unwrap().len(), 27);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
