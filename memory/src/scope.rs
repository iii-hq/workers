//! Which bank a turn, its extraction pass, and its agent's `memory::*`
//! calls use: one resolver for all three, so a session reads and writes
//! the same bank.
//!
//! Order: turn metadata `memory_bank` → session metadata `memory_bank` →
//! the parent turn's bank, else the parent session's, walked up (a sub-agent
//! works in its parent's bank) → the bank of the turn's `fs_scope.root`
//! repository, else of that directory ([`root_bank`]) → `default_bank`.
//! A bank the directory chose reads `default_bank` under its own
//! ([`Resolved::layer`]); so does the session's own `agent_profile` bank,
//! `agent-<id>`, under any bank no metadata named ([`Resolved::layers`]).
//! `""` is absent. An invalid name, a deleted bank
//! other than the default, or a parent that no longer exists means NO
//! memory — never a fallback to another bank. A failed session lookup is
//! an error: the hook injects nothing new, extraction retries, a call
//! fails.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::store::validate_name;
use crate::types::{now_ms, Memory};

/// Parent hops followed before giving up on a sub-agent chain.
const MAX_PARENT_DEPTH: usize = 8;

/// `memory_bank` in a metadata object; `""` or a non-string is absent.
pub(crate) fn bank_field(metadata: Option<&Value>) -> Option<String> {
    metadata?
        .get("memory_bank")
        .and_then(Value::as_str)
        .filter(|b| !b.is_empty())
        .map(str::to_string)
}

/// `fs_scope.root` (the turn's working directory) in a metadata object.
fn root_field(metadata: Option<&Value>) -> Option<&str> {
    metadata?
        .pointer("/fs_scope/root")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty())
}

/// The `agent_profile` id in a metadata object: the harness's snapshot
/// `{ "id": … }`, or a bare id. The bundled general identities read as no
/// profile: the console preselects `default` for every new chat, so it names
/// no role whose rules belong apart from the user's.
pub(crate) fn profile_field(metadata: &Value) -> Option<String> {
    let profile = metadata.get("agent_profile")?;
    profile
        .get("id")
        .unwrap_or(profile)
        .as_str()
        .filter(|id| !id.is_empty() && !["default", "iii-minimal", "iii"].contains(id))
        .map(str::to_string)
}

/// The bank an agent profile's own memory lives in, across repositories.
pub(crate) fn agent_bank(profile: &str) -> String {
    format!("agent-{profile}")
}

/// The bank a working directory selects: `<name>-<6 hex>`, the name from
/// the repository holding `root` (its main checkout's folder, so every
/// worktree of a repo shares one bank), else from `root` itself; the hex
/// from the canonical path, so two folders of one name never share. `None`
/// when `root` does not exist.
pub(crate) fn root_bank(root: &Path) -> Option<String> {
    // Canonical first: a missing path under a repository selects nothing,
    // and a symlink is discovered from where it points.
    let root = root.canonicalize().ok()?;
    let hashed = match common_dir(&root) {
        Some(common) => common.canonicalize().ok()?,
        None => root,
    };
    // `<checkout>/.git` names the checkout; a bare `<name>.git` itself.
    let named = match hashed.file_name() {
        Some(n) if n == ".git" => hashed.parent().unwrap_or(&hashed),
        _ => &hashed,
    };
    let name = named.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let name = name.strip_suffix(".git").unwrap_or(name);
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        let c = if c.is_ascii_alphanumeric() || c == '_' {
            c
        } else {
            '-'
        };
        if !(c == '-' && slug.ends_with('-')) {
            slug.push(c);
        }
    }
    let slug: String = slug.trim_matches(['-', '_']).chars().take(57).collect();
    let slug = slug.trim_end_matches(['-', '_']);
    let slug = if slug.is_empty() { "dir" } else { slug };
    let hash = fnv1a64(hashed.as_os_str().as_encoded_bytes()) & 0xff_ffff;
    Some(format!("{slug}-{hash:06x}"))
}

/// Whether `bank` has [`root_bank`]'s shape, `<slug>-<6 lowercase hex>`.
// ponytail: by name — a hand-named `notes-c0ffee` previews layered too;
// record the root in bank.yaml if that ever misleads.
pub(crate) fn directory_shaped(bank: &str) -> bool {
    bank.rsplit_once('-').is_some_and(|(slug, hex)| {
        !slug.is_empty()
            && hex.len() == 6
            && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// FNV-1a: stable across builds, unlike `DefaultHasher`, so a bank name
/// never moves on an upgrade.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The common git dir of the repository `dir` sits in, found the way git
/// finds it: the nearest ancestor holding a `.git` directory, or a `.git`
/// file naming one (`gitdir: <path>`, a linked worktree), with a HEAD; its
/// `commondir` when it has one. Mirrors `ide`'s `git_events::discover`.
fn common_dir(dir: &Path) -> Option<PathBuf> {
    let joined = |base: &Path, rel: &str| -> PathBuf {
        let rel = Path::new(rel);
        if rel.is_absolute() {
            rel.to_path_buf()
        } else {
            base.join(rel)
        }
    };
    for top in dir.ancestors() {
        let dot = top.join(".git");
        let Ok(meta) = std::fs::metadata(&dot) else {
            continue;
        };
        let git_dir = if meta.is_dir() {
            dot
        } else {
            let Some(rel) = std::fs::read_to_string(&dot).ok().and_then(|text| {
                text.lines()
                    .next()
                    .and_then(|l| l.strip_prefix("gitdir:"))
                    .map(|r| r.trim().to_string())
            }) else {
                continue;
            };
            joined(top, &rel)
        };
        if !git_dir.join("HEAD").is_file() {
            continue;
        }
        return Some(match std::fs::read_to_string(git_dir.join("commondir")) {
            Ok(text) if !text.trim().is_empty() => joined(&git_dir, text.trim()),
            _ => git_dir,
        });
    }
    None
}

/// A resolved bank, whether the turn's `fs_scope.root` chose it, and the
/// session's agent profile read under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub bank: String,
    pub derived: bool,
    /// The session's own `agent_profile` id, never a parent's: its
    /// [`agent_bank`] is read under this bank. `None` without a profile, or
    /// under a bank metadata named ([`Resolved::takes_agents`]).
    pub agent: Option<String>,
}

impl Resolved {
    /// A bank named by metadata, a caller, or the default.
    pub(crate) fn named(bank: impl Into<String>) -> Self {
        Self {
            bank: bank.into(),
            derived: false,
            agent: None,
        }
    }

    /// Whether agent banks are read under this one: not under a bank
    /// metadata named, which is read alone, as for [`Resolved::layer`].
    // shortcut: `default_bank` named by metadata reads like the fallback
    // (agents layered); carry provenance in `Resolved` if that misleads.
    pub(crate) fn takes_agents(&self, default_bank: &str) -> bool {
        self.derived || self.bank == default_bank
    }

    /// Every bank read under this one, in rule order: [`Resolved::layer`],
    /// then the session's [`agent_bank`].
    pub(crate) fn layers(&self, default_bank: &str) -> Vec<String> {
        let agent = self.agent.as_deref().map(agent_bank);
        self.layer(default_bank)
            .map(str::to_string)
            .into_iter()
            .chain(agent)
            .collect()
    }

    /// The bank read under this one: `default_bank`, when the directory
    /// chose another — the user's own rules and memories about the user
    /// follow them into every repository. A named bank is read alone.
    pub(crate) fn layer<'a>(&self, default_bank: &'a str) -> Option<&'a str> {
        (self.derived && self.bank != default_bank).then_some(default_bank)
    }
}

/// The entity a layer's memories must carry to be read under a directory
/// bank: `default_bank` also holds project facts every repository saved
/// there before directory banks, which must stay in their own.
pub(crate) const LAYER_ENTITY: &str = "user";

/// Whether a layer memory is read under a directory bank ([`LAYER_ENTITY`]).
pub(crate) fn in_layer(memory: &Memory) -> bool {
    memory.entities.iter().any(|e| e == LAYER_ENTITY)
}

/// Whether a memory of `layer` is read under a turn's bank: every memory of
/// an agent bank; of `default_bank`, those about the user ([`in_layer`]).
pub(crate) fn layer_keeps(layer: &str, default_bank: &str, memory: &Memory) -> bool {
    layer != default_bank || in_layer(memory)
}

/// Tags `entities` with [`LAYER_ENTITY`] unless one already is.
pub(crate) fn into_layer(entities: &mut Vec<String>) {
    if !entities.iter().any(|e| e == LAYER_ENTITY) {
        entities.push(LAYER_ENTITY.to_string());
    }
}

/// The resolver over injected lookups, so tests need no engine: `recorded`
/// is the bank a turn was injected with ([`turn_bank`]), `lookup` a
/// session's metadata object (`None`: no such session), `from_root` the
/// bank a working directory selects ([`root_bank`]).
pub(crate) async fn resolve_with<L, Fut>(
    turn_metadata: Option<&Value>,
    session_id: &str,
    default_bank: &str,
    deleted: impl Fn(&str) -> bool,
    recorded: impl Fn(&str) -> Option<Option<Resolved>>,
    from_root: impl Fn(&str) -> Option<String>,
    lookup: L,
) -> Result<Option<Resolved>, String>
where
    L: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Option<Value>, String>>,
{
    let mut chosen = match bank_field(turn_metadata) {
        Some(bank) => Resolved::named(bank),
        None => {
            // Only a chain that names no bank reaches the directory's.
            let top = || match root_field(turn_metadata).and_then(&from_root) {
                Some(bank) => Resolved {
                    bank,
                    derived: true,
                    agent: None,
                },
                None => Resolved::named(default_bank),
            };
            match walk(session_id, top, recorded, lookup).await? {
                Some(bank) => bank,
                None => return Ok(None),
            }
        }
    };
    if validate_name(&chosen.bank).is_err() {
        tracing::warn!(session_id, bank = %chosen.bank, "invalid memory_bank; no memory for this session");
        return Ok(None);
    }
    // The default materializes on first use, deleted or not (README).
    if chosen.bank != default_bank && deleted(&chosen.bank) {
        return Ok(None);
    }
    // An unusable agent bank only drops that layer, never the session's
    // memory; a deleted one is never brought back.
    let takes = chosen.takes_agents(default_bank);
    chosen.agent = chosen.agent.take().filter(|id| {
        let bank = agent_bank(id);
        takes && validate_name(&bank).is_ok() && !deleted(&bank)
    });
    Ok(Some(chosen))
}

/// The session's own bank, else a sub-agent's parent turn's (the bank that
/// turn was injected with, an override included, its layer kept) or the
/// parent session's, walked up; `top` at the top. `None` — no memory — for
/// a session that no longer exists, or a chain that cycles or runs past
/// [`MAX_PARENT_DEPTH`]: never `top` in their place. The bank carries
/// `session_id`'s own `agent_profile`: a sub-agent reads its own profile's
/// bank, never its parent's.
async fn walk<L, Fut>(
    session_id: &str,
    top: impl FnOnce() -> Resolved,
    recorded: impl Fn(&str) -> Option<Option<Resolved>>,
    mut lookup: L,
) -> Result<Option<Resolved>, String>
where
    L: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Option<Value>, String>>,
{
    let mut id = session_id.to_string();
    let mut agent = None;
    for depth in 0..=MAX_PARENT_DEPTH {
        let Some(meta) = lookup(id.clone()).await? else {
            return Ok(None);
        };
        if depth == 0 {
            agent = profile_field(&meta);
        }
        let own = |bank: Resolved| Resolved {
            agent: agent.clone(),
            ..bank
        };
        if let Some(bank) = bank_field(Some(&meta)) {
            return Ok(Some(own(Resolved::named(bank))));
        }
        let field = |key| {
            meta.get(key)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
        };
        if let Some(bank) = field("parent_turn_id").and_then(&recorded) {
            return Ok(bank.map(own));
        }
        match field("parent_session_id") {
            Some(parent) => id = parent.to_string(),
            None => return Ok(Some(own(top()))),
        }
    }
    Ok(None)
}

/// [`resolve_with`] against the live session store.
pub async fn resolve(
    deps: &Deps,
    session_id: &str,
    turn_metadata: Option<&Value>,
) -> Result<Option<Resolved>, String> {
    let cfg = deps.config().await;
    let store = deps.store().await;
    resolve_with(
        turn_metadata,
        session_id,
        &cfg.default_bank,
        |bank| store.is_deleted(bank),
        turn_bank,
        |root| root_bank(Path::new(root)),
        |id| session_metadata(&deps.iii, id),
    )
    .await
}

/// One session's `meta` record (`Null` when absent); `None` when the
/// session does not exist (`session::get` replies `null`).
pub(crate) async fn session_meta(
    iii: &IIIClient,
    session_id: String,
) -> Result<Option<Value>, String> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "session::get".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|e| format!("session::get {session_id}: {e}"))?;
    if reply.is_null() {
        return Ok(None);
    }
    Ok(Some(reply.get("meta").cloned().unwrap_or(Value::Null)))
}

/// One session's metadata object (`Null` when it has none); `None` when
/// the session does not exist.
async fn session_metadata(iii: &IIIClient, session_id: String) -> Result<Option<Value>, String> {
    Ok(session_meta(iii, session_id)
        .await?
        .map(|meta| meta.get("metadata").cloned().unwrap_or(Value::Null)))
}

/// The bank the hook stamped on `turn_id` (its `memory_bank`, `memory_layer`
/// and `memory_agent` annotations, which the harness merges into the origin
/// of each assistant entry the turn writes): the newest stamp on a
/// `session::messages-tail` page. Another hook can write `memory_bank`
/// too, so a name that is not a valid bank is no stamp.
pub(crate) fn stamped_bank(reply: &Value, turn_id: &str) -> Option<Resolved> {
    let (bank, origin) = reply
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .filter_map(|item| item.get("origin"))
        .filter(|origin| origin.get("turn_id").and_then(Value::as_str) == Some(turn_id))
        .find_map(|origin| bank_field(Some(origin)).map(|bank| (bank, origin)))?;
    validate_name(&bank).ok()?;
    let derived = origin
        .get("memory_layer")
        .and_then(Value::as_str)
        .is_some_and(|layer| !layer.is_empty());
    let agent = origin
        .get("memory_agent")
        .and_then(Value::as_str)
        .and_then(|bank| bank.strip_prefix("agent-"))
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    Some(Resolved {
        bank,
        derived,
        agent,
    })
}

/// Recent turns tracked in [`TURN_BANKS`]; the least recently used goes
/// first past this.
const TURN_BANKS_CAP: usize = 4_096;

/// Turn id → (its bank, last used ms).
type TurnBanks = HashMap<String, (Option<Resolved>, u64)>;

/// The bank each recent turn was injected with (`None`: no memory), so the
/// turn's extraction job and its agent's calls use exactly that bank, a
/// turn-only `memory_bank` override and its layer included.
// ponytail: in-process — a turn this process never saw (a restart) reads
// its bank back from the hook's stamp ([`stamped_bank`]). A turn with
// injection off, or whose every step's lookup failed, carries none: its job
// and calls re-resolve without the turn's metadata, which misses a
// turn-only override and the `fs_scope.root` bank (`default_bank` instead).
static TURN_BANKS: LazyLock<Mutex<TurnBanks>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn remember_in(turns: &mut TurnBanks, turn_id: &str, bank: Option<Resolved>, now_ms: u64) {
    if turns.len() >= TURN_BANKS_CAP && !turns.contains_key(turn_id) {
        if let Some(oldest) = turns
            .iter()
            .min_by_key(|(_, (_, used))| *used)
            .map(|(id, _)| id.clone())
        {
            turns.remove(&oldest);
        }
    }
    turns.insert(turn_id.to_string(), (bank, now_ms));
}

/// Record the bank `turn_id` was injected with.
pub(crate) fn remember_turn(turn_id: &str, bank: Option<Resolved>) {
    if turn_id.is_empty() {
        return;
    }
    let mut turns = TURN_BANKS.lock().unwrap_or_else(|e| e.into_inner());
    remember_in(&mut turns, turn_id, bank, now_ms());
}

/// The bank `turn_id` was injected with, when this process saw it.
pub(crate) fn turn_bank(turn_id: &str) -> Option<Option<Resolved>> {
    let turns = TURN_BANKS.lock().unwrap_or_else(|e| e.into_inner());
    turns.get(turn_id).map(|(bank, _)| bank.clone())
}

/// One bank per turn: a step after the opening one keeps the bank that
/// step recorded — no lookup, and a session bank switched mid-turn takes
/// effect on the next turn instead of splitting this one across banks.
/// `None`: resolve (the opening step, or a turn this process never saw).
pub(crate) fn pinned_turn_bank(turn_id: Option<&str>, step: u64) -> Option<Option<Resolved>> {
    turn_id.filter(|_| step > 0).and_then(turn_bank)
}

/// [`turn_bank`], forgetting the turn (it completed).
pub(crate) fn take_turn_bank(turn_id: &str) -> Option<Option<Resolved>> {
    let mut turns = TURN_BANKS.lock().unwrap_or_else(|e| e.into_inner());
    turns.remove(turn_id).map(|(bank, _)| bank)
}

/// Who is calling, from the invocation baggage the harness stamps on every
/// turn step: `iii.session.id`, and `iii.message.id` (the turn id). It is
/// caller-supplied and unattested, which is fine here: it only picks the
/// default bank, one the caller could name with `bank` anyway. Read it
/// inside the handler's future — outside it there is no invocation context.
#[derive(Debug, Default)]
pub(crate) struct Caller {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
}

impl Caller {
    fn from_baggage() -> Self {
        let read = |key| {
            iii_helpers::observability::get_baggage_entry(key).filter(|v: &String| !v.is_empty())
        };
        Self {
            session_id: read("iii.session.id"),
            turn_id: read("iii.message.id"),
        }
    }
}

/// The bank a `memory::*` call acts on: `bank` when given, else the calling
/// session's bank (the one its turn was injected with), else the default
/// for callers outside a session (CLI, console, other workers).
pub async fn request_bank(deps: &Deps, explicit: Option<String>) -> Result<String, MemoryError> {
    Ok(request_scope(deps, explicit).await?.bank)
}

/// [`request_bank`] with its provenance, for the reads that layer
/// `default_bank` under a directory bank like the hook (recall, preview).
pub(crate) async fn request_scope(
    deps: &Deps,
    explicit: Option<String>,
) -> Result<Resolved, MemoryError> {
    if let Some(bank) = explicit {
        return Ok(Resolved::named(bank));
    }
    let caller = Caller::from_baggage();
    let cfg = deps.config().await;
    let store = deps.store().await;
    let recorded = match (caller.session_id.as_deref(), caller.turn_id.as_deref()) {
        (Some(session_id), Some(turn_id)) => match turn_bank(turn_id) {
            Some(bank) => Some(bank),
            // Not seen by this process (a restart): the hook's stamp.
            None => {
                let tail = crate::extract::messages_tail(&deps.iii, session_id).await;
                let stamped = tail.and_then(|reply| stamped_bank(&reply, turn_id));
                if let Some(bank) = &stamped {
                    remember_turn(turn_id, Some(bank.clone()));
                }
                stamped.map(Some)
            }
        },
        _ => None,
    };
    // Deleted since the turn was injected: no memory, never a resurrection.
    let recorded = recorded
        .map(|bank| bank.filter(|b| b.bank == cfg.default_bank || !store.is_deleted(&b.bank)));
    request_bank_with(caller, &cfg.default_bank, recorded, |session| async move {
        resolve(deps, &session, None).await
    })
    .await
}

async fn request_bank_with<R, Fut>(
    caller: Caller,
    default_bank: &str,
    recorded: Option<Option<Resolved>>,
    resolve: R,
) -> Result<Resolved, MemoryError>
where
    R: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<Option<Resolved>, String>>,
{
    let Some(session_id) = caller.session_id else {
        return Ok(Resolved::named(default_bank));
    };
    let bank = match recorded {
        Some(bank) => bank,
        None => resolve(session_id)
            .await
            .map_err(MemoryError::SiblingUnavailable)?,
    };
    bank.ok_or_else(|| {
        MemoryError::InvalidInput(
            "no memory bank applies to this session (its memory_bank is invalid or was \
             deleted); pass `bank` to name one"
                .into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turns_later_steps_keep_the_bank_its_opening_step_chose() {
        remember_turn("t-pinned-1", Some(Resolved::named("blog")));
        remember_turn("t-pinned-2", None);
        assert_eq!(
            pinned_turn_bank(Some("t-pinned-1"), 3),
            Some(Some(Resolved::named("blog")))
        );
        assert_eq!(
            pinned_turn_bank(Some("t-pinned-2"), 1),
            Some(None),
            "no memory stays no memory"
        );
        assert_eq!(
            pinned_turn_bank(Some("t-pinned-1"), 0),
            None,
            "the opening step resolves"
        );
        assert_eq!(
            pinned_turn_bank(Some("t-unseen"), 2),
            None,
            "not recorded here: resolve"
        );
        assert_eq!(pinned_turn_bank(None, 2), None);
    }

    #[test]
    fn a_turn_bank_is_kept_until_taken_and_evicted_oldest_first() {
        let mut turns = HashMap::new();
        remember_in(&mut turns, "t1", Some(Resolved::named("blog")), 1);
        remember_in(&mut turns, "t2", None, 2);
        assert_eq!(
            turns.get("t1").map(|t| t.0.clone()),
            Some(Some(Resolved::named("blog")))
        );
        assert_eq!(turns.get("t2").map(|t| t.0.clone()), Some(None));

        let mut full: TurnBanks = (0..TURN_BANKS_CAP)
            .map(|i| (format!("t{i}"), (None, i as u64)))
            .collect();
        remember_in(&mut full, "t0", Some(Resolved::named("x")), 1_000_000); // refresh t0
        remember_in(&mut full, "new", None, 1_000_001);
        assert_eq!(full.len(), TURN_BANKS_CAP);
        assert!(full.contains_key("t0"));
        assert!(!full.contains_key("t1"), "least recently used goes first");
    }

    async fn call(
        caller: Caller,
        recorded: Option<Option<String>>,
        resolved: Result<Option<String>, String>,
    ) -> Result<String, MemoryError> {
        let recorded = recorded.map(|bank| bank.map(Resolved::named));
        let resolved = resolved.map(|bank| bank.map(Resolved::named));
        request_bank_with(caller, "main", recorded, |_| async move { resolved })
            .await
            .map(|r| r.bank)
    }

    fn in_session(turn: Option<&str>) -> Caller {
        Caller {
            session_id: Some("s".into()),
            turn_id: turn.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn a_call_outside_a_session_uses_the_default() {
        let bank = call(Caller::default(), None, Ok(Some("blog".into()))).await;
        assert_eq!(bank.unwrap(), "main");
    }

    #[tokio::test]
    async fn a_call_in_a_session_uses_the_turns_bank_then_the_sessions() {
        let turn = call(
            in_session(Some("t")),
            Some(Some("coding".into())),
            Ok(Some("blog".into())),
        );
        assert_eq!(
            turn.await.unwrap(),
            "coding",
            "the bank the turn was injected with"
        );
        let session = call(in_session(Some("t")), None, Ok(Some("blog".into())));
        assert_eq!(session.await.unwrap(), "blog");
    }

    #[tokio::test]
    async fn a_session_without_memory_or_a_failed_lookup_is_an_error() {
        let off = call(in_session(Some("t")), Some(None), Ok(Some("blog".into()))).await;
        assert!(matches!(off, Err(MemoryError::InvalidInput(_))), "{off:?}");
        let unusable = call(in_session(None), None, Ok(None)).await;
        assert!(
            matches!(unusable, Err(MemoryError::InvalidInput(_))),
            "{unusable:?}"
        );
        let failed = call(in_session(None), None, Err("session::get: timeout".into())).await;
        assert!(
            matches!(failed, Err(MemoryError::SiblingUnavailable(_))),
            "{failed:?}"
        );
    }

    /// A session store: id → metadata.
    fn sessions(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(id, meta)| (id.to_string(), meta.clone()))
            .collect()
    }

    async fn resolve(
        turn: Option<Value>,
        session: &str,
        store: &HashMap<String, Value>,
        deleted: &[&str],
    ) -> Result<Option<String>, String> {
        resolve_recorded(turn, session, store, deleted, &[]).await
    }

    /// A turn working in `root` (no filesystem: `w` selects `w-bank`).
    fn in_root(root: &str) -> Option<Value> {
        Some(json!({ "fs_scope": { "root": root } }))
    }

    /// [`resolve`] with turn banks the hook recorded (`turn id → bank`).
    async fn resolve_recorded(
        turn: Option<Value>,
        session: &str,
        store: &HashMap<String, Value>,
        deleted: &[&str],
        recorded: &[(&str, Option<&str>)],
    ) -> Result<Option<String>, String> {
        let recorded: Vec<(&str, Option<Resolved>)> = recorded
            .iter()
            .map(|(id, bank)| (*id, bank.map(Resolved::named)))
            .collect();
        resolve_full(turn, session, store, deleted, &recorded)
            .await
            .map(|r| r.map(|r| r.bank))
    }

    /// [`resolve_recorded`] with the provenance.
    async fn resolve_full(
        turn: Option<Value>,
        session: &str,
        store: &HashMap<String, Value>,
        deleted: &[&str],
        recorded: &[(&str, Option<Resolved>)],
    ) -> Result<Option<Resolved>, String> {
        resolve_with(
            turn.as_ref(),
            session,
            "main",
            |b| deleted.contains(&b),
            |t| {
                recorded
                    .iter()
                    .find(|(id, _)| *id == t)
                    .map(|(_, bank)| bank.clone())
            },
            |root| (root != "gone").then(|| format!("{root}-bank")),
            // An unknown session reads as `null`, like session::get.
            |id| {
                let found = store.get(&id).cloned();
                async move { Ok(found) }
            },
        )
        .await
    }

    #[test]
    fn bank_field_reads_a_non_empty_string_only() {
        assert_eq!(
            bank_field(Some(&json!({ "memory_bank": "blog" }))),
            Some("blog".into())
        );
        assert_eq!(bank_field(Some(&json!({ "memory_bank": "" }))), None);
        assert_eq!(bank_field(Some(&json!({ "memory_bank": 7 }))), None);
        assert_eq!(bank_field(None), None);
    }

    #[tokio::test]
    async fn turn_then_session_then_default() {
        let store = sessions(&[
            ("s", json!({ "memory_bank": "blog" })),
            ("bare", json!({})),
            ("empty", json!({ "memory_bank": "" })),
        ]);
        let turn = Some(json!({ "memory_bank": "coding" }));
        assert_eq!(
            resolve(turn, "s", &store, &[]).await,
            Ok(Some("coding".into()))
        );
        assert_eq!(
            resolve(None, "s", &store, &[]).await,
            Ok(Some("blog".into()))
        );
        assert_eq!(
            resolve(None, "bare", &store, &[]).await,
            Ok(Some("main".into()))
        );
        assert_eq!(
            resolve(None, "empty", &store, &[]).await,
            Ok(Some("main".into())),
            "an empty memory_bank is absent, as on the turn"
        );
    }

    #[tokio::test]
    async fn a_sub_agent_inherits_its_parents_bank_up_the_chain() {
        let store = sessions(&[
            ("root", json!({ "memory_bank": "blog" })),
            ("child", json!({ "parent_session_id": "root", "depth": 1 })),
            (
                "grandchild",
                json!({ "parent_session_id": "child", "depth": 2 }),
            ),
            (
                "own",
                json!({ "parent_session_id": "root", "memory_bank": "notes" }),
            ),
        ]);
        assert_eq!(
            resolve(None, "child", &store, &[]).await,
            Ok(Some("blog".into()))
        );
        assert_eq!(
            resolve(None, "grandchild", &store, &[]).await,
            Ok(Some("blog".into()))
        );
        assert_eq!(
            resolve(None, "own", &store, &[]).await,
            Ok(Some("notes".into())),
            "a child's own bank wins over the parent's"
        );
    }

    #[tokio::test]
    async fn a_sub_agent_follows_its_parent_turns_own_bank() {
        let store = sessions(&[
            ("root", json!({ "memory_bank": "blog" })),
            (
                "child",
                json!({ "parent_session_id": "root", "parent_turn_id": "rt" }),
            ),
        ]);
        let child = |recorded| resolve_recorded(None, "child", &store, &[], recorded);
        assert_eq!(
            child(&[("rt", Some("coding"))]).await,
            Ok(Some("coding".into())),
            "the parent turn ran on a turn-only override"
        );
        assert_eq!(child(&[("rt", None)]).await, Ok(None));
        assert_eq!(
            child(&[]).await,
            Ok(Some("blog".into())),
            "not recorded here: the parent session's bank"
        );
    }

    #[tokio::test]
    async fn a_missing_parent_or_a_cycle_is_no_memory_never_the_default() {
        let store = sessions(&[
            ("a", json!({ "parent_session_id": "b" })),
            ("b", json!({ "parent_session_id": "a" })),
            ("orphan", json!({ "parent_session_id": "deleted" })),
        ]);
        assert_eq!(resolve(None, "a", &store, &[]).await, Ok(None));
        assert_eq!(resolve(None, "orphan", &store, &[]).await, Ok(None));
        assert_eq!(resolve(None, "nobody", &store, &[]).await, Ok(None));
    }

    #[tokio::test]
    async fn an_invalid_or_deleted_bank_is_no_memory_never_another_bank() {
        let store = sessions(&[
            ("bad", json!({ "memory_bank": "My Blog" })),
            ("gone", json!({ "memory_bank": "blog" })),
            ("child", json!({ "parent_session_id": "gone" })),
            ("bare", json!({})),
        ]);
        assert_eq!(resolve(None, "bad", &store, &[]).await, Ok(None));
        let turn = Some(json!({ "memory_bank": "Nope!" }));
        assert_eq!(resolve(turn, "bare", &store, &[]).await, Ok(None));
        assert_eq!(resolve(None, "gone", &store, &["blog"]).await, Ok(None));
        assert_eq!(resolve(None, "child", &store, &["blog"]).await, Ok(None));
        assert_eq!(
            resolve(None, "bare", &store, &["main"]).await,
            Ok(Some("main".into())),
            "the default bank always materializes, deleted or not"
        );
    }

    #[tokio::test]
    async fn only_a_bank_the_directory_chose_reads_the_default_under_it() {
        let store = sessions(&[
            ("bare", json!({})),
            ("blog", json!({ "memory_bank": "blog" })),
            ("root", json!({})),
            (
                "child",
                json!({ "parent_session_id": "root", "parent_turn_id": "rt" }),
            ),
        ]);
        let layer = |r: Result<Option<Resolved>, String>| {
            r.unwrap().and_then(|r| r.layer("main").map(str::to_string))
        };
        let derived = resolve_full(in_root("w"), "bare", &store, &[], &[]).await;
        assert_eq!(
            derived,
            Ok(Some(Resolved {
                bank: "w-bank".into(),
                derived: true,
                agent: None,
            }))
        );
        assert_eq!(layer(derived), Some("main".into()));
        let mut named = in_root("w").unwrap();
        named["memory_bank"] = json!("coding");
        assert_eq!(
            layer(resolve_full(Some(named), "bare", &store, &[], &[]).await),
            None,
            "a turn naming its bank reads it alone"
        );
        assert_eq!(
            layer(resolve_full(in_root("w"), "blog", &store, &[], &[]).await),
            None,
            "so does a session naming one"
        );
        assert_eq!(
            layer(resolve_full(in_root("gone"), "bare", &store, &[], &[]).await),
            None,
            "the default itself: nothing under it"
        );
        let directory = Resolved {
            bank: "main".into(),
            derived: true,
            agent: None,
        };
        assert_eq!(directory.layer("main"), None, "never the default twice");

        // A sub-agent inherits its parent turn's bank with its layer.
        let parent = Resolved {
            bank: "w-bank".into(),
            derived: true,
            agent: None,
        };
        let child = resolve_full(None, "child", &store, &[], &[("rt", Some(parent))]).await;
        assert_eq!(layer(child), Some("main".into()));
        let unseen = resolve_full(in_root("w"), "child", &store, &[], &[]).await;
        assert_eq!(
            layer(unseen),
            Some("main".into()),
            "not recorded here: up the chain to its own directory's"
        );
    }

    #[tokio::test]
    async fn a_profile_reads_its_own_agent_bank_under_any_bank_no_metadata_named() {
        let reviewer = json!({ "id": "reviewer", "name": "Code Reviewer" });
        let store = sessions(&[
            ("solo", json!({ "agent_profile": reviewer })),
            ("bare", json!({ "agent_profile": "reviewer" })),
            ("plain", json!({})),
            ("general", json!({ "agent_profile": { "id": "default" } })),
            (
                "named",
                json!({ "agent_profile": reviewer, "memory_bank": "blog" }),
            ),
            ("bad", json!({ "agent_profile": { "id": "Code Reviewer" } })),
            ("lead", json!({ "agent_profile": { "id": "tech-lead" } })),
            (
                "child",
                json!({ "parent_session_id": "lead", "parent_turn_id": "lt", "agent_profile": reviewer }),
            ),
            (
                "anon",
                json!({ "parent_session_id": "lead", "parent_turn_id": "lt" }),
            ),
            ("blogger", json!({ "memory_bank": "blog" })),
            (
                "blog-child",
                json!({ "parent_session_id": "blogger", "agent_profile": reviewer }),
            ),
        ]);
        let layers = |r: Result<Option<Resolved>, String>| r.unwrap().unwrap().layers("main");
        let at = |turn, session, deleted: &'static [&'static str]| {
            resolve_full(turn, session, &store, deleted, &[])
        };
        assert_eq!(layers(at(None, "solo", &[]).await), ["agent-reviewer"]);
        assert_eq!(
            layers(at(None, "bare", &[]).await),
            ["agent-reviewer"],
            "a bare id"
        );
        assert_eq!(
            layers(at(in_root("w"), "solo", &[]).await),
            ["main", "agent-reviewer"],
            "the default's rules first, then the agent's"
        );
        assert!(layers(at(None, "plain", &[]).await).is_empty());
        assert!(
            layers(at(None, "general", &[]).await).is_empty(),
            "the bundled default identity is no profile"
        );
        assert!(
            layers(at(in_root("w"), "named", &[]).await).is_empty(),
            "a session naming its bank reads it alone"
        );
        let mut turn = in_root("w").unwrap();
        turn["memory_bank"] = json!("coding");
        assert!(
            layers(at(Some(turn), "solo", &[]).await).is_empty(),
            "so does a turn naming one"
        );
        let unusable = at(None, "bad", &[]).await.unwrap().unwrap();
        assert_eq!(
            (unusable.bank.as_str(), unusable.agent),
            ("main", None),
            "an id no bank can carry drops the layer, not the memory"
        );
        let gone = at(None, "solo", &["agent-reviewer"])
            .await
            .unwrap()
            .unwrap();
        assert_eq!((gone.bank.as_str(), gone.agent), ("main", None));

        // A sub-agent reads its own profile's bank, never its parent's.
        let parent = Resolved {
            bank: "w-bank".into(),
            derived: true,
            agent: Some("tech-lead".into()),
        };
        let recorded = [("lt", Some(parent))];
        let child = resolve_full(None, "child", &store, &[], &recorded).await;
        assert_eq!(layers(child), ["main", "agent-reviewer"]);
        let anon = resolve_full(None, "anon", &store, &[], &recorded).await;
        assert_eq!(layers(anon), ["main"]);
        assert!(
            layers(at(None, "blog-child", &[]).await).is_empty(),
            "a parent's named bank is read alone by its sub-agents too"
        );
    }

    #[test]
    fn a_layer_reads_only_memories_about_the_user() {
        let memory = |entities: &[&str]| -> Memory {
            serde_json::from_value(json!({
                "id": "m", "text": "t", "entities": entities, "confidence": "stated",
                "created_at": 1, "updated_at": 1,
            }))
            .unwrap()
        };
        let about = |entities: &[&str]| in_layer(&memory(entities));
        assert!(about(&["user"]));
        assert!(about(&["pnpm", "user"]));
        assert!(!about(&["ledger-sync"]), "another repository's fact");
        assert!(!about(&[]));
        assert!(!about(&["users"]));
        let role = memory(&["review"]);
        assert!(!layer_keeps("main", "main", &role));
        assert!(
            layer_keeps("agent-reviewer", "main", &role),
            "an agent bank is read whole"
        );
    }

    #[test]
    fn a_turn_the_process_never_saw_reads_its_bank_from_its_stamp() {
        let page = json!({ "messages": [
            { "entry_id": "a0", "message": { "role": "assistant" },
              "origin": { "turn_id": "t0", "memory_bank": "old-000000" } },
            { "entry_id": "a1", "message": { "role": "assistant" },
              "origin": { "turn_id": "t1", "memory_bank": "w-abc123", "memory_layer": "main" } },
            { "entry_id": "n1", "message": { "role": "user" }, "origin": { "turn_id": "t1" } },
            { "entry_id": "a2", "message": { "role": "assistant" },
              "origin": { "turn_id": "t2", "memory_bank": "blog" } },
            { "entry_id": "a3", "message": { "role": "assistant" },
              "origin": { "turn_id": "t3", "memory_bank": "Not A Bank" } },
            { "entry_id": "a4", "message": { "role": "assistant" },
              "origin": { "turn_id": "t4", "memory_bank": "main", "memory_agent": "agent-reviewer" } },
        ]});
        assert_eq!(
            stamped_bank(&page, "t1"),
            Some(Resolved {
                bank: "w-abc123".into(),
                derived: true,
                agent: None,
            }),
            "a notice after the reply carries no stamp: the reply's counts"
        );
        assert_eq!(stamped_bank(&page, "t2"), Some(Resolved::named("blog")));
        assert_eq!(
            stamped_bank(&page, "t4").and_then(|r| r.agent),
            Some("reviewer".into()),
            "the agent layer survives a restart"
        );
        assert_eq!(stamped_bank(&page, "t3"), None, "not a valid bank");
        assert_eq!(stamped_bank(&page, "t9"), None);
        assert_eq!(stamped_bank(&json!({}), "t1"), None);
    }

    #[tokio::test]
    async fn a_failed_lookup_is_an_error_not_the_default() {
        let failing = |turn: Option<Value>| async move {
            resolve_with(
                turn.as_ref(),
                "s",
                "main",
                |_| false,
                |_| None,
                |_| None,
                |id| async move { Err::<Option<Value>, _>(format!("session::get {id}: timeout")) },
            )
            .await
        };
        assert!(failing(None).await.is_err());
        // A turn override needs no lookup.
        let turn = Some(json!({ "memory_bank": "blog" }));
        assert_eq!(failing(turn).await, Ok(Some(Resolved::named("blog"))));
    }

    #[tokio::test]
    async fn a_session_naming_no_bank_works_in_its_directorys() {
        let store = sessions(&[
            ("bare", json!({})),
            ("blog", json!({ "memory_bank": "blog" })),
            ("root", json!({ "memory_bank": "notes" })),
            ("child", json!({ "parent_session_id": "root" })),
        ]);
        assert_eq!(
            resolve(in_root("w"), "bare", &store, &[]).await,
            Ok(Some("w-bank".into()))
        );
        assert_eq!(
            resolve(in_root("gone"), "bare", &store, &[]).await,
            Ok(Some("main".into())),
            "a root that selects nothing: the default"
        );
        let mut turn = in_root("w").unwrap();
        turn["memory_bank"] = json!("coding");
        assert_eq!(
            resolve(Some(turn), "bare", &store, &[]).await,
            Ok(Some("coding".into()))
        );
        assert_eq!(
            resolve(in_root("w"), "blog", &store, &[]).await,
            Ok(Some("blog".into())),
            "the session's bank wins"
        );
        assert_eq!(
            resolve(in_root("w"), "child", &store, &[]).await,
            Ok(Some("notes".into())),
            "a sub-agent stays in its parent's bank"
        );
        assert_eq!(
            resolve(in_root("w"), "bare", &store, &["w-bank"]).await,
            Ok(None),
            "a deleted directory bank is no memory, not the default"
        );
    }

    fn git_init(dir: &Path) {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    }

    #[test]
    fn a_repo_and_its_worktrees_share_one_bank_per_path() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("a/workers");
        git_init(&repo);
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        let bank = root_bank(&repo).unwrap();
        assert!(bank.starts_with("workers-"), "{bank}");
        assert_eq!(root_bank(&repo.join("src/deep")), Some(bank.clone()));

        // A linked worktree elsewhere: `.git` file → its git dir → commondir.
        let wt_git = repo.join(".git/worktrees/feat");
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::write(wt_git.join("HEAD"), "ref: refs/heads/feat\n").unwrap();
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        let wt = tmp.path().join("elsewhere/feat");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
        assert_eq!(root_bank(&wt), Some(bank.clone()), "same repo, same bank");

        let other = tmp.path().join("b/workers");
        git_init(&other);
        let other = root_bank(&other).unwrap();
        assert!(other.starts_with("workers-") && other != bank, "{other}");

        let plain = tmp.path().join("Notes Dir");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(root_bank(&plain).unwrap().starts_with("notes-dir-"));
        assert_eq!(root_bank(&tmp.path().join("missing")), None);
        assert_eq!(
            root_bank(&repo.join("missing")),
            None,
            "a missing path inside a repository selects nothing"
        );
    }

    #[test]
    fn a_directory_bank_is_always_a_valid_name() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["My.Repo", "--__--", "ünï", &"x".repeat(100), "a_b-c"] {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let bank = root_bank(&dir).unwrap();
            assert!(validate_name(&bank).is_ok(), "{name:?} → {bank:?}");
            assert!(directory_shaped(&bank), "{name:?} → {bank:?}");
        }
    }

    #[test]
    fn only_a_slug_and_six_lowercase_hex_look_like_a_directorys_bank() {
        assert!(directory_shaped("workers-7c6923"));
        for bank in [
            "main",
            "blog",
            "blog-old",
            "notes-2024",
            "x-ABC123",
            "-abc123",
            "abc123",
            "w-abc12",
        ] {
            assert!(!directory_shaped(bank), "{bank}");
        }
    }
}
