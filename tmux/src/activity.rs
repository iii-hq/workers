//! What Claude Code and Codex record about themselves on disk, for the work
//! their screen does not show: sub-agents that keep running after the main
//! agent's turn ended leave no `esc to interrupt` behind.
//!
//! Claude Code keeps `<config>/sessions/<pid>.json` with its own status
//! (`busy` while a turn or any delegated task runs) and, next to the
//! session's transcript, a `subagents/agent-<id>.meta.json` and
//! `agent-<id>.jsonl` per sub-agent. Recent versions mark the meta
//! `finished`; with older ones, and for one that was stopped, a sub-agent
//! is done once its transcript ends on the model's final answer or on an
//! interruption. Codex runs sub-agents as threads of
//! its own process, which keeps every thread's `rollout-*.jsonl` open: a
//! thread whose last lifecycle event is `task_started` is working, and its
//! `session_meta` line says whether the main agent spawned it.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::model::{AgentState, SubagentStatus};
use crate::procs::{self, ProcTable};

/// How long a Claude Code session's transcript folder is not looked for
/// again after it was missing (it appears with the first tool result or
/// sub-agent).
const TRANSCRIPT_RETRY: Duration = Duration::from_secs(5);
/// How long the rollouts a Codex process holds open are reused before the
/// process is asked again (a new sub-agent opens a new one).
const OPEN_FILES_TTL: Duration = Duration::from_secs(2);
/// How deep below the agent's pid the Codex process holding the rollouts is
/// looked for (`node → codex`).
const CODEX_DEPTH: usize = 3;
/// Read size when looking backwards through a rollout for its last
/// lifecycle event.
const CHUNK: u64 = 256 * 1024;
/// The longest first line (`session_meta`, with the base instructions) read.
const META_LINE_LIMIT: u64 = 4 * 1024 * 1024;

/// Which on-disk records an agent keeps, by matcher id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorder {
    Claude,
    Codex,
}

impl Recorder {
    pub fn of(agent_id: &str) -> Option<Self> {
        match agent_id {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Activity {
    /// The state the agent reports, or `None` to leave it to the screen.
    pub state: Option<AgentState>,
    /// Sub-agents at work, oldest first.
    pub subagents: Vec<SubagentStatus>,
}

/// Reads [`Activity`] and caches what does not change between polls.
#[derive(Debug, Default)]
pub struct ActivityReader {
    /// Each poll bumps it; caches not touched by a poll are dropped after it.
    poll: u64,
    claude_dirs: HashMap<u32, Cached<Option<PathBuf>>>,
    transcripts: HashMap<String, Cached<Lookup>>,
    metas: HashMap<PathBuf, Cached<(SystemTime, Option<ClaudeSubagentMeta>)>>,
    /// Whether a sub-agent transcript has ended, by path, while its length
    /// holds.
    ended: HashMap<PathBuf, Cached<(u64, bool)>>,
    open: HashMap<u32, Cached<(Instant, Vec<PathBuf>)>>,
    rollouts: HashMap<PathBuf, Cached<Rollout>>,
}

#[derive(Debug)]
struct Cached<T> {
    poll: u64,
    value: T,
}

#[derive(Debug, Clone)]
enum Lookup {
    Found(PathBuf),
    Missing(Instant),
}

impl ActivityReader {
    /// One poll: the activity of each `(key, recorder, agent pid)` that has
    /// any, by key. Blocking.
    pub fn read(
        &mut self,
        table: &ProcTable,
        agents: &[(String, Recorder, u32)],
    ) -> HashMap<String, Activity> {
        self.poll += 1;
        let mut out = HashMap::new();
        for (key, recorder, pid) in agents {
            let activity = match recorder {
                Recorder::Claude => self.claude(*pid),
                Recorder::Codex => self.codex(table, *pid),
            };
            if let Some(activity) = activity {
                out.insert(key.clone(), activity);
            }
        }
        let poll = self.poll;
        self.claude_dirs.retain(|_, entry| entry.poll == poll);
        self.transcripts.retain(|_, entry| entry.poll == poll);
        self.metas.retain(|_, entry| entry.poll == poll);
        self.ended.retain(|_, entry| entry.poll == poll);
        self.open.retain(|_, entry| entry.poll == poll);
        self.rollouts.retain(|_, entry| entry.poll == poll);
        out
    }

    // ── Claude Code ────────────────────────────────────────────────────

    fn claude(&mut self, pid: u32) -> Option<Activity> {
        let poll = self.poll;
        let dir = self.claude_dirs.entry(pid).or_insert_with(|| Cached {
            poll,
            value: claude_config_dir(pid),
        });
        dir.poll = poll;
        let dir = dir.value.clone()?;
        self.claude_at(&dir, pid, procs::start_ticks(pid))
    }

    /// The activity Claude Code process `pid` records under its config
    /// directory `dir`; `start_ticks` is the process's start time when known,
    /// to tell its file from one a reused pid left behind.
    fn claude_at(&mut self, dir: &Path, pid: u32, start_ticks: Option<u64>) -> Option<Activity> {
        let text =
            std::fs::read_to_string(dir.join("sessions").join(format!("{pid}.json"))).ok()?;
        let session: ClaudeSession = serde_json::from_str(&text).ok()?;
        if session.pid != pid {
            return None;
        }
        if let (Some(recorded), Some(actual)) = (
            session
                .proc_start
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok()),
            start_ticks,
        ) {
            if recorded != actual {
                return None;
            }
        }
        let state = claude_state(session.status.as_deref(), session.waiting_for.as_deref());
        let subagents = if state == Some(AgentState::Working) {
            match self.transcript_dir(dir, &session) {
                Some(transcript) => {
                    self.claude_subagents(&transcript, session.started_at.unwrap_or(0))
                }
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        Some(Activity { state, subagents })
    }

    /// [`find_transcript_dir`], remembering a missing folder for a while.
    fn transcript_dir(&mut self, config: &Path, session: &ClaudeSession) -> Option<PathBuf> {
        let poll = self.poll;
        if let Some(entry) = self.transcripts.get_mut(&session.session_id) {
            entry.poll = poll;
            match &entry.value {
                Lookup::Found(path) => return Some(path.clone()),
                Lookup::Missing(at) if at.elapsed() < TRANSCRIPT_RETRY => return None,
                Lookup::Missing(_) => {}
            }
        }
        let found = find_transcript_dir(config, session);
        let value = match &found {
            Some(path) => Lookup::Found(path.clone()),
            None => Lookup::Missing(Instant::now()),
        };
        self.transcripts
            .insert(session.session_id.clone(), Cached { poll, value });
        found
    }

    /// Sub-agents of the session still at work: their meta does not say
    /// `finished` and their transcript has not ended. A meta
    /// older than the process (`started_ms`) belongs to an earlier run of a
    /// resumed session; it is not at work.
    fn claude_subagents(&mut self, transcript: &Path, started_ms: u64) -> Vec<SubagentStatus> {
        let poll = self.poll;
        let Ok(entries) = std::fs::read_dir(transcript.join("subagents")) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("agent-")?.strip_suffix(".meta.json"))
                .map(str::to_string)
            else {
                continue;
            };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            if epoch_ms(modified) < started_ms {
                continue;
            }
            let cached = match self.metas.get_mut(&path) {
                Some(entry) if entry.value.0 == modified => {
                    entry.poll = poll;
                    entry.value.1.clone()
                }
                _ => {
                    let meta = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|text| serde_json::from_str::<ClaudeSubagentMeta>(&text).ok());
                    self.metas.insert(
                        path.clone(),
                        Cached {
                            poll,
                            value: (modified, meta.clone()),
                        },
                    );
                    meta
                }
            };
            let Some(meta) = cached.filter(|meta| !meta.finished) else {
                continue;
            };
            if self.ended(
                &transcript
                    .join("subagents")
                    .join(format!("agent-{id}.jsonl")),
            ) {
                continue;
            }
            // The meta is written when the sub-agent starts and again when
            // it finishes, so while it runs its times are the start.
            let since = metadata.created().unwrap_or(modified);
            out.push(SubagentStatus {
                description: meta
                    .description
                    .clone()
                    .filter(|d| !d.trim().is_empty())
                    .or_else(|| meta.agent_type.clone())
                    .unwrap_or_else(|| id.clone()),
                id,
                kind: meta.agent_type,
                depth: meta.spawn_depth.unwrap_or(1).max(1),
                since_ms: epoch_ms(since),
            });
        }
        sort_subagents(&mut out);
        out
    }

    fn ended(&mut self, path: &Path) -> bool {
        let poll = self.poll;
        // No transcript yet: the sub-agent was just spawned.
        let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
            return false;
        };
        if let Some(entry) = self.ended.get_mut(path) {
            if entry.value.0 == len {
                entry.poll = poll;
                return entry.value.1;
            }
        }
        let ended = File::open(path)
            .ok()
            .and_then(|mut file| transcript_ended(&mut file, len))
            .unwrap_or(false);
        self.ended.insert(
            path.to_path_buf(),
            Cached {
                poll,
                value: (len, ended),
            },
        );
        ended
    }

    // ── Codex ──────────────────────────────────────────────────────────

    fn codex(&mut self, table: &ProcTable, pid: u32) -> Option<Activity> {
        let poll = self.poll;
        let fresh = self
            .open
            .get(&pid)
            .is_some_and(|entry| entry.value.0.elapsed() < OPEN_FILES_TTL);
        if !fresh {
            let files = procs::open_files(&codex_pids(table, pid), is_rollout);
            self.open.insert(
                pid,
                Cached {
                    poll,
                    value: (Instant::now(), files),
                },
            );
        }
        let entry = self.open.get_mut(&pid)?;
        entry.poll = poll;
        let files = entry.value.1.clone();
        self.codex_threads(&files)
    }

    /// The activity of a Codex process holding these rollouts open: working
    /// while any thread's turn is open, else left to the screen.
    fn codex_threads(&mut self, files: &[PathBuf]) -> Option<Activity> {
        if files.is_empty() {
            return None;
        }
        let mut main_working = false;
        let mut subagents = Vec::new();
        for path in files {
            let Some(rollout) = self.rollout(path) else {
                continue;
            };
            let (Some(thread), Some(since_ms)) = (&rollout.thread, rollout.open_since_ms) else {
                continue;
            };
            match &thread.spawn {
                None => main_working = true,
                Some(spawn) => subagents.push(SubagentStatus {
                    id: thread.id.clone(),
                    description: spawn.description.clone(),
                    kind: spawn.role.clone(),
                    depth: spawn.depth,
                    since_ms,
                }),
            }
        }
        sort_subagents(&mut subagents);
        let working = main_working || !subagents.is_empty();
        Some(Activity {
            state: working.then_some(AgentState::Working),
            subagents,
        })
    }

    fn rollout(&mut self, path: &Path) -> Option<Rollout> {
        let poll = self.poll;
        let metadata = std::fs::metadata(path).ok()?;
        let len = metadata.len();
        let previous = self
            .rollouts
            .get(path)
            .map(|entry| entry.value.clone())
            // A shorter file is another file under the same name.
            .filter(|previous| previous.len <= len);
        let rollout = match previous {
            Some(previous) if previous.len == len => previous,
            previous => {
                let mut file = File::open(path).ok()?;
                // A first line still being written is read again as the file grows.
                let thread = match previous.as_ref().and_then(|p| p.thread.clone()) {
                    Some(thread) => Some(thread),
                    None => read_thread(&mut file),
                };
                // Only the bytes added since the last poll can hold a newer
                // event (plus a marker's length, in case one was cut).
                let floor = previous
                    .as_ref()
                    .map(|previous| previous.len.saturating_sub(MARKER_OVERLAP))
                    .unwrap_or(0);
                let open_since_ms = match last_lifecycle(&mut file, floor, len) {
                    Some(Lifecycle::Started(since)) => Some(
                        since
                            .unwrap_or_else(|| epoch_ms(metadata.modified().unwrap_or(UNIX_EPOCH))),
                    ),
                    Some(Lifecycle::Ended) => None,
                    None => previous.and_then(|previous| previous.open_since_ms),
                };
                Rollout {
                    len,
                    thread,
                    open_since_ms,
                }
            }
        };
        self.rollouts.insert(
            path.to_path_buf(),
            Cached {
                poll,
                value: rollout.clone(),
            },
        );
        Some(rollout)
    }
}

fn sort_subagents(subagents: &mut [SubagentStatus]) {
    subagents.sort_by(|a, b| (a.since_ms, &a.id).cmp(&(b.since_ms, &b.id)));
}

fn epoch_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

// ── Claude Code records ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeSession {
    pid: u32,
    session_id: String,
    cwd: Option<String>,
    started_at: Option<u64>,
    proc_start: Option<String>,
    status: Option<String>,
    waiting_for: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeSubagentMeta {
    agent_type: Option<String>,
    description: Option<String>,
    spawn_depth: Option<u32>,
    #[serde(default)]
    finished: bool,
}

/// `<config>/projects/<project>/<session id>`: the project folder is the
/// cwd with every other character than a letter or digit as `-`, except
/// for long paths, which get a shortened name; those are looked for.
fn find_transcript_dir(config: &Path, session: &ClaudeSession) -> Option<PathBuf> {
    let projects = config.join("projects");
    let direct = session.cwd.as_deref().map(|cwd| {
        let folder: String = cwd
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        projects.join(folder).join(&session.session_id)
    });
    direct.filter(|path| path.is_dir()).or_else(|| {
        std::fs::read_dir(&projects)
            .ok()?
            .flatten()
            .map(|entry| entry.path().join(&session.session_id))
            .find(|path| path.is_dir())
    })
}

/// The transcript of sub-agent `id` of Claude Code process `pid`, finished
/// or not, if it has one yet. `id` must be a bare file-name part.
pub fn claude_subagent_file(pid: u32, id: &str) -> Option<PathBuf> {
    claude_subagent_file_at(&claude_config_dir(pid)?, pid, id)
}

fn claude_subagent_file_at(config: &Path, pid: u32, id: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config.join("sessions").join(format!("{pid}.json"))).ok()?;
    let session: ClaudeSession = serde_json::from_str(&text).ok()?;
    if session.pid != pid {
        return None;
    }
    let path = find_transcript_dir(config, &session)?
        .join("subagents")
        .join(format!("agent-{id}.jsonl"));
    path.is_file().then_some(path)
}

/// The rollout of thread `id` held open by the Codex process under agent
/// `pid`: its name ends with the thread id. `id` must be a bare file-name
/// part.
pub fn codex_thread_file(table: &ProcTable, pid: u32, id: &str) -> Option<PathBuf> {
    let suffix = format!("-{id}.jsonl");
    procs::open_files(&codex_pids(table, pid), is_rollout)
        .into_iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(&suffix))
        })
}

/// The wrapper (`node … codex.js`) and the native `codex` under agent `pid`;
/// MCP servers and shells in the subtree hold no rollouts.
fn codex_pids(table: &ProcTable, pid: u32) -> Vec<u32> {
    table
        .subtree(pid, CODEX_DEPTH)
        .into_iter()
        .map(|(pid, _)| pid)
        .filter(|pid| {
            table
                .get(*pid)
                .is_some_and(|proc| proc.names().any(|name| name == "codex"))
        })
        .collect()
}

/// `CLAUDE_CONFIG_DIR` of the process, else `~/.claude` of its home.
fn claude_config_dir(pid: u32) -> Option<PathBuf> {
    if let Some(dir) = procs::environ_var(pid, "CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let home = procs::environ_var(pid, "HOME")
        .or_else(|| std::env::var("HOME").ok())
        .filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join(".claude"))
}

/// Claude Code's own status: `busy` while a turn or a delegated task
/// (sub-agent, workflow, teammate) runs, `waiting` while a prompt or dialog
/// is open, `idle` (or `shell`, idle with a background command) otherwise.
fn claude_state(status: Option<&str>, waiting_for: Option<&str>) -> Option<AgentState> {
    match status? {
        "busy" => Some(AgentState::Working),
        "idle" | "shell" => Some(AgentState::Idle),
        // A dialog the user opened (`/config`, `/resume`) asks nothing of them.
        "waiting" if waiting_for != Some("dialog open") => Some(AgentState::Waiting),
        _ => None,
    }
}

/// The farthest back a sub-agent transcript is read for its last message.
const TRANSCRIPT_TAIL_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Deserialize)]
struct TranscriptEntry {
    #[serde(rename = "type")]
    kind: String,
    message: Option<TranscriptMessage>,
}

#[derive(Deserialize)]
struct TranscriptMessage {
    stop_reason: Option<String>,
    #[serde(default)]
    content: TranscriptContent,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
enum TranscriptContent {
    Text(String),
    Blocks(Vec<TranscriptBlock>),
    #[default]
    Other,
}

#[derive(Deserialize)]
struct TranscriptBlock {
    text: Option<String>,
}

/// What Claude Code writes as the user's message when a turn is stopped:
/// `[Request interrupted by user]`, or `… for tool use]` mid tool call.
/// A stopped sub-agent (Esc, or its parent stopping it) ends on it, and
/// 2.1.308 does not mark its meta `finished`.
const INTERRUPTED: &str = "[Request interrupted by user";

impl TranscriptMessage {
    fn is_interruption(&self) -> bool {
        match &self.content {
            TranscriptContent::Text(text) => text.starts_with(INTERRUPTED),
            TranscriptContent::Blocks(blocks) => blocks.iter().any(|block| {
                block
                    .text
                    .as_deref()
                    .is_some_and(|text| text.starts_with(INTERRUPTED))
            }),
            TranscriptContent::Other => false,
        }
    }
}

/// Whether the last message of a transcript (`len` bytes) ends the
/// sub-agent: the model's final answer, or an interruption. `None` when no
/// message was found. A tool call, a tool result or a prompt last means the
/// sub-agent is still at it; bookkeeping entries (`api-request`,
/// `attachment`, …) are skipped.
fn transcript_ended(file: &mut File, len: u64) -> Option<bool> {
    let mut end = len;
    // The start of a line cut by the previous (later) chunk.
    let mut carry: Vec<u8> = Vec::new();
    while end > 0 && len - end < TRANSCRIPT_TAIL_LIMIT {
        let start = end.saturating_sub(CHUNK);
        let mut buffer = vec![0; (end - start) as usize];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut buffer).ok()?;
        buffer.extend_from_slice(&carry);
        // Unless the chunk starts the file, its first line may be cut.
        let first_break = if start == 0 {
            Some(0)
        } else {
            buffer.iter().position(|b| *b == b'\n').map(|at| at + 1)
        };
        let Some(first_break) = first_break else {
            carry = buffer;
            end = start;
            continue;
        };
        for line in buffer[first_break..].rsplit(|b| *b == b'\n') {
            let Ok(entry) = serde_json::from_slice::<TranscriptEntry>(line) else {
                continue;
            };
            match entry.kind.as_str() {
                "assistant" => {
                    let stop = entry.message.and_then(|message| message.stop_reason);
                    return Some(matches!(
                        stop.as_deref(),
                        Some("end_turn" | "stop_sequence" | "refusal")
                    ));
                }
                "user" => {
                    return Some(
                        entry
                            .message
                            .is_some_and(|message| message.is_interruption()),
                    )
                }
                _ => {}
            }
        }
        carry = buffer[..first_break.saturating_sub(1)].to_vec();
        end = start;
    }
    None
}

// ── Codex records ───────────────────────────────────────────────────────

fn is_rollout(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
}

#[derive(Debug, Clone)]
struct Rollout {
    len: u64,
    thread: Option<CodexThread>,
    /// When the thread's open turn started; `None` while no turn is open.
    open_since_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
struct CodexThread {
    id: String,
    /// Set for a thread another thread spawned.
    spawn: Option<CodexSpawn>,
}

#[derive(Debug, Clone, PartialEq)]
struct CodexSpawn {
    description: String,
    role: Option<String>,
    depth: u32,
}

#[derive(Deserialize)]
struct MetaLine {
    #[serde(rename = "type")]
    kind: String,
    payload: MetaPayload,
}

#[derive(Deserialize)]
struct MetaPayload {
    id: String,
    #[serde(default)]
    source: serde_json::Value,
    agent_path: Option<String>,
    agent_nickname: Option<String>,
    agent_role: Option<String>,
}

/// The thread a rollout records, from its first line (`session_meta`).
fn read_thread(file: &mut File) -> Option<CodexThread> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut line = Vec::new();
    BufReader::new(file.take(META_LINE_LIMIT))
        .read_until(b'\n', &mut line)
        .ok()?;
    parse_thread(&line)
}

fn parse_thread(line: &[u8]) -> Option<CodexThread> {
    let meta: MetaLine = serde_json::from_slice(line).ok()?;
    if meta.kind != "session_meta" {
        return None;
    }
    let payload = meta.payload;
    // `"source": "cli"` for a thread the user started;
    // `{"subagent": {"thread_spawn": {…}}}` for one an agent spawned, and
    // `{"subagent": {"other": "guardian"}}` for Codex's own reviewers.
    let Some(subagent) = payload.source.get("subagent") else {
        return Some(CodexThread {
            id: payload.id,
            spawn: None,
        });
    };
    let spawn = subagent.get("thread_spawn");
    let field = |key: &str| {
        spawn
            .and_then(|spawn| spawn.get(key))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    };
    let path = payload.agent_path.or_else(|| field("agent_path"));
    let description = path
        .as_deref()
        .map(|path| path.strip_prefix("/root/").unwrap_or(path).to_string())
        .filter(|path| !path.is_empty())
        .or(payload.agent_nickname)
        .or_else(|| field("agent_nickname"))
        .or_else(|| {
            subagent
                .get("other")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "sub-agent".to_string());
    let depth = spawn
        .and_then(|spawn| spawn.get("depth"))
        .and_then(|depth| depth.as_u64())
        .map_or(1, |depth| depth.max(1) as u32);
    Some(CodexThread {
        id: payload.id,
        spawn: Some(CodexSpawn {
            description,
            role: payload.agent_role.or_else(|| field("agent_role")),
            depth,
        }),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Lifecycle {
    /// A turn started, at this time when the event says.
    Started(Option<u64>),
    Ended,
}

/// Lifecycle events as they appear in a rollout line (`turn_*` are older
/// names of the same events).
const MARKERS: [(&[u8], bool); 5] = [
    (br#""type":"task_started""#, true),
    (br#""type":"turn_started""#, true),
    (br#""type":"task_complete""#, false),
    (br#""type":"turn_complete""#, false),
    (br#""type":"turn_aborted""#, false),
];
const MARKER_OVERLAP: u64 = 24;

/// The last lifecycle event in `file[floor..len]`, looking backwards.
fn last_lifecycle(file: &mut File, floor: u64, len: u64) -> Option<Lifecycle> {
    let mut end = len;
    let mut buffer = Vec::new();
    while end > floor {
        let start = end.saturating_sub(CHUNK).max(floor);
        let stop = (end + MARKER_OVERLAP).min(len);
        buffer.resize((stop - start) as usize, 0);
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut buffer).ok()?;
        let last = MARKERS
            .iter()
            .filter_map(|(marker, starts)| Some((rfind(&buffer, marker)?, *starts)))
            .max_by_key(|(at, _)| *at);
        if let Some((at, starts)) = last {
            if !starts {
                return Some(Lifecycle::Ended);
            }
            // `"started_at":<seconds>` follows the type in the same line,
            // which may run past this chunk.
            let mut line = Vec::new();
            file.seek(SeekFrom::Start(start + at as u64)).ok()?;
            BufReader::new((&mut *file).take(4096))
                .read_until(b'\n', &mut line)
                .ok()?;
            return Some(Lifecycle::Started(started_at_ms(&line)));
        }
        end = start;
    }
    None
}

fn started_at_ms(line: &[u8]) -> Option<u64> {
    const KEY: &[u8] = br#""started_at":"#;
    let at = find(line, KEY)? + KEY.len();
    let digits: Vec<u8> = line[at..]
        .iter()
        .copied()
        .take_while(u8::is_ascii_digit)
        .collect();
    let seconds: u64 = std::str::from_utf8(&digits).ok()?.parse().ok()?;
    Some(seconds * 1000)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A scratch directory removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "tmux-activity-{name}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, rel: &str, content: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn claude_session(pid: u32, status: &str, waiting_for: Option<&str>) -> String {
        serde_json::json!({
            "pid": pid,
            "sessionId": "5d2378cc-1286-4924-9d15-86ef21717aa0",
            "cwd": "/home/dev/code/deep.key",
            "startedAt": 1_000,
            "procStart": "199241211",
            "version": "2.1.308",
            "kind": "interactive",
            "tmux": "work:@483.%609",
            "status": status,
            "waitingFor": waiting_for,
            "statusUpdatedAt": 2_000,
        })
        .to_string()
    }

    const TRANSCRIPT: &str =
        "projects/-home-dev-code-deep-key/5d2378cc-1286-4924-9d15-86ef21717aa0";

    #[test]
    fn claude_status_maps_to_states() {
        assert_eq!(claude_state(Some("busy"), None), Some(AgentState::Working));
        assert_eq!(claude_state(Some("idle"), None), Some(AgentState::Idle));
        assert_eq!(claude_state(Some("shell"), None), Some(AgentState::Idle));
        assert_eq!(
            claude_state(Some("waiting"), Some("permission prompt")),
            Some(AgentState::Waiting)
        );
        assert_eq!(claude_state(Some("waiting"), Some("dialog open")), None);
        assert_eq!(claude_state(Some("compacting"), None), None);
        assert_eq!(claude_state(None, None), None);
    }

    #[test]
    fn claude_busy_lists_its_unfinished_subagents() {
        let scratch = Scratch::new("claude");
        scratch.write("sessions/4242.json", &claude_session(4242, "busy", None));
        scratch.write(
            &format!("{TRANSCRIPT}/subagents/agent-a09cd1aaf64a55b3b.meta.json"),
            r#"{"agentType":"general-purpose","description":"Adversarial review of signing keys","toolUseId":"toolu_1","spawnDepth":1,"requestShape":"background","requestNonInteractive":true}"#,
        );
        scratch.write(
            &format!("{TRANSCRIPT}/subagents/agent-a1f679f6882873fad.meta.json"),
            r#"{"agentType":"Explore","description":"Map the parser","spawnDepth":2,"finished":true}"#,
        );
        scratch.write(
            &format!("{TRANSCRIPT}/subagents/agent-a09cd1aaf64a55b3b.jsonl"),
            "{}\n",
        );
        let mut reader = ActivityReader::default();
        let activity = reader
            .claude_at(&scratch.0, 4242, Some(199_241_211))
            .unwrap();
        assert_eq!(activity.state, Some(AgentState::Working));
        assert_eq!(activity.subagents.len(), 1);
        let subagent = &activity.subagents[0];
        assert_eq!(subagent.id, "a09cd1aaf64a55b3b");
        assert_eq!(subagent.description, "Adversarial review of signing keys");
        assert_eq!(subagent.kind.as_deref(), Some("general-purpose"));
        assert_eq!(subagent.depth, 1);
        assert!(subagent.since_ms > 1_000);
    }

    fn entry(kind: &str, content: &str, stop: Option<&str>) -> String {
        serde_json::json!({
            "parentUuid": "p",
            "isSidechain": true,
            "agentId": "a1",
            "type": kind,
            "message": {"role": kind, "content": [{"type": content}], "stop_reason": stop},
            "timestamp": "2026-10-22T18:47:12.263Z",
        })
        .to_string()
    }

    #[test]
    fn claude_without_finished_marks_reads_the_transcript() {
        // Claude Code 2.1.300 never marks a meta `finished`.
        let scratch = Scratch::new("claude-old");
        scratch.write("sessions/4242.json", &claude_session(4242, "busy", None));
        let meta = r#"{"agentType":"general-purpose","description":"D","spawnDepth":1,"requestShape":"background"}"#;
        let bookkeeping = r#"{"type":"api-request","timestamp":"2026-10-22T18:46:55.587Z"}"#;
        let answer = entry("assistant", "text", Some("end_turn"));
        let thinking = entry("assistant", "thinking", None);
        let tool_call = entry("assistant", "tool_use", Some("tool_use"));
        let tool_result = entry("user", "tool_result", None);
        // As Claude Code 2.1.308 writes them when a sub-agent is stopped.
        let stopped = serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": "[Request interrupted by user]"},
        })
        .to_string();
        let stopped_mid_call = serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "text", "text": "[Request interrupted by user for tool use]"}
            ]},
        })
        .to_string();
        let prompt = serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "text", "text": "Also check the tests"}]},
        })
        .to_string();
        for (id, lines) in [
            (
                "done",
                vec![
                    tool_result.as_str(),
                    bookkeeping,
                    thinking.as_str(),
                    answer.as_str(),
                ],
            ),
            (
                "thinking",
                vec![
                    answer.as_str(),
                    tool_result.as_str(),
                    bookkeeping,
                    thinking.as_str(),
                ],
            ),
            ("calling", vec![tool_result.as_str(), tool_call.as_str()]),
            (
                "waiting",
                vec![tool_call.as_str(), tool_result.as_str(), bookkeeping],
            ),
            (
                "stopped",
                vec![thinking.as_str(), bookkeeping, stopped.as_str()],
            ),
            (
                "stopped-mid-call",
                vec![
                    tool_call.as_str(),
                    tool_result.as_str(),
                    stopped_mid_call.as_str(),
                ],
            ),
            ("prompted", vec![answer.as_str(), prompt.as_str()]),
        ] {
            scratch.write(
                &format!("{TRANSCRIPT}/subagents/agent-{id}.meta.json"),
                meta,
            );
            scratch.write(
                &format!("{TRANSCRIPT}/subagents/agent-{id}.jsonl"),
                &(lines.join("\n") + "\n"),
            );
        }
        // Spawned, transcript not written yet.
        scratch.write(&format!("{TRANSCRIPT}/subagents/agent-new.meta.json"), meta);
        let mut reader = ActivityReader::default();
        let activity = reader.claude_at(&scratch.0, 4242, None).unwrap();
        let mut ids: Vec<&str> = activity.subagents.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["calling", "new", "prompted", "thinking", "waiting"]
        );
    }

    #[test]
    fn transcript_answers_are_found_across_chunks() {
        let scratch = Scratch::new("transcript-big");
        let answer = entry("assistant", "text", Some("end_turn"));
        // One line longer than a chunk after the answer, then a short one.
        let blob = format!(
            r#"{{"type":"api-request-blob","body":"{}"}}"#,
            "y".repeat(CHUNK as usize + 10)
        );
        let path = scratch.write(
            "agent-big.jsonl",
            &format!(
                "{}\n{answer}\n{blob}\n{{\"type\":\"attachment\"}}\n",
                entry("user", "text", None)
            ),
        );
        let len = std::fs::metadata(&path).unwrap().len();
        let mut file = File::open(&path).unwrap();
        assert_eq!(transcript_ended(&mut file, len), Some(true));
        let empty = scratch.write("agent-empty.jsonl", "");
        assert_eq!(transcript_ended(&mut File::open(&empty).unwrap(), 0), None);
    }

    #[test]
    fn claude_finds_shortened_transcript_folders() {
        let scratch = Scratch::new("claude-long");
        scratch.write("sessions/4242.json", &claude_session(4242, "busy", None));
        scratch.write(
            "projects/-home-dev-code-deep-k-1a2b3c/5d2378cc-1286-4924-9d15-86ef21717aa0/subagents/agent-a1.meta.json",
            r#"{"agentType":"fork","description":"Checking the parser","spawnDepth":1}"#,
        );
        let mut reader = ActivityReader::default();
        let activity = reader.claude_at(&scratch.0, 4242, None).unwrap();
        assert_eq!(activity.subagents[0].description, "Checking the parser");
        assert_eq!(activity.subagents[0].kind.as_deref(), Some("fork"));
    }

    #[test]
    fn claude_idle_or_stale_files_list_nothing() {
        let scratch = Scratch::new("claude-idle");
        scratch.write(
            &format!("{TRANSCRIPT}/subagents/agent-a1.meta.json"),
            r#"{"agentType":"general-purpose","description":"Killed with its session"}"#,
        );
        let mut reader = ActivityReader::default();

        scratch.write("sessions/4242.json", &claude_session(4242, "idle", None));
        let idle = reader.claude_at(&scratch.0, 4242, None).unwrap();
        assert_eq!(idle.state, Some(AgentState::Idle));
        assert!(idle.subagents.is_empty());

        // The file a previous process with this pid left behind.
        scratch.write("sessions/4242.json", &claude_session(4242, "busy", None));
        assert_eq!(reader.claude_at(&scratch.0, 4242, Some(7)), None);
        // A file that is not this pid's.
        assert_eq!(reader.claude_at(&scratch.0, 4343, None), None);
    }

    #[test]
    fn claude_skips_subagents_of_an_earlier_run() {
        let scratch = Scratch::new("claude-resumed");
        let session = serde_json::json!({
            "pid": 4242,
            "sessionId": "5d2378cc-1286-4924-9d15-86ef21717aa0",
            "cwd": "/home/dev/code/deep.key",
            // Started after the meta below was written.
            "startedAt": now_ms() + 60_000,
            "status": "busy",
        });
        scratch.write("sessions/4242.json", &session.to_string());
        scratch.write(
            &format!("{TRANSCRIPT}/subagents/agent-a1.meta.json"),
            r#"{"agentType":"general-purpose","description":"Crashed with the last run"}"#,
        );
        let mut reader = ActivityReader::default();
        let activity = reader.claude_at(&scratch.0, 4242, None).unwrap();
        assert_eq!(activity.state, Some(AgentState::Working));
        assert!(activity.subagents.is_empty());
    }

    fn now_ms() -> u64 {
        epoch_ms(SystemTime::now())
    }

    const ROOT_META: &str = r#"{"timestamp":"2026-10-22T19:48:10.128Z","type":"session_meta","payload":{"session_id":"01a16528-1d0d","id":"01a16528-1d0d","cwd":"/work","originator":"codex-tui","cli_version":"0.160.0","source":"cli","thread_source":"user","base_instructions":{"text":"You are Codex."}}}"#;
    const CHILD_META: &str = r#"{"timestamp":"2026-10-22T19:48:57.000Z","type":"session_meta","payload":{"id":"01a16528-d646","cwd":"/work","source":{"subagent":{"thread_spawn":{"parent_thread_id":"01a16528-1d0d","depth":1,"agent_path":"/root/sleep_test","agent_nickname":"Nash","agent_role":null}}},"thread_source":"subagent","agent_path":"/root/sleep_test","agent_nickname":"Nash"}}"#;

    fn started(turn: &str, at: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-10-22T19:48:53.000Z","type":"event_msg","payload":{{"type":"task_started","turn_id":"{turn}","root_turn_id":"{turn}","started_at":{at},"model_context_window":475000}}}}"#
        )
    }

    fn completed(turn: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-10-22T19:49:01.000Z","type":"event_msg","payload":{{"type":"task_complete","turn_id":"{turn}","last_agent_message":"The sub-agent was started."}}}}"#
        )
    }

    /// Model output that quotes an event: escaped inside a string, never a marker.
    const QUOTED: &str = r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"{\"type\":\"task_started\"}"}]}}"#;

    fn rollout(scratch: &Scratch, name: &str, lines: &[&str]) -> PathBuf {
        scratch.write(name, &(lines.join("\n") + "\n"))
    }

    #[test]
    fn codex_subagent_with_an_open_turn_is_working() {
        let scratch = Scratch::new("codex");
        let root = rollout(
            &scratch,
            "rollout-2026-10-22T16-48-10-01a16528-1d0d.jsonl",
            &[
                ROOT_META,
                &started("t1", 1_792_698_533),
                QUOTED,
                &completed("t1"),
            ],
        );
        let child = rollout(
            &scratch,
            "rollout-2026-10-22T16-48-57-01a16528-d646.jsonl",
            &[CHILD_META, &started("t2", 1_792_698_537), QUOTED],
        );
        let mut reader = ActivityReader::default();
        let activity = reader
            .codex_threads(&[root.clone(), child.clone()])
            .unwrap();
        assert_eq!(activity.state, Some(AgentState::Working));
        assert_eq!(
            activity.subagents,
            vec![SubagentStatus {
                id: "01a16528-d646".into(),
                description: "sleep_test".into(),
                kind: None,
                depth: 1,
                since_ms: 1_792_698_537_000,
            }]
        );

        // The sub-agent finishes: nothing is open, the screen decides.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&child)
            .unwrap();
        writeln!(file, "{}", completed("t2")).unwrap();
        let activity = reader.codex_threads(&[root, child]).unwrap();
        assert_eq!(activity.state, None);
        assert!(activity.subagents.is_empty());
    }

    #[test]
    fn codex_rereads_a_first_line_caught_half_written() {
        let scratch = Scratch::new("codex-partial");
        let half = &CHILD_META[..CHILD_META.len() / 2];
        let path = scratch.write("rollout-partial.jsonl", half);
        let mut reader = ActivityReader::default();
        assert_eq!(
            reader
                .codex_threads(std::slice::from_ref(&path))
                .unwrap()
                .state,
            None
        );
        scratch.write(
            "rollout-partial.jsonl",
            &format!("{CHILD_META}\n{}\n", started("t1", 7)),
        );
        let activity = reader.codex_threads(&[path]).unwrap();
        assert_eq!(activity.subagents[0].description, "sleep_test");
    }

    #[test]
    fn codex_main_turn_counts_without_subagents() {
        let scratch = Scratch::new("codex-main");
        let root = rollout(
            &scratch,
            "rollout-root.jsonl",
            &[ROOT_META, &started("t1", 5)],
        );
        let mut reader = ActivityReader::default();
        let activity = reader.codex_threads(&[root]).unwrap();
        assert_eq!(activity.state, Some(AgentState::Working));
        assert!(activity.subagents.is_empty());
        assert_eq!(reader.codex_threads(&[]), None);
    }

    #[test]
    fn codex_finds_an_event_far_behind_the_end() {
        let scratch = Scratch::new("codex-big");
        let filler = format!(
            r#"{{"type":"response_item","payload":{{"type":"function_call_output","output":"{}"}}}}"#,
            "x".repeat(4096)
        );
        let mut lines = vec![CHILD_META.to_string(), completed("t0"), started("t1", 9)];
        lines.extend(std::iter::repeat_n(filler, 200));
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let path = rollout(&scratch, "rollout-big.jsonl", &refs);
        assert!(std::fs::metadata(&path).unwrap().len() > 3 * CHUNK);
        let mut reader = ActivityReader::default();
        let activity = reader.codex_threads(&[path]).unwrap();
        assert_eq!(activity.subagents[0].since_ms, 9_000);
    }

    #[test]
    fn codex_thread_sources() {
        let root = parse_thread(ROOT_META.as_bytes()).unwrap();
        assert_eq!(root.id, "01a16528-1d0d");
        assert_eq!(root.spawn, None);
        let child = parse_thread(CHILD_META.as_bytes()).unwrap();
        assert_eq!(
            child.spawn,
            Some(CodexSpawn {
                description: "sleep_test".into(),
                role: None,
                depth: 1
            })
        );
        let nested = r#"{"type":"session_meta","payload":{"id":"c2","source":{"subagent":{"thread_spawn":{"depth":2,"agent_path":"/root/review/tests","agent_role":"explorer"}}}}}"#;
        let nested = parse_thread(nested.as_bytes()).unwrap().spawn.unwrap();
        assert_eq!(nested.description, "review/tests");
        assert_eq!(nested.role.as_deref(), Some("explorer"));
        assert_eq!(nested.depth, 2);
        let guardian = r#"{"type":"session_meta","payload":{"id":"g1","source":{"subagent":{"other":"guardian"}}}}"#;
        assert_eq!(
            parse_thread(guardian.as_bytes())
                .unwrap()
                .spawn
                .unwrap()
                .description,
            "guardian"
        );
        assert_eq!(parse_thread(started("t", 1).as_bytes()), None);
    }

    #[test]
    fn rollout_names() {
        assert!(is_rollout(Path::new(
            "/home/dev/.codex/sessions/2026/10/22/rollout-2026-10-22T16-48-10-01a1.jsonl"
        )));
        assert!(!is_rollout(Path::new("/home/dev/.codex/logs_2.sqlite")));
        assert!(!is_rollout(Path::new("/home/dev/.codex/history.jsonl")));
    }
}
