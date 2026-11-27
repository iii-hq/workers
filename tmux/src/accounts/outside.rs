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
//!   that changed, in `<accounts root>/claude-sessions.json`. Its replies
//!   go to that folder's account at the reply's time, or, on the slot, to
//!   the account global then.
//! - **Other sessions** go to the account global at the reply's time
//!   (`global_history`), counted in `attributed`; one found only in an
//!   account's own `projects/` (not shared with `~/.claude`) goes to that
//!   account, since only Claude Code on that folder writes there.
//!
//! A reply counts once, by its message id (its request id when it has
//! none), across lines and folders (a moved conversation's transcript is
//! copied), with the largest figures any of its lines shows. The
//! transcripts are read only when asked, at most every five minutes, and
//! only what was added since the last read ([`Ledger`]): each file's read
//! offset is kept, and replies are summed by session, folder, model and
//! minute, which is also the precision of the attribution. The Chat's
//! provider writes no transcript, so nothing is counted twice.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
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
/// How long a reading answers before the transcripts are read again.
pub const CACHE_MS: u64 = 5 * 60_000;
/// The ledger reaches at least this far back, so the accounts' weeks
/// (each with its own reset) are answered by one ledger.
const SCAN_BACK_MS: u64 = 8 * 24 * 3_600_000;
/// A session not seen for this long leaves the sessions file.
const KEEP_MS: u64 = 35 * 24 * 3_600_000;
/// How often a session still seen where it was is written down again.
const SEEN_EVERY_MS: u64 = 10 * 60_000;
/// How deep below `projects/` transcripts are looked for
/// (`<project>/<session>/subagents/<agent>.jsonl`).
const DEPTH: usize = 4;
/// The longest line kept in memory whole.
const LINE_WHOLE: usize = 1024 * 1024;
/// How much of the end of a longer line is read to tell whether it is a
/// reply (its usage and type come after its content).
const LINE_TAIL: u64 = 64 * 1024;
/// A line longer than this is skipped unread.
const LINE_MAX: u64 = 256 * 1024 * 1024;
/// Replies are summed by this span: the precision of the attribution.
const BUCKET_MS: u64 = 60_000;
/// How often replies that left the window are dropped from memory.
const PRUNE_EVERY_MS: u64 = 3_600_000;
/// No owner (`~/.claude/projects`).
const NO_OWNER: u32 = u32::MAX;

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
    /// False when an unreadable file could not be moved aside: it is
    /// never written over.
    writable: bool,
    file: SessionFile,
    written_at_ms: u64,
}

/// The sessions tmux saw and the transcripts read so far.
pub struct Outside {
    file: Option<PathBuf>,
    log: Mutex<SessionLog>,
    ledger: tokio::sync::Mutex<Option<Ledger>>,
}

impl Outside {
    pub fn new(paths: Option<&ClaudePaths>) -> Self {
        Self {
            file: paths.map(|paths| paths.root.join(SESSIONS_FILE)),
            log: Mutex::new(SessionLog::default()),
            ledger: tokio::sync::Mutex::new(None),
        }
    }

    fn log(&self) -> std::sync::MutexGuard<'_, SessionLog> {
        let mut log = self.log.lock().unwrap_or_else(|p| p.into_inner());
        if !log.loaded {
            log.loaded = true;
            log.writable = true;
            if let Some(file) = &self.file {
                match std::fs::read_to_string(file) {
                    Ok(text) => match serde_json::from_str(&text) {
                        Ok(read) => log.file = read,
                        Err(error) => {
                            tracing::warn!(%error, "claude-sessions.json unreadable");
                            log.writable = set_aside(file);
                        }
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        tracing::warn!(%error, "claude-sessions.json unreadable");
                        log.writable = set_aside(file);
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
        if (changed || due) && log.writable {
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

    /// Read the transcripts again on the next ask, from where each was
    /// left (tests).
    pub async fn read_again(&self) {
        if let Some(ledger) = self.ledger.lock().await.as_mut() {
            ledger.as_of_ms = 0;
        }
    }
}

/// Move an unreadable `claude-sessions.json` aside
/// (`claude-sessions.json.corrupt-<ms>`), so a new one never replaces it;
/// whether that worked.
fn set_aside(file: &Path) -> bool {
    let aside = file.with_file_name(format!("{SESSIONS_FILE}.corrupt-{}", now_ms()));
    match std::fs::rename(file, &aside) {
        Ok(()) => {
            tracing::warn!(to = %aside.display(), "claude-sessions.json moved aside; started again");
            true
        }
        Err(error) => {
            tracing::warn!(%error, "claude-sessions.json could not be moved aside; not written");
            false
        }
    }
}

/// Where transcripts are read: a `projects/` folder, and the account it
/// belongs to when it is an account's own (not `~/.claude`'s).
#[derive(Debug, Clone)]
pub struct Root {
    pub dir: PathBuf,
    pub owner: Option<String>,
}

/// How far a transcript was read.
#[derive(Debug, Clone, Copy)]
struct FileState {
    dev: u64,
    ino: u64,
    len: u64,
    modified_ms: u64,
    /// The end of the last whole line read.
    offset: u64,
}

/// What replies are summed by: their session, the account whose own
/// `projects/` held them ([`NO_OWNER`] for `~/.claude`'s), their model and
/// their minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BucketKey {
    session: u32,
    owner: u32,
    model: u32,
    minute: u64,
}

#[derive(Debug, Clone)]
struct Bucket {
    key: BucketKey,
    tokens: [u64; 4],
    messages: u64,
}

/// One reply counted: its bucket and the figures it added there.
#[derive(Debug, Clone, Copy)]
struct Reply {
    bucket: u32,
    tokens: [u32; 4],
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

/// The replies read from the transcripts since `window_ms`, summed, and
/// how far each transcript was read: kept between asks, so a new read
/// only reads what Claude Code added since.
pub struct Ledger {
    /// Replies from here on are kept.
    window_ms: u64,
    /// When the transcripts were last read.
    as_of_ms: u64,
    pruned_ms: u64,
    files: HashMap<PathBuf, FileState>,
    sessions: Interner,
    owners: Interner,
    models: Interner,
    buckets: Vec<Bucket>,
    index: HashMap<BucketKey, u32>,
    /// Each reply by a hash of its message id (or request id).
    replies: HashMap<u64, Reply>,
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

/// The `.jsonl` files below `dir` changed since `since_ms`, with their
/// metadata, never through a symbolic link.
fn transcripts(
    dir: &Path,
    since_ms: u64,
    depth: usize,
    out: &mut Vec<(PathBuf, std::fs::Metadata)>,
) {
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
            && let Ok(meta) = entry.metadata()
            && modified_ms(&meta) >= since_ms
        {
            out.push((path, meta));
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

/// Read one line from `reader`, keeping at most `limit` bytes of it in
/// `keep`: its whole length (the newline included), and whether it ended
/// with one (else the file ended first: a line still being written).
fn read_line(
    reader: &mut impl BufRead,
    keep: &mut Vec<u8>,
    limit: usize,
) -> std::io::Result<(u64, bool)> {
    let mut total = 0u64;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok((total, false));
        }
        let (taken, done) = match buf.iter().position(|byte| *byte == b'\n') {
            Some(at) => (at + 1, true),
            None => (buf.len(), false),
        };
        let room = limit.saturating_sub(keep.len());
        keep.extend_from_slice(&buf[..taken.min(room)]);
        total += taken as u64;
        reader.consume(taken);
        if done {
            return Ok((total, true));
        }
    }
}

/// Whether a line may be a reply with usage (most lines are skipped
/// unparsed).
fn may_be_reply(text: &[u8]) -> bool {
    let has = |needle: &[u8]| text.windows(needle.len()).any(|w| w == needle);
    has(b"\"usage\"") && has(b"\"assistant\"")
}

/// A line too long to keep, parsed from the file field by field: its
/// content is skipped, never held.
fn parse_long(path: &Path, start: u64, len: u64) -> Option<Line> {
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(start)).ok()?;
    let reader = BufReader::with_capacity(64 * 1024, file.take(len));
    let mut de = serde_json::Deserializer::from_reader(reader);
    Line::deserialize(&mut de).ok()
}

/// The end of a line too long to keep, for [`may_be_reply`].
fn tail_of(path: &Path, start: u64, len: u64) -> Vec<u8> {
    let from = start + len.saturating_sub(LINE_TAIL);
    let mut tail = Vec::new();
    if let Ok(mut file) = std::fs::File::open(path)
        && file.seek(SeekFrom::Start(from)).is_ok()
    {
        let _ = file.take(start + len - from).read_to_end(&mut tail);
    }
    tail
}

/// A reply's key: its message id, else its request id.
fn reply_key(message_id: Option<&str>, request_id: Option<&str>) -> Option<u64> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match (message_id, request_id) {
        (Some(id), _) if !id.is_empty() => ("m", id).hash(&mut hasher),
        (_, Some(request)) if !request.is_empty() => ("r", request).hash(&mut hasher),
        _ => return None,
    }
    Some(hasher.finish())
}

impl Ledger {
    /// An empty ledger keeping replies from `window_ms` on.
    pub fn new(window_ms: u64) -> Self {
        Self {
            window_ms,
            as_of_ms: 0,
            pruned_ms: 0,
            files: HashMap::new(),
            sessions: Interner::new(),
            owners: Interner::new(),
            models: Interner::new(),
            buckets: Vec::new(),
            index: HashMap::new(),
            replies: HashMap::new(),
        }
    }

    /// Whether it answers for `since_ms` at `now` without reading again.
    fn answers(&self, since_ms: u64, now: u64) -> bool {
        self.window_ms <= since_ms && now.saturating_sub(self.as_of_ms) < CACHE_MS
    }

    /// Read what was added to the transcripts of `roots` since the last
    /// read (the first copy of a reply decides its bucket), at `now`.
    /// Blocking.
    pub fn update(&mut self, roots: &[Root], now: u64) {
        let mut roots_seen: HashSet<PathBuf> = HashSet::new();
        for root in roots {
            let Ok(real) = std::fs::canonicalize(&root.dir) else {
                continue;
            };
            if !roots_seen.insert(real.clone()) {
                continue;
            }
            let owner = root
                .owner
                .as_deref()
                .map_or(NO_OWNER, |owner| self.owners.of(owner));
            let mut files = Vec::new();
            transcripts(&real, self.window_ms, DEPTH, &mut files);
            files.sort_by(|a, b| a.0.cmp(&b.0));
            for (path, meta) in files {
                self.update_file(&path, &meta, owner);
            }
        }
        self.as_of_ms = now;
        if now.saturating_sub(self.pruned_ms) >= PRUNE_EVERY_MS {
            self.prune(now);
        }
    }

    /// Read `path` from where it was left; from its start when it is
    /// another file now (replaced, or cut shorter).
    fn update_file(&mut self, path: &Path, meta: &std::fs::Metadata, owner: u32) {
        let (dev, ino, len, modified) = (meta.dev(), meta.ino(), meta.len(), modified_ms(meta));
        let from = match self.files.get(path) {
            Some(state) if state.dev == dev && state.ino == ino && state.offset <= len => {
                if state.len == len && state.modified_ms == modified {
                    return;
                }
                state.offset
            }
            _ => 0,
        };
        let offset = self.read_from(path, from, owner);
        self.files.insert(
            path.to_path_buf(),
            FileState {
                dev,
                ino,
                len,
                modified_ms: modified,
                offset,
            },
        );
    }

    /// Count the whole lines of `path` from byte `from`: the end of the
    /// last one counted (where the next read starts, also after an error).
    fn read_from(&mut self, path: &Path, from: u64, owner: u32) -> u64 {
        let Ok(mut file) = std::fs::File::open(path) else {
            return from;
        };
        if file.seek(SeekFrom::Start(from)).is_err() {
            return from;
        }
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let fallback_session = session_of_path(path);
        let mut offset = from;
        let mut line = Vec::new();
        loop {
            line.clear();
            let Ok((len, whole)) = read_line(&mut reader, &mut line, LINE_WHOLE) else {
                break;
            };
            if len == 0 || !whole {
                // The end, or a line Claude Code is still writing: read
                // again next time.
                break;
            }
            let start = offset;
            offset += len;
            let parsed = if len as usize <= LINE_WHOLE {
                if !may_be_reply(&line) {
                    continue;
                }
                match serde_json::from_slice::<Line>(&line) {
                    Ok(parsed) => parsed,
                    Err(_) => continue,
                }
            } else {
                if len > LINE_MAX {
                    continue;
                }
                let mut ends = std::mem::take(&mut line);
                ends.extend_from_slice(&tail_of(path, start, len));
                let reply = may_be_reply(&ends);
                line = ends;
                if !reply {
                    continue;
                }
                match parse_long(path, start, len) {
                    Some(parsed) => parsed,
                    None => continue,
                }
            };
            self.count(parsed, owner, fallback_session.as_deref());
        }
        offset
    }

    /// Count one parsed line: a reply with usage since the window, once.
    fn count(&mut self, parsed: Line, owner: u32, fallback_session: Option<&str>) {
        if parsed.kind.as_deref() != Some("assistant") {
            return;
        }
        let Some(message) = parsed.message else {
            return;
        };
        let Some(usage) = message.usage else {
            return;
        };
        let model = message.model.unwrap_or_default();
        if model.is_empty() || model == "<synthetic>" {
            return;
        }
        let tokens = [
            usage.input_tokens.unwrap_or(0),
            usage.output_tokens.unwrap_or(0),
            usage.cache_read_input_tokens.unwrap_or(0),
            usage.cache_creation_input_tokens.unwrap_or(0),
        ];
        if tokens.iter().all(|n| *n == 0) {
            return;
        }
        let Some(at_ms) = parsed
            .timestamp
            .as_deref()
            .and_then(super::time::parse_rfc3339_ms)
        else {
            return;
        };
        if at_ms < self.window_ms {
            return;
        }
        let key = reply_key(message.id.as_deref(), parsed.request_id.as_deref());
        let small = tokens.map(|n| u32::try_from(n).unwrap_or(u32::MAX));
        // One reply is written as several lines (one per content block),
        // and a copied transcript holds it again: counted once, with the
        // largest figures any copy shows.
        if let Some(reply) = key.and_then(|key| self.replies.get_mut(&key)) {
            let bucket = &mut self.buckets[reply.bucket as usize];
            for (at, figure) in small.iter().enumerate() {
                if *figure > reply.tokens[at] {
                    bucket.tokens[at] += u64::from(*figure - reply.tokens[at]);
                    reply.tokens[at] = *figure;
                }
            }
            return;
        }
        let session = parsed
            .session_id
            .filter(|id| !id.is_empty())
            .or_else(|| fallback_session.map(str::to_string))
            .unwrap_or_default();
        let bucket_key = BucketKey {
            session: self.sessions.of(&session),
            owner,
            model: self.models.of(&model),
            minute: at_ms / BUCKET_MS,
        };
        let at = match self.index.get(&bucket_key) {
            Some(at) => *at,
            None => {
                let at = self.buckets.len() as u32;
                self.buckets.push(Bucket {
                    key: bucket_key,
                    tokens: [0; 4],
                    messages: 0,
                });
                self.index.insert(bucket_key, at);
                at
            }
        };
        let bucket = &mut self.buckets[at as usize];
        for (sum, figure) in bucket.tokens.iter_mut().zip(small) {
            *sum += u64::from(figure);
        }
        bucket.messages += 1;
        if let Some(key) = key {
            self.replies.insert(
                key,
                Reply {
                    bucket: at,
                    tokens: small,
                },
            );
        }
    }

    /// Drop what left the window (eight days back now), and what is known
    /// of transcripts not changed since.
    fn prune(&mut self, now: u64) {
        self.pruned_ms = now;
        self.window_ms = self.window_ms.max(now.saturating_sub(SCAN_BACK_MS));
        let first_minute = self.window_ms / BUCKET_MS;
        let mut moved: Vec<Option<u32>> = Vec::with_capacity(self.buckets.len());
        let mut kept = Vec::new();
        for bucket in self.buckets.drain(..) {
            if bucket.key.minute < first_minute {
                moved.push(None);
            } else {
                moved.push(Some(kept.len() as u32));
                kept.push(bucket);
            }
        }
        self.buckets = kept;
        self.index = self
            .buckets
            .iter()
            .enumerate()
            .map(|(at, bucket)| (bucket.key, at as u32))
            .collect();
        self.replies
            .retain(|_, reply| match moved[reply.bucket as usize] {
                Some(at) => {
                    reply.bucket = at;
                    true
                }
                None => false,
            });
        let window = self.window_ms;
        self.files.retain(|_, state| state.modified_ms >= window);
    }

    /// `account`'s spend since `since_ms`, by model: each bucket goes to
    /// the account its session ran on at its minute (`sessions`), else to
    /// the owner of the `projects/` it was read from, else to the account
    /// global then (`history`, counted in `attributed`).
    pub fn spend(
        &self,
        account: &str,
        since_ms: u64,
        sessions: &HashMap<String, SessionRecord>,
        history: &[GlobalChange],
        global_now: &str,
    ) -> OutsideSpend {
        let mut by_model: HashMap<u32, ModelSpend> = HashMap::new();
        let mut attributed = 0;
        let first_minute = since_ms / BUCKET_MS;
        for bucket in &self.buckets {
            let key = bucket.key;
            if key.minute < first_minute {
                continue;
            }
            let at_ms = key.minute * BUCKET_MS;
            let session = &self.sessions.names[key.session as usize];
            let (owner, fallback) = match sessions.get(session).and_then(|r| r.folder_at(at_ms)) {
                Some(Some(folder)) => (folder.as_str(), false),
                Some(None) => (registry::global_at(history, global_now, at_ms), false),
                None if key.owner != NO_OWNER => {
                    (self.owners.names[key.owner as usize].as_str(), false)
                }
                None => (registry::global_at(history, global_now, at_ms), true),
            };
            if owner != account {
                continue;
            }
            let spend = by_model.entry(key.model).or_insert_with(|| ModelSpend {
                model: self.models.names[key.model as usize].clone(),
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                messages: 0,
            });
            spend.input += bucket.tokens[0];
            spend.output += bucket.tokens[1];
            spend.cache_read += bucket.tokens[2];
            spend.cache_write += bucket.tokens[3];
            spend.messages += bucket.messages;
            if fallback {
                attributed += bucket.messages;
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
        let mut kept = self.outside.ledger.lock().await;
        if !kept
            .as_ref()
            .is_some_and(|ledger| ledger.answers(since_ms, now))
        {
            // Asked further back than it reaches: read it all again.
            let mut ledger = match kept.take() {
                Some(ledger) if ledger.window_ms <= since_ms => ledger,
                _ => Ledger::new(since_ms.min(now.saturating_sub(SCAN_BACK_MS))),
            };
            let roots = self.transcript_roots(&registry);
            let started = std::time::Instant::now();
            let read = tokio::task::spawn_blocking(move || {
                ledger.update(&roots, now);
                ledger
            })
            .await;
            match read {
                Ok(ledger) => {
                    tracing::debug!(
                        buckets = ledger.buckets.len(),
                        replies = ledger.replies.len(),
                        took_ms = started.elapsed().as_millis() as u64,
                        "Claude Code transcripts read"
                    );
                    *kept = Some(ledger);
                }
                Err(error) => tracing::warn!(%error, "Claude Code transcripts not read"),
            }
        }
        let ledger = kept.as_ref()?;
        Some(ledger.spend(
            &account,
            since_ms,
            &self.outside.sessions(),
            &registry.global_history,
            &registry.global,
        ))
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
        accounts.outside.read_again().await;
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

    /// `ledger`'s messages and tokens for `default`, the account global
    /// all along, since `since_ms`.
    fn counted(ledger: &Ledger, since_ms: u64) -> (u64, u64) {
        total(&ledger.spend("default", since_ms, &HashMap::new(), &[], "default"))
    }

    #[test]
    fn transcripts_are_read_from_where_they_were_left() {
        let home = Home::new("outside-ledger");
        let now = now_ms();
        let dir = home.paths.claude_home.join("projects");
        let file = dir.join("p/s.jsonl");
        let roots = [Root {
            dir: dir.clone(),
            owner: None,
        }];
        let first = reply("s", "m1", "r1", "claude-opus-4-5", now - HOUR, [1, 1, 0, 0]);
        let second = reply("s", "m2", "r2", "claude-opus-4-5", now - HOUR, [2, 2, 0, 0]);
        // The second line is still being written.
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let (head, rest) = second.split_at(second.len() / 2);
        std::fs::write(&file, format!("{first}\n{head}")).unwrap();
        let mut ledger = Ledger::new(now - DAY);
        ledger.update(&roots, now);
        assert_eq!(counted(&ledger, now - DAY), (1, 2));
        assert_eq!(ledger.files[&file].offset, first.len() as u64 + 1);
        // Finished, and the first reply's last line with larger figures.
        let more = reply("s", "m1", "r1", "claude-opus-4-5", now - HOUR, [1, 5, 0, 0]);
        {
            use std::io::Write;
            let mut out = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            write!(out, "{rest}\n{more}\n").unwrap();
        }
        ledger.update(&roots, now + 1);
        assert_eq!(counted(&ledger, now - DAY), (2, 6 + 4));
        let read_to = std::fs::metadata(&file).unwrap().len();
        assert_eq!(ledger.files[&file].offset, read_to);
        // Nothing new: nothing read.
        ledger.update(&roots, now + 2);
        assert_eq!(counted(&ledger, now - DAY), (2, 10));
        // Replaced by another file (a copy put back): read from its start,
        // what was counted counted once.
        let third = reply(
            "s",
            "m3",
            "r3",
            "claude-haiku-4-5",
            now - HOUR,
            [3, 0, 0, 0],
        );
        let replacement = file.with_extension("new");
        std::fs::write(&replacement, format!("{first}\n{second}\n{third}\n")).unwrap();
        std::fs::rename(&replacement, &file).unwrap();
        ledger.update(&roots, now + 3);
        assert_eq!(counted(&ledger, now - DAY), (3, 13));
        // Cut shorter in place: read from its start too.
        let fourth = reply(
            "s",
            "m4",
            "r4",
            "claude-haiku-4-5",
            now - HOUR,
            [4, 0, 0, 0],
        );
        std::fs::write(&file, format!("{fourth}\n")).unwrap();
        ledger.update(&roots, now + 4);
        assert_eq!(counted(&ledger, now - DAY), (4, 17));
        // Counted by the minute: a later start leaves earlier minutes out.
        assert_eq!(counted(&ledger, now - HOUR + BUCKET_MS), (0, 0));
    }

    #[test]
    fn a_line_too_long_to_keep_is_read_without_keeping_it() {
        let home = Home::new("outside-long");
        let now = now_ms();
        let dir = home.paths.claude_home.join("projects");
        let long_text = "x".repeat(LINE_WHOLE + LINE_WHOLE / 2);
        let mut long_reply: serde_json::Value = serde_json::from_str(&reply(
            "s",
            "m1",
            "r1",
            "claude-opus-4-5",
            now - HOUR,
            [10, 20, 0, 0],
        ))
        .unwrap();
        long_reply["message"]["content"][0]["text"] = serde_json::Value::String(long_text.clone());
        let long_user = serde_json::json!({"type": "user", "sessionId": "s",
            "timestamp": iso(now - HOUR), "message": {"role": "user", "content": long_text}});
        write(
            &dir.join("p/s.jsonl"),
            &[
                long_user.to_string(),
                long_reply.to_string(),
                reply("s", "m2", "r2", "claude-opus-4-5", now - HOUR, [1, 1, 0, 0]),
            ],
        );
        let mut ledger = Ledger::new(now - DAY);
        ledger.update(
            &[Root {
                dir: dir.clone(),
                owner: None,
            }],
            now,
        );
        assert_eq!(counted(&ledger, now - DAY), (2, 32));
    }

    #[test]
    fn an_unreadable_sessions_file_is_moved_aside_before_a_new_one() {
        let home = Home::new("outside-corrupt");
        std::fs::create_dir_all(&home.paths.root).unwrap();
        let file = home.paths.root.join(SESSIONS_FILE);
        std::fs::write(&file, "{\"sessions\": {\"s1\": [cut").unwrap();
        let outside = Outside::new(Some(&home.paths));
        outside.note([("s2", Some(&Place::Folder("work".into())))], 1_000);
        let aside: Vec<_> = std::fs::read_dir(&home.paths.root)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(&format!("{SESSIONS_FILE}.corrupt-")))
            .collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(
            std::fs::read_to_string(home.paths.root.join(&aside[0])).unwrap(),
            "{\"sessions\": {\"s1\": [cut"
        );
        let again = Outside::new(Some(&home.paths));
        assert!(again.sessions().contains_key("s2"));
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
