//! `tmux::usage::claude_outside` (chat-projects spec §4.2.1, D28, D134):
//! the Claude Code spend on an account outside the Chat (tmux windows and
//! any other Claude Code on this computer), from Claude Code's own
//! transcripts.
//!
//! Claude Code writes the usage of every reply into its transcripts
//! (`<config>/projects/<project>/<session>.jsonl`, and a sub-agent's under
//! `<session>/subagents/`). The default `conversations` group links every
//! account's `projects/` to `~/.claude/projects`, so the folder does not
//! tell the account; a session id does:
//!
//! - **Sessions tmux ran.** Each Claude Code session seen in a pane is
//!   noted with where it ran (an account's folder, or the slot) and when
//!   that changed, in `<accounts root>/claude-sessions.json`. Its entries
//!   go to that folder's account at the entry's time, or, on the slot, to
//!   the account global then.
//! - **Other sessions** go to the account global at the entry's time
//!   (`global_history`), counted in `attributed`; one found only in an
//!   account's own `projects/` (not shared with `~/.claude`) goes to that
//!   account, since only Claude Code on that folder writes there.
//!
//! Entries are counted once by message id and request id, across folders
//! (a moved conversation's transcript is copied). A scan reads only files
//! changed since the week asked about; it is kept five minutes and read
//! only when asked. The Chat's provider writes no transcript, so nothing is
//! counted twice.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::paths::ClaudePaths;
use super::registry;
use super::slot::Place;
use crate::model::{GlobalChange, ModelSpend, OutsideSpend, now_ms};

/// The sessions file's name in the accounts root.
pub const SESSIONS_FILE: &str = "claude-sessions.json";
/// How long a scan answers before the transcripts are read again.
pub const CACHE_MS: u64 = 5 * 60_000;
/// A scan reaches at least this far back, so the accounts' weeks (each
/// with its own reset) are answered by one scan.
const SCAN_BACK_MS: u64 = 8 * 24 * 3_600_000;
/// A session not seen for this long leaves the sessions file.
const KEEP_MS: u64 = 35 * 24 * 3_600_000;
/// How often a session still seen where it was is written down again.
const SEEN_EVERY_MS: u64 = 10 * 60_000;
/// How deep below `projects/` transcripts are looked for
/// (`<project>/<session>/subagents/<agent>.jsonl`).
const DEPTH: usize = 4;
/// The longest transcript line read; a longer one is skipped.
const LINE_LIMIT: usize = 64 * 1024 * 1024;

/// Where a session ran from `at_ms` on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPlace {
    /// The account whose folder it ran on; absent on the slot, where it
    /// ran as whichever account was global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    pub at_ms: u64,
}

/// One session tmux saw.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Where it ran, oldest first: one entry per change.
    pub places: Vec<SessionPlace>,
    /// When a pane last showed it.
    pub seen_ms: u64,
}

impl SessionRecord {
    /// Where it ran at `at_ms`: the latest change at or before it, else
    /// the first (it ran there before tmux saw it).
    fn folder_at(&self, at_ms: u64) -> Option<&Option<String>> {
        self.places
            .iter()
            .rev()
            .find(|place| place.at_ms <= at_ms)
            .or_else(|| self.places.first())
            .map(|place| &place.folder)
    }
}

/// `claude-sessions.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SessionFile {
    #[serde(default)]
    sessions: HashMap<String, SessionRecord>,
}

#[derive(Debug, Default)]
struct SessionLog {
    loaded: bool,
    file: SessionFile,
    written_at_ms: u64,
}

/// The sessions tmux saw and the last scan.
pub struct Outside {
    file: Option<PathBuf>,
    log: Mutex<SessionLog>,
    scan: tokio::sync::Mutex<Option<Scan>>,
}

impl Outside {
    pub fn new(paths: Option<&ClaudePaths>) -> Self {
        Self {
            file: paths.map(|paths| paths.root.join(SESSIONS_FILE)),
            log: Mutex::new(SessionLog::default()),
            scan: tokio::sync::Mutex::new(None),
        }
    }

    fn log(&self) -> std::sync::MutexGuard<'_, SessionLog> {
        let mut log = self.log.lock().unwrap_or_else(|p| p.into_inner());
        if !log.loaded {
            log.loaded = true;
            if let Some(text) = self
                .file
                .as_ref()
                .and_then(|f| std::fs::read_to_string(f).ok())
            {
                match serde_json::from_str(&text) {
                    Ok(file) => log.file = file,
                    Err(error) => {
                        tracing::warn!(%error, "claude-sessions.json unreadable; started again")
                    }
                }
            }
        }
        log
    }

    /// Note the sessions panes show now, each with where it runs (`None`:
    /// on no account's folder, not noted), at `now`. Written when a place
    /// changed or a session is new, else at most every ten minutes.
    pub fn note<'a>(&self, seen: impl IntoIterator<Item = (&'a str, Option<&'a Place>)>, now: u64) {
        let mut log = self.log();
        let mut changed = false;
        let mut seen_any = false;
        for (session, place) in seen {
            let Some(place) = place else {
                continue;
            };
            if session.is_empty() {
                continue;
            }
            let folder = match place {
                Place::Slot => None,
                Place::Folder(id) => Some(id.clone()),
            };
            let record = log.file.sessions.entry(session.to_string()).or_default();
            if record.places.last().map(|last| &last.folder) != Some(&folder) {
                record.places.push(SessionPlace { folder, at_ms: now });
                changed = true;
            }
            record.seen_ms = now;
            seen_any = true;
        }
        let due = seen_any && now.saturating_sub(log.written_at_ms) >= SEEN_EVERY_MS;
        if changed || due {
            log.file
                .sessions
                .retain(|_, record| now.saturating_sub(record.seen_ms) < KEEP_MS);
            if let Some(file) = &self.file {
                match registry::write_atomic(file, &log.file) {
                    Ok(()) => log.written_at_ms = now,
                    Err(error) => tracing::debug!(%error, "claude-sessions.json not saved"),
                }
            }
        }
    }

    /// The sessions noted so far.
    pub fn sessions(&self) -> HashMap<String, SessionRecord> {
        self.log().file.sessions.clone()
    }

    /// Forget the last scan (tests).
    pub async fn forget_scan(&self) {
        *self.scan.lock().await = None;
    }
}

/// Where transcripts are read: a `projects/` folder, and the account it
/// belongs to when it is an account's own (not `~/.claude`'s).
#[derive(Debug, Clone)]
pub struct Root {
    pub dir: PathBuf,
    pub owner: Option<String>,
}

/// What a scan needs to know, taken before it runs.
#[derive(Debug, Clone)]
pub struct ScanInput {
    pub roots: Vec<Root>,
    pub sessions: HashMap<String, SessionRecord>,
    pub history: Vec<GlobalChange>,
    pub global_now: String,
    pub since_ms: u64,
    pub now_ms: u64,
}

/// One reply counted.
#[derive(Debug, Clone)]
struct Counted {
    account: u32,
    model: u32,
    at_ms: u64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    /// Dated against `global_history` alone.
    fallback: bool,
}

/// The replies since `since_ms`, each with its account.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    since_ms: u64,
    as_of_ms: u64,
    accounts: Vec<String>,
    models: Vec<String>,
    entries: Vec<Counted>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
        .map_or(u64::MAX, |at| at.as_millis() as u64)
}

/// The `.jsonl` files below `dir` changed since `since_ms`, never through
/// a symbolic link.
fn transcripts(dir: &Path, since_ms: u64, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() && depth > 1 {
            transcripts(&path, since_ms, depth - 1, out);
        } else if kind.is_file()
            && path.extension().is_some_and(|ext| ext == "jsonl")
            && entry
                .metadata()
                .is_ok_and(|meta| modified_ms(&meta) >= since_ms)
        {
            out.push(path);
        }
    }
}

/// The session a transcript belongs to when its lines don't say:
/// `<session>.jsonl`, or `<session>/subagents/<agent>.jsonl`.
fn session_of_path(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    if parent.file_name().is_some_and(|name| name == "subagents") {
        return parent
            .parent()?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
    }
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

struct Interner {
    names: Vec<String>,
    index: HashMap<String, u32>,
}

impl Interner {
    fn new() -> Self {
        Self {
            names: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn of(&mut self, name: &str) -> u32 {
        if let Some(at) = self.index.get(name) {
            return *at;
        }
        let at = self.names.len() as u32;
        self.names.push(name.to_string());
        self.index.insert(name.to_string(), at);
        at
    }
}

/// Read the transcripts of `input.roots` (the first copy of a reply
/// counts) and give each reply since `input.since_ms` its account.
/// Blocking.
pub fn scan(input: &ScanInput) -> Scan {
    let mut accounts = Interner::new();
    let mut models = Interner::new();
    let mut entries: Vec<Counted> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut roots_seen: HashSet<PathBuf> = HashSet::new();
    for root in &input.roots {
        let Ok(real) = std::fs::canonicalize(&root.dir) else {
            continue;
        };
        if !roots_seen.insert(real.clone()) {
            continue;
        }
        let mut files = Vec::new();
        transcripts(&real, input.since_ms, DEPTH, &mut files);
        files.sort();
        for file in files {
            read_file(
                &file,
                root.owner.as_deref(),
                input,
                &mut accounts,
                &mut models,
                &mut entries,
                &mut seen,
            );
        }
    }
    Scan {
        since_ms: input.since_ms,
        as_of_ms: input.now_ms,
        accounts: accounts.names,
        models: models.names,
        entries,
    }
}

#[allow(clippy::too_many_arguments)]
fn read_file(
    path: &Path,
    owner: Option<&str>,
    input: &ScanInput,
    accounts: &mut Interner,
    models: &mut Interner,
    entries: &mut Vec<Counted>,
    seen: &mut HashMap<String, usize>,
) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    let fallback_session = session_of_path(path);
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.len() > LINE_LIMIT {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            continue;
        };
        // Only replies carry usage; most lines are skipped unparsed.
        if !text.contains("\"usage\"") || !text.contains("\"assistant\"") {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Line>(text) else {
            continue;
        };
        if parsed.kind.as_deref() != Some("assistant") {
            continue;
        }
        let Some(message) = parsed.message else {
            continue;
        };
        let Some(usage) = message.usage else {
            continue;
        };
        let model = message.model.unwrap_or_default();
        if model.is_empty() || model == "<synthetic>" {
            continue;
        }
        let tokens = [
            usage.input_tokens.unwrap_or(0),
            usage.output_tokens.unwrap_or(0),
            usage.cache_read_input_tokens.unwrap_or(0),
            usage.cache_creation_input_tokens.unwrap_or(0),
        ];
        if tokens.iter().all(|n| *n == 0) {
            continue;
        }
        let Some(at_ms) = parsed
            .timestamp
            .as_deref()
            .and_then(super::time::parse_rfc3339_ms)
        else {
            continue;
        };
        if at_ms < input.since_ms {
            continue;
        }
        let key = match (&message.id, &parsed.request_id) {
            (Some(id), Some(request)) => Some(format!("{id}:{request}")),
            (Some(id), None) => Some(id.clone()),
            (None, Some(request)) => Some(format!(":{request}")),
            (None, None) => None,
        };
        // One reply is written as several lines (one per content block),
        // and a copied transcript holds it again: counted once, with the
        // largest figures any copy shows.
        if let Some(at) = key.as_ref().and_then(|key| seen.get(key)) {
            let counted = &mut entries[*at];
            counted.input = counted.input.max(tokens[0]);
            counted.output = counted.output.max(tokens[1]);
            counted.cache_read = counted.cache_read.max(tokens[2]);
            counted.cache_write = counted.cache_write.max(tokens[3]);
            continue;
        }
        let session = parsed
            .session_id
            .filter(|id| !id.is_empty())
            .or_else(|| fallback_session.clone())
            .unwrap_or_default();
        let (account, fallback) = match input
            .sessions
            .get(&session)
            .and_then(|r| r.folder_at(at_ms))
        {
            Some(Some(folder)) => (folder.as_str(), false),
            Some(None) => (
                registry::global_at(&input.history, &input.global_now, at_ms),
                false,
            ),
            None => match owner {
                Some(owner) => (owner, false),
                None => (
                    registry::global_at(&input.history, &input.global_now, at_ms),
                    true,
                ),
            },
        };
        if let Some(key) = key {
            seen.insert(key, entries.len());
        }
        entries.push(Counted {
            account: accounts.of(account),
            model: models.of(&model),
            at_ms,
            input: tokens[0],
            output: tokens[1],
            cache_read: tokens[2],
            cache_write: tokens[3],
            fallback,
        });
    }
}

impl Scan {
    /// Whether it answers for `since_ms` at `now`.
    fn answers(&self, since_ms: u64, now: u64) -> bool {
        self.since_ms <= since_ms && now.saturating_sub(self.as_of_ms) < CACHE_MS
    }

    /// `account`'s spend since `since_ms`, by model.
    pub fn spend(&self, account: &str, since_ms: u64) -> OutsideSpend {
        let mut by_model: HashMap<u32, ModelSpend> = HashMap::new();
        let mut attributed = 0;
        let index = self.accounts.iter().position(|name| name == account);
        for entry in &self.entries {
            if Some(entry.account as usize) != index || entry.at_ms < since_ms {
                continue;
            }
            let spend = by_model.entry(entry.model).or_insert_with(|| ModelSpend {
                model: self.models[entry.model as usize].clone(),
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                messages: 0,
            });
            spend.input += entry.input;
            spend.output += entry.output;
            spend.cache_read += entry.cache_read;
            spend.cache_write += entry.cache_write;
            spend.messages += 1;
            if entry.fallback {
                attributed += 1;
            }
        }
        let mut by_model: Vec<ModelSpend> = by_model.into_values().collect();
        by_model.sort_by(|a, b| a.model.cmp(&b.model));
        OutsideSpend {
            account: account.to_string(),
            since_ms,
            as_of_ms: self.as_of_ms,
            attributed,
            by_model,
        }
    }
}

impl super::Accounts {
    /// Note the Claude Code sessions the panes show now (only with
    /// `accounts.json`: without it every session is `default`'s).
    pub fn note_sessions(&self, panes: &HashMap<String, super::panes::PaneState>) {
        if !self.managed() {
            return;
        }
        let seen = panes.values().filter_map(|pane| {
            let session = pane.info.session_id.as_deref()?;
            pane.running().then_some((session, pane.place.as_ref()))
        });
        self.outside.note(seen, now_ms());
    }

    /// Where transcripts are read: `~/.claude/projects` first, then each
    /// account's own `projects/` (one shared with `~/.claude` is the same
    /// folder and read once).
    fn transcript_roots(&self, registry: &registry::Registry) -> Vec<Root> {
        let Some(paths) = self.paths() else {
            return Vec::new();
        };
        let mut roots = vec![Root {
            dir: paths.claude_home.join("projects"),
            owner: None,
        }];
        if self.managed() {
            let slot = std::fs::canonicalize(paths.claude_home.join("projects")).ok();
            for entry in &registry.accounts {
                let dir = paths.account_dir(&entry.id).join("projects");
                let real = std::fs::canonicalize(&dir).ok();
                if real.is_none() || real == slot {
                    continue;
                }
                roots.push(Root {
                    dir,
                    owner: Some(entry.id.clone()),
                });
            }
        }
        roots
    }

    /// `account`'s Claude Code spend outside the Chat since `since_ms`
    /// (see the module's description); `None` for an account that isn't
    /// on the list. `global` names the account global now.
    pub async fn spend_outside(
        self: &std::sync::Arc<Self>,
        account: &str,
        since_ms: u64,
    ) -> Option<OutsideSpend> {
        let registry = self.registry();
        let account = match account.trim() {
            super::GLOBAL => registry.global.clone(),
            other => other.to_string(),
        };
        if !registry.has(&account) {
            return None;
        }
        let now = now_ms();
        let mut cached = self.outside.scan.lock().await;
        if !cached
            .as_ref()
            .is_some_and(|scan| scan.answers(since_ms, now))
        {
            let input = ScanInput {
                roots: self.transcript_roots(&registry),
                sessions: self.outside.sessions(),
                history: registry.global_history.clone(),
                global_now: registry.global.clone(),
                since_ms: since_ms.min(now.saturating_sub(SCAN_BACK_MS)),
                now_ms: now,
            };
            let started = std::time::Instant::now();
            let scanned = tokio::task::spawn_blocking(move || scan(&input))
                .await
                .unwrap_or_default();
            tracing::debug!(
                entries = scanned.entries.len(),
                took_ms = started.elapsed().as_millis() as u64,
                "Claude Code transcripts read"
            );
            *cached = Some(scanned);
        }
        cached.as_ref().map(|scan| scan.spend(&account, since_ms))
    }
}

/// A time as milliseconds since the epoch (tests).
#[cfg(test)]
fn iso(at_ms: u64) -> String {
    let secs = (at_ms / 1000) as i64;
    let millis = at_ms % 1000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::test_support::Home;
    use std::sync::Arc;

    const HOUR: u64 = 3_600_000;
    const DAY: u64 = 24 * HOUR;

    fn reply(
        session: &str,
        id: &str,
        request: &str,
        model: &str,
        at_ms: u64,
        tokens: [u64; 4],
    ) -> String {
        serde_json::json!({
            "type": "assistant", "sessionId": session, "timestamp": iso(at_ms),
            "requestId": request, "uuid": uuid::Uuid::new_v4().to_string(),
            "message": {"id": id, "type": "message", "role": "assistant", "model": model,
                "content": [{"type": "text", "text": "ok"}],
                "usage": {"input_tokens": tokens[0], "output_tokens": tokens[1],
                    "cache_read_input_tokens": tokens[2], "cache_creation_input_tokens": tokens[3]}}
        })
        .to_string()
    }

    fn user(session: &str, at_ms: u64) -> String {
        serde_json::json!({"type": "user", "sessionId": session, "timestamp": iso(at_ms),
            "message": {"role": "user", "content": "the usage of \"assistant\" words"}})
        .to_string()
    }

    fn write(path: &Path, lines: &[String]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    fn registry_with(home: &Home, global: &str, history: Vec<GlobalChange>) {
        std::fs::create_dir_all(&home.paths.root).unwrap();
        let registry = serde_json::json!({
            "version": 1, "global": global, "slot_version": 2,
            "accounts": [
                {"id": "default", "name": "Claude Code", "color": "blue",
                 "shared": ["settings", "extensions", "plugins", "conversations"]},
                {"id": "personal", "name": "Personal", "color": "green",
                 "shared": ["settings", "extensions", "plugins", "conversations"], "created_at_ms": 1},
                {"id": "work", "name": "Work", "color": "pink", "shared": [], "created_at_ms": 2}
            ],
            "global_history": history,
        });
        std::fs::write(home.paths.registry_file(), registry.to_string()).unwrap();
    }

    fn total(spend: &OutsideSpend) -> (u64, u64) {
        (
            spend.by_model.iter().map(|m| m.messages).sum(),
            spend
                .by_model
                .iter()
                .map(|m| m.input + m.output + m.cache_read + m.cache_write)
                .sum(),
        )
    }

    #[tokio::test]
    async fn spend_goes_to_the_session_s_account_or_the_global_one_then() {
        let home = Home::new("outside");
        let now = now_ms();
        let week_start = now - 7 * DAY;
        let switched = now - 2 * DAY;
        // `default` was global until two days ago, `personal` since.
        registry_with(
            &home,
            "personal",
            vec![
                GlobalChange {
                    account: "default".into(),
                    at_ms: 0,
                },
                GlobalChange {
                    account: "personal".into(),
                    at_ms: switched,
                },
            ],
        );
        let slot_projects = home.paths.claude_home.join("projects");
        // `personal` shares conversations: its projects/ is the slot's.
        let personal = home.paths.account_dir("personal");
        std::fs::create_dir_all(&personal).unwrap();
        std::fs::create_dir_all(&slot_projects).unwrap();
        std::os::unix::fs::symlink(&slot_projects, personal.join("projects")).unwrap();
        let accounts = home.accounts();
        // tmux ran `s-work` on work's folder, then moved it to personal's;
        // `s-slot` on the slot.
        let moved_at = now - 3 * DAY;
        accounts.outside.note(
            [
                ("s-work", Some(&Place::Folder("work".into()))),
                ("s-slot", Some(&Place::Slot)),
            ],
            now - 5 * DAY,
        );
        accounts.outside.note(
            [("s-work", Some(&Place::Folder("personal".into())))],
            moved_at,
        );
        let project = slot_projects.join("-home-dev-app");
        write(
            &project.join("s-work.jsonl"),
            &[
                user("s-work", now - 4 * DAY),
                // Before tmux saw it: where it was first seen (work).
                reply(
                    "s-work",
                    "m1",
                    "r1",
                    "claude-opus-4-5",
                    now - 6 * DAY,
                    [10, 20, 30, 40],
                ),
                // One reply over two lines: counted once, largest figures.
                reply(
                    "s-work",
                    "m2",
                    "r2",
                    "claude-opus-4-5",
                    now - 4 * DAY,
                    [1, 5, 0, 0],
                ),
                reply(
                    "s-work",
                    "m2",
                    "r2",
                    "claude-opus-4-5",
                    now - 4 * DAY,
                    [1, 9, 0, 0],
                ),
                // After the move: personal.
                reply(
                    "s-work",
                    "m3",
                    "r3",
                    "claude-sonnet-4-5",
                    now - DAY,
                    [2, 2, 2, 2],
                ),
                // Older than the week: left out.
                reply(
                    "s-work",
                    "m0",
                    "r0",
                    "claude-opus-4-5",
                    now - 9 * DAY,
                    [100, 100, 0, 0],
                ),
            ],
        );
        // A sub-agent of the slot session, before and after the switch.
        write(
            &project.join("s-slot/subagents/agent-a1.jsonl"),
            &[
                reply(
                    "s-slot",
                    "m4",
                    "r4",
                    "claude-haiku-4-5",
                    now - 3 * DAY,
                    [1, 1, 1, 1],
                ),
                reply(
                    "s-slot",
                    "m5",
                    "r5",
                    "claude-haiku-4-5",
                    now - DAY,
                    [3, 3, 3, 3],
                ),
            ],
        );
        // A session tmux never ran, before and after the switch, plus a
        // synthetic message and an API error with no usage.
        write(
            &project.join("s-other.jsonl"),
            &[
                reply(
                    "s-other",
                    "m6",
                    "r6",
                    "claude-opus-4-5",
                    now - 4 * DAY,
                    [7, 7, 0, 0],
                ),
                reply(
                    "s-other",
                    "m7",
                    "r7",
                    "claude-opus-4-5",
                    now - HOUR,
                    [8, 8, 0, 0],
                ),
                reply(
                    "s-other",
                    "m8",
                    "r8",
                    "<synthetic>",
                    now - HOUR,
                    [1, 1, 0, 0],
                ),
                reply(
                    "s-other",
                    "m9",
                    "r9",
                    "claude-opus-4-5",
                    now - HOUR,
                    [0, 0, 0, 0],
                ),
            ],
        );
        // The same session copied into work's own folder (a move copies the
        // transcript): its replies count once.
        let work_projects = home
            .paths
            .account_dir("work")
            .join("projects/-home-dev-app");
        std::fs::create_dir_all(&work_projects).unwrap();
        std::fs::copy(
            project.join("s-other.jsonl"),
            work_projects.join("s-other.jsonl"),
        )
        .unwrap();
        // A session found only in work's own projects/: work's.
        write(
            &work_projects.join("s-by-hand.jsonl"),
            &[reply(
                "s-by-hand",
                "m10",
                "r10",
                "claude-opus-4-5",
                now - HOUR,
                [5, 5, 5, 5],
            )],
        );

        // m1 (before tmux saw it: where it was first seen), m2 (once, its
        // largest figures), m10 (in work's own projects/ alone).
        let work = accounts.spend_outside("work", week_start).await.unwrap();
        assert_eq!(work.attributed, 0);
        assert_eq!(total(&work), (3, 100 + 10 + 20));
        let personal = accounts
            .spend_outside("personal", week_start)
            .await
            .unwrap();
        // m3 (moved there), m5 (slot after the switch), m7 (other session).
        assert_eq!(total(&personal), (3, 8 + 12 + 16));
        assert_eq!(personal.attributed, 1);
        let default = accounts.spend_outside("default", week_start).await.unwrap();
        // m4 (the slot before the switch), m6 (another session, then; its
        // copy in work's folder counted once).
        assert_eq!(total(&default), (2, 4 + 14));
        assert_eq!(default.attributed, 1);
        let opus = default
            .by_model
            .iter()
            .find(|m| m.model == "claude-opus-4-5")
            .unwrap();
        assert_eq!((opus.input, opus.output, opus.messages), (7, 7, 1));
        assert_eq!(default.by_model.len(), 2);
        // `global` names the account global now.
        let global = accounts.spend_outside("global", week_start).await.unwrap();
        assert_eq!(global.account, "personal");
        assert_eq!(total(&global), total(&personal));
        // A later start answers from the same scan; unknown accounts none.
        let recent = accounts
            .spend_outside("personal", now - 2 * HOUR)
            .await
            .unwrap();
        assert_eq!(total(&recent).0, 1);
        assert_eq!(recent.as_of_ms, personal.as_of_ms);
        assert!(accounts.spend_outside("nobody", week_start).await.is_none());
        // Cached: a reply written now shows only after the scan is redone.
        write(
            &project.join("s-late.jsonl"),
            &[reply(
                "s-late",
                "m11",
                "r11",
                "claude-opus-4-5",
                now_ms(),
                [1, 0, 0, 0],
            )],
        );
        let again = accounts
            .spend_outside("personal", week_start)
            .await
            .unwrap();
        assert_eq!(total(&again).0, 3);
        accounts.outside.forget_scan().await;
        let fresh = accounts
            .spend_outside("personal", week_start)
            .await
            .unwrap();
        assert_eq!(total(&fresh).0, 4);
    }

    #[test]
    fn sessions_are_noted_once_per_place_and_kept_on_disk() {
        let home = Home::new("outside-log");
        std::fs::create_dir_all(&home.paths.root).unwrap();
        let outside = Outside::new(Some(&home.paths));
        let work = Place::Folder("work".into());
        outside.note([("s1", Some(&work)), ("s2", None)], 1_000);
        outside.note([("s1", Some(&work))], 2_000);
        outside.note([("s1", Some(&Place::Slot))], 3_000);
        let sessions = outside.sessions();
        assert!(
            !sessions.contains_key("s2"),
            "a session on no account's folder"
        );
        let s1 = &sessions["s1"];
        assert_eq!(
            s1.places,
            vec![
                SessionPlace {
                    folder: Some("work".into()),
                    at_ms: 1_000
                },
                SessionPlace {
                    folder: None,
                    at_ms: 3_000
                },
            ]
        );
        assert_eq!(s1.folder_at(500), Some(&Some("work".into())));
        assert_eq!(s1.folder_at(2_999), Some(&Some("work".into())));
        assert_eq!(s1.folder_at(3_000), Some(&None));
        // Read back by another process.
        let again = Outside::new(Some(&home.paths));
        assert_eq!(again.sessions()["s1"].places, s1.places);
        // Not seen for 35 days: dropped with the next write.
        again.note([("s3", Some(&work))], 3_000 + KEEP_MS);
        assert!(!again.sessions().contains_key("s1"));
    }

    #[tokio::test]
    async fn without_accounts_json_everything_is_the_default_account_s() {
        let home = Home::new("outside-bare");
        let now = now_ms();
        write(
            &home.paths.claude_home.join("projects/p/s.jsonl"),
            &[reply(
                "s",
                "m1",
                "r1",
                "claude-opus-4-5",
                now - HOUR,
                [1, 2, 3, 4],
            )],
        );
        let accounts: Arc<super::super::Accounts> = home.accounts();
        let spend = accounts.spend_outside("default", now - DAY).await.unwrap();
        assert_eq!(total(&spend), (1, 10));
        assert_eq!(spend.attributed, 1);
        assert!(!home.paths.root.join(SESSIONS_FILE).exists());
    }
}
