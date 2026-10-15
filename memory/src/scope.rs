//! Which bank a turn, its extraction pass, and its agent's `memory::*`
//! calls use: one resolver for all three, so a session reads and writes
//! the same bank. Hindsight derives one bank id for inject, capture and its
//! tools alike (`hindsight-integrations/claude-code/scripts/lib/bank.py:93`).
//!
//! Order: turn metadata `memory_bank` → session metadata `memory_bank` →
//! the parent turn's bank, else the parent session's, walked up (a sub-agent
//! works in its parent's bank) → `default_bank`. `""` is absent. An invalid
//! name, a deleted bank other than the default, or a parent that no longer
//! exists means NO memory — never a fallback to another bank. A failed
//! session lookup is an error: the hook injects nothing new, extraction
//! retries, a call fails.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{LazyLock, Mutex};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::store::validate_name;
use crate::types::now_ms;

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

/// The resolver over injected lookups, so tests need no engine: `recorded`
/// is the bank a turn was injected with ([`turn_bank`]), `lookup` a
/// session's metadata object (`None`: no such session).
pub(crate) async fn resolve_with<L, Fut>(
    turn_metadata: Option<&Value>,
    session_id: &str,
    default_bank: &str,
    deleted: impl Fn(&str) -> bool,
    recorded: impl Fn(&str) -> Option<Option<String>>,
    lookup: L,
) -> Result<Option<String>, String>
where
    L: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Option<Value>, String>>,
{
    let chosen = match bank_field(turn_metadata) {
        Some(bank) => bank,
        None => match walk(session_id, default_bank, recorded, lookup).await? {
            Some(bank) => bank,
            None => return Ok(None),
        },
    };
    if validate_name(&chosen).is_err() {
        tracing::warn!(session_id, bank = %chosen, "invalid memory_bank; no memory for this session");
        return Ok(None);
    }
    // The default materializes on first use, deleted or not (README).
    if chosen != default_bank && deleted(&chosen) {
        return Ok(None);
    }
    Ok(Some(chosen))
}

/// The session's own bank, else a sub-agent's parent turn's (the bank that
/// turn was injected with, an override included) or the parent session's,
/// walked up; the default at the top. `None` — no memory — for a session
/// that no longer exists, or a chain that cycles or runs past
/// [`MAX_PARENT_DEPTH`]: never the default in their place.
async fn walk<L, Fut>(
    session_id: &str,
    default_bank: &str,
    recorded: impl Fn(&str) -> Option<Option<String>>,
    mut lookup: L,
) -> Result<Option<String>, String>
where
    L: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Option<Value>, String>>,
{
    let mut id = session_id.to_string();
    for _ in 0..=MAX_PARENT_DEPTH {
        let Some(meta) = lookup(id.clone()).await? else {
            return Ok(None);
        };
        if let Some(bank) = bank_field(Some(&meta)) {
            return Ok(Some(bank));
        }
        let field = |key| {
            meta.get(key)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
        };
        if let Some(bank) = field("parent_turn_id").and_then(&recorded) {
            return Ok(bank);
        }
        match field("parent_session_id") {
            Some(parent) => id = parent.to_string(),
            None => return Ok(Some(default_bank.to_string())),
        }
    }
    Ok(None)
}

/// [`resolve_with`] against the live session store.
pub async fn resolve(
    deps: &Deps,
    session_id: &str,
    turn_metadata: Option<&Value>,
) -> Result<Option<String>, String> {
    let cfg = deps.config().await;
    let store = deps.store().await;
    resolve_with(
        turn_metadata,
        session_id,
        &cfg.default_bank,
        |bank| store.is_deleted(bank),
        turn_bank,
        |id| session_metadata(&deps.iii, id),
    )
    .await
}

/// One session's metadata object (`Null` when it has none); `None` when
/// the session does not exist (`session::get` replies `null`).
async fn session_metadata(iii: &IIIClient, session_id: String) -> Result<Option<Value>, String> {
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
    Ok(Some(
        reply
            .pointer("/meta/metadata")
            .cloned()
            .unwrap_or(Value::Null),
    ))
}

/// Recent turns tracked in [`TURN_BANKS`]; the least recently used goes
/// first past this.
const TURN_BANKS_CAP: usize = 4_096;

/// Turn id → (its bank, last used ms).
type TurnBanks = HashMap<String, (Option<String>, u64)>;

/// The bank each recent turn was injected with (`None`: no memory), so the
/// turn's extraction job and its agent's calls use exactly that bank, a
/// turn-only `memory_bank` override included.
// ponytail: in-process — after a restart the job and the calls re-resolve
// from session metadata, which only misses a turn-only override.
static TURN_BANKS: LazyLock<Mutex<TurnBanks>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn remember_in(turns: &mut TurnBanks, turn_id: &str, bank: Option<String>, now_ms: u64) {
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
pub(crate) fn remember_turn(turn_id: &str, bank: Option<String>) {
    if turn_id.is_empty() {
        return;
    }
    let mut turns = TURN_BANKS.lock().unwrap_or_else(|e| e.into_inner());
    remember_in(&mut turns, turn_id, bank, now_ms());
}

/// The bank `turn_id` was injected with, when this process saw it.
pub(crate) fn turn_bank(turn_id: &str) -> Option<Option<String>> {
    let turns = TURN_BANKS.lock().unwrap_or_else(|e| e.into_inner());
    turns.get(turn_id).map(|(bank, _)| bank.clone())
}

/// [`turn_bank`], forgetting the turn (it completed).
pub(crate) fn take_turn_bank(turn_id: &str) -> Option<Option<String>> {
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
    if let Some(bank) = explicit {
        return Ok(bank);
    }
    let caller = Caller::from_baggage();
    let recorded = caller.turn_id.as_deref().and_then(turn_bank);
    let cfg = deps.config().await;
    request_bank_with(caller, &cfg.default_bank, recorded, |session| async move {
        resolve(deps, &session, None).await
    })
    .await
}

async fn request_bank_with<R, Fut>(
    caller: Caller,
    default_bank: &str,
    recorded: Option<Option<String>>,
    resolve: R,
) -> Result<String, MemoryError>
where
    R: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<Option<String>, String>>,
{
    let Some(session_id) = caller.session_id else {
        return Ok(default_bank.to_string());
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
    fn a_turn_bank_is_kept_until_taken_and_evicted_oldest_first() {
        let mut turns = HashMap::new();
        remember_in(&mut turns, "t1", Some("blog".into()), 1);
        remember_in(&mut turns, "t2", None, 2);
        assert_eq!(
            turns.get("t1").map(|t| t.0.clone()),
            Some(Some("blog".into()))
        );
        assert_eq!(turns.get("t2").map(|t| t.0.clone()), Some(None));

        let mut full: TurnBanks = (0..TURN_BANKS_CAP)
            .map(|i| (format!("t{i}"), (None, i as u64)))
            .collect();
        remember_in(&mut full, "t0", Some("x".into()), 1_000_000); // refresh t0
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
        request_bank_with(caller, "main", recorded, |_| async move { resolved }).await
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

    /// [`resolve`] with turn banks the hook recorded (`turn id → bank`).
    async fn resolve_recorded(
        turn: Option<Value>,
        session: &str,
        store: &HashMap<String, Value>,
        deleted: &[&str],
        recorded: &[(&str, Option<&str>)],
    ) -> Result<Option<String>, String> {
        resolve_with(
            turn.as_ref(),
            session,
            "main",
            |b| deleted.contains(&b),
            |t| {
                recorded
                    .iter()
                    .find(|(id, _)| *id == t)
                    .map(|(_, bank)| bank.map(str::to_string))
            },
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
    async fn a_failed_lookup_is_an_error_not_the_default() {
        let failing = |turn: Option<Value>| async move {
            resolve_with(
                turn.as_ref(),
                "s",
                "main",
                |_| false,
                |_| None,
                |id| async move { Err::<Option<Value>, _>(format!("session::get {id}: timeout")) },
            )
            .await
        };
        assert!(failing(None).await.is_err());
        // A turn override needs no lookup.
        let turn = Some(json!({ "memory_bank": "blog" }));
        assert_eq!(failing(turn).await, Ok(Some("blog".into())));
    }
}
