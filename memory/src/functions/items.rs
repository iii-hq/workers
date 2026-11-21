//! `memory::save/get/list/update/delete/pin` — memory CRUD. Every mutation
//! goes through the bank's commit choke point and emits an item event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::events::ItemEvent;
use crate::types::{Confidence, Memory, Provenance, fingerprint, now_ms, stated_now_ms};

/// Normalized entity handles: trimmed, lowercased, empty dropped, capped.
fn normalize_entities(entities: Vec<String>) -> Vec<String> {
    entities
        .into_iter()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .take(8)
        .collect()
}

/// The text bound, after redaction; a note saved or revised from a turn is short.
fn check_text(text: &str, in_turn: bool) -> Result<(), MemoryError> {
    // A note counts characters, so a non-English one is not cut short; the
    // bound outside a turn keeps counting bytes, as it always has.
    let (len, max) = if in_turn {
        (text.chars().count(), 500)
    } else {
        (text.len(), 2_000)
    };
    if len < 3 || len > max {
        return Err(MemoryError::InvalidInput(format!(
            "text must be 3..{max} characters"
        )));
    }
    Ok(())
}

/// From a turn, `label` may not be added: the `user` entity puts a memory in
/// every repository's layer, the `demoted-rule` tag changes what promotion
/// may pick. None can be removed: a turn edits only notes that hold neither
/// ([`a_turns_note`]).
fn refuse_label(labels: &[String], label: &str, kind: &str) -> Result<(), MemoryError> {
    if labels.iter().any(|l| l.trim().to_lowercase() == label) {
        return Err(crate::scope::not_from_a_turn(format_args!(
            "adding the `{label}` {kind}"
        )));
    }
    Ok(())
}

/// A memory a turn may revise or bring back: a note saved from a turn, read
/// only in its bank (not pinned, not about the user, not a demoted rule line).
fn a_turns_note(memory: &Memory) -> bool {
    let no = |labels: &[String], label: &str| !labels.iter().any(|l| l == label);
    memory.is_agent_note()
        && !memory.pinned
        && no(&memory.entities, crate::scope::LAYER_ENTITY)
        && no(&memory.tags, crate::learned::DEMOTED_TAG)
}

/// Starts the error of a write on a retired record.
const MEMORY_RETIRED: &str = "memory_retired";

/// The live record at the end of `memory`'s `superseded_by` chain, if any.
async fn live_successor(bank: &crate::store::Bank, memory: &Memory) -> Option<Memory> {
    let mut next = memory.superseded_by.clone();
    // One hop per correction; the bound only stops a cycle.
    for _ in 0..8 {
        let m = bank.get(&next?).await?;
        if m.is_live() {
            return Some(m);
        }
        next = m.superseded_by;
    }
    None
}

/// What a caller refused on a retired record does with the live one instead.
fn act_on_live(in_turn: bool) -> &'static str {
    if in_turn {
        "delete that memory or save a new note instead"
    } else {
        "update or delete that memory instead"
    }
}

/// The refusal of a turn writing a pinned memory.
fn pinned_from_a_turn(id: &str) -> MemoryError {
    crate::scope::not_from_a_turn(format_args!(
        "changing or deleting the pinned memory `{id}`"
    ))
}

/// Inputs for one save transition (grouped: clippy arity cap, and the
/// call sites read better named).
#[derive(Clone)]
struct SaveInput {
    id: String,
    text: String,
    entities: Vec<String>,
    tags: Vec<String>,
    pinned: bool,
    source: Option<Provenance>,
    now: u64,
    /// A re-observation adds `entities` too, not only tags.
    merge_entities: bool,
    /// The user's own restatement (a save from outside a turn): only that
    /// adds corroboration.
    corroborates: bool,
}

/// The save state transition, pure so its invariants are testable:
/// - same fingerprint + live: reinforce (corroboration + 1 when the input
///   corroborates, revision + 1);
/// - same fingerprint + retired: RESURRECT — clear the tombstone and
///   CONTINUE the revision counter (a fresh revision-0 record would lose
///   to the tombstone under last-wins replay on the next boot);
/// - unknown fingerprint: a fresh record.
fn save_transition(existing: Option<Memory>, input: SaveInput) -> (Memory, ItemEvent, bool) {
    let SaveInput {
        id,
        text,
        entities,
        tags,
        pinned,
        source,
        now,
        merge_entities,
        corroborates,
    } = input;
    match existing {
        Some(existing) => {
            let live = existing.is_live();
            let mut f = existing;
            if !live {
                f.invalid_at = None;
                f.superseded_by = None;
            }
            // A re-observation can add labels; it never removes them.
            for tag in normalize_entities(tags) {
                if !f.tags.contains(&tag) && f.tags.len() < 8 {
                    f.tags.push(tag);
                }
            }
            if merge_entities {
                for entity in normalize_entities(entities) {
                    if !f.entities.contains(&entity) && f.entities.len() < 8 {
                        f.entities.push(entity);
                    }
                }
            }
            if corroborates {
                f.corroboration = f.corroboration.saturating_add(1);
            }
            f.pinned = f.pinned || pinned;
            f.updated_at = now;
            f.stated_at = Some(now);
            f.revision += 1;
            if live {
                (f, ItemEvent::Updated, false)
            } else {
                (f, ItemEvent::Created, true)
            }
        }
        None => (
            Memory {
                id,
                text,
                entities: normalize_entities(entities),
                tags: normalize_entities(tags),
                confidence: Confidence::Stated,
                corroboration: 0,
                pinned,
                source,
                created_at: now,
                updated_at: now,
                invalid_at: None,
                superseded_by: None,
                revision: 0,
                stated_at: Some(now),
            },
            ItemEvent::Created,
            true,
        ),
    }
}

/// A save into the default bank that names no entities is the user's own
/// (the memory page's add field, the CLI): about the user, so it is read
/// under every directory bank too. A save from a turn is the agent's note,
/// never tagged as the user's.
fn about_the_user(entities: &[String], bank: &str, default_bank: &str, in_turn: bool) -> bool {
    !in_turn && entities.iter().all(|e| e.trim().is_empty()) && bank == default_bank
}

/// Where a save came from: the turn it was called from (invocation
/// baggage: its session and turn id), else the session the request names.
fn save_source(named: Option<String>, caller: crate::scope::Caller) -> Option<Provenance> {
    let (session_id, turn_id) = match caller.session_id {
        Some(session_id) => (session_id, caller.turn_id),
        None => (named?, None),
    };
    Some(Provenance {
        session_id: Some(session_id),
        turn_id,
        ..Provenance::default()
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveRequest {
    /// Target bank; omitted: the calling session's bank, else the configured
    /// default. From a turn only the turn's own bank.
    #[serde(default)]
    pub bank: Option<String>,
    /// The memory, one self-contained sentence.
    pub text: String,
    /// Entity handles (people/projects/tools) used as a retrieval signal.
    /// None into the default bank from outside a turn (the memory page, the
    /// CLI): `user`, read under every repository's bank. From a turn `user`
    /// is refused.
    #[serde(default)]
    pub entities: Vec<String>,
    /// Topical labels for filtering within the bank (organization, not
    /// ranking).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Refused from a turn.
    #[serde(default)]
    pub pinned: bool,
    /// Provenance session for a save made outside a turn on behalf of a
    /// conversation. A save from a turn records that turn's session and turn
    /// id instead.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SaveResponse {
    pub memory: Memory,
    /// False when the text fingerprint matched an existing memory (it was
    /// reinforced instead, or, from a turn, left as it was).
    pub created: bool,
}

pub async fn save(deps: &Deps, req: SaveRequest) -> Result<SaveResponse, MemoryError> {
    save_in(deps, req, false).await
}

/// [`save`]; `demoting`: a `learned` line moving back to memories. A bank
/// trashed since is not brought back (`BankNotFound`), and a memory that
/// already holds the line gains the request's entities, so it is read
/// wherever the line was.
pub(crate) async fn save_in(
    deps: &Deps,
    req: SaveRequest,
    demoting: bool,
) -> Result<SaveResponse, MemoryError> {
    // A demotion is the worker's own write, whatever baggage its job carries.
    let caller = if demoting {
        crate::scope::Caller::default()
    } else {
        crate::scope::Caller::from_baggage()
    };
    let in_turn = caller.in_turn();
    let text = req.text.trim();
    crate::redact::reject_injection("text", text)?;
    // Redact before the fingerprint: dedup and the stored text agree, and
    // the caller sees what was kept in the returned record. Bounded after:
    // stripping invisible characters can shorten the text or empty it.
    let redaction = crate::redact::redact(text);
    let text = redaction.text.trim().to_string();
    check_text(&text, in_turn)?;
    // Short fields too, before normalization lowercases them (a lowercased
    // key still leaks its material, and the key shapes are case-sensitive).
    let (mut entities, mut tags) = (req.entities, req.tags);
    let mut labels = redaction.labels;
    labels.extend(crate::redact::drop_secrets(&mut entities));
    labels.extend(crate::redact::drop_secrets(&mut tags));
    // From a turn: a note, read only where the turn's own memories are.
    let turn = if in_turn {
        let Some(turn_id) = caller.turn_id.clone() else {
            return Err(MemoryError::Forbidden(
                "a save from inside a turn needs its turn id (`iii.message.id`)".into(),
            ));
        };
        if req.pinned {
            return Err(crate::scope::not_from_a_turn("saving a pinned memory"));
        }
        refuse_label(&entities, crate::scope::LAYER_ENTITY, "entity")?;
        refuse_label(&tags, crate::learned::DEMOTED_TAG, "tag")?;
        Some((turn_id, deps.config().await.max_memories_per_turn))
    } else {
        None
    };
    let (bank_name, _) = crate::scope::write_bank(deps, &caller, req.bank.clone(), false).await?;
    let default_bank = deps.config().await.default_bank.clone();
    if about_the_user(&entities, &bank_name, &default_bank, in_turn) {
        crate::scope::into_layer(&mut entities);
    }
    let store = deps.store().await;
    // A save from a turn never brings back a bank trashed since `write_bank`.
    let (bank, bank_created) = if demoting || in_turn {
        store
            .ensure_extraction_bank(&bank_name, &default_bank)
            .await?
    } else {
        store.ensure_bank(&bank_name, None).await?
    };
    if bank_created {
        deps.emitter.bank("created", &bank_name).await;
    }

    let input = SaveInput {
        id: fingerprint(&text),
        text,
        entities,
        tags,
        pinned: req.pinned,
        source: save_source(req.session_id, caller),
        now: stated_now_ms(),
        merge_entities: demoting,
        corroborates: !in_turn,
    };
    // The same text saved twice at once: the commit that lands second
    // re-reads and corroborates the first instead of failing.
    let mut tries = 0;
    let (memory, event, created) = loop {
        let existing = bank.get(&input.id).await;
        // A text a newer memory replaced stays retired: never both lines live.
        if let Some(old) = existing.as_ref().filter(|m| !m.is_live())
            && let Some(live) = live_successor(&bank, old).await
        {
            return Err(MemoryError::InvalidInput(format!(
                "{MEMORY_RETIRED}: this text was superseded by memory `{}`; {}",
                live.id,
                act_on_live(in_turn)
            )));
        }
        if let Some((turn_id, cap)) = &turn {
            turn_may_save(&bank, existing.as_ref(), turn_id, *cap).await?;
            // A turn's echo of a line it may not revise is not the user saying it again.
            if let Some(live) = existing
                .as_ref()
                .filter(|m| m.is_live() && !a_turns_note(m))
            {
                return Ok(SaveResponse {
                    memory: live.clone(),
                    created: false,
                });
            }
        }
        let mut next = save_transition(existing.clone(), input.clone());
        if turn.is_some() && next.2 {
            // Brought back from a turn: this turn's note now, under its cap.
            next.0.source = input.source.clone();
        }
        if bank
            .commit_expected(next.0.clone(), existing.as_ref())
            .await?
            .is_some()
        {
            break next;
        }
        tries += 1;
        if tries == COMMIT_TRIES {
            return Err(still_changing(&input.id));
        }
    };
    let reason = crate::redact::reason(&labels);
    deps.emitter
        .item_with(event, &bank_name, &memory, reason.as_deref())
        .await;
    crate::embed_client::spawn_backfill(deps, &bank_name);
    Ok(SaveResponse { memory, created })
}

/// Whether a save from `turn_id` may write over `existing`: a live memory is
/// only restated (no corroboration; its tags may grow when it is a turn's
/// note, `user` and `demoted-rule` were refused on the request), a new line
/// counts against the turn's `cap`, and a retired one comes back only when
/// it is a note a turn may revise.
async fn turn_may_save(
    bank: &crate::store::Bank,
    existing: Option<&Memory>,
    turn_id: &str,
    cap: usize,
) -> Result<(), MemoryError> {
    if existing.is_some_and(Memory::is_live) {
        return Ok(());
    }
    if let Some(old) = existing.filter(|old| !a_turns_note(old)) {
        return Err(crate::scope::not_from_a_turn(format_args!(
            "bringing back the retired memory `{}`",
            old.id
        )));
    }
    // shortcut: two saves racing at the cap can both land; count under the bank lock if a turn ever saves in parallel.
    if bank.saved_in_turn(turn_id).await >= cap {
        return Err(MemoryError::Forbidden(format!(
            "this turn already saved {cap} memories, the `max_memories_per_turn` cap"
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetResponse {
    pub memory: Memory,
}

pub async fn get(deps: &Deps, req: GetRequest) -> Result<GetResponse, MemoryError> {
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let memory = bank
        .get(&req.id)
        .await
        .ok_or_else(|| MemoryError::MemoryNotFound(req.id.clone()))?;
    Ok(GetResponse { memory })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    /// Page size, default 50, max 500.
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    /// Include superseded/tombstoned records (the history view).
    #[serde(default)]
    pub include_superseded: Option<bool>,
    /// Only memories carrying this tag.
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub memories: Vec<Memory>,
    pub total: usize,
}

pub async fn list(deps: &Deps, req: ListRequest) -> Result<ListResponse, MemoryError> {
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let limit = req.limit.unwrap_or(50).min(500);
    let (memories, total) = bank
        .list(
            limit,
            req.offset.unwrap_or(0),
            req.include_superseded.unwrap_or(false),
            req.tag.as_deref(),
        )
        .await;
    Ok(ListResponse { memories, total })
}

/// Shared response for update/delete/pin.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MemoryResponse {
    pub memory: Memory,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UpdateRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub entities: Option<Vec<String>>,
    /// Replace the tag set (empty list clears).
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub pinned: Option<bool>,
}

pub async fn update(deps: &Deps, req: UpdateRequest) -> Result<MemoryResponse, MemoryError> {
    // A write that lands between the read and the commit re-reads.
    for _ in 0..COMMIT_TRIES {
        if let Some(updated) = update_once(deps, req.clone()).await? {
            return Ok(updated);
        }
    }
    Err(still_changing(&req.id))
}

async fn update_once(
    deps: &Deps,
    req: UpdateRequest,
) -> Result<Option<MemoryResponse>, MemoryError> {
    let caller = crate::scope::Caller::from_baggage();
    let in_turn = caller.in_turn();
    if in_turn && req.pinned.is_some() {
        return Err(crate::scope::not_from_a_turn(
            "pinning or unpinning a memory",
        ));
    }
    let (bank_name, _) = crate::scope::write_bank(deps, &caller, req.bank.clone(), false).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let mut memory = bank
        .get(&req.id)
        .await
        .ok_or_else(|| MemoryError::MemoryNotFound(req.id.clone()))?;
    // An edit of a retired record would never reach recall; a record retired
    // after this read fails the commit, and the retry lands here.
    if !memory.is_live() {
        return Err(MemoryError::InvalidInput(
            match live_successor(&bank, &memory).await {
                Some(live) => format!(
                    "{MEMORY_RETIRED}: memory `{}` was superseded by `{}`; {}",
                    memory.id,
                    live.id,
                    act_on_live(in_turn)
                ),
                None => format!(
                    "{MEMORY_RETIRED}: memory `{}` is retired and no live memory replaced it; \
                     save its text again to bring it back",
                    memory.id
                ),
            },
        ));
    }
    if in_turn && memory.pinned {
        return Err(pinned_from_a_turn(&memory.id));
    }
    if in_turn && !a_turns_note(&memory) {
        return Err(MemoryError::Forbidden(format!(
            "memory `{}` is not a note saved from a turn, or it is read beyond this bank; \
             from inside a turn, delete it and save the corrected text instead",
            memory.id
        )));
    }
    let expected = memory.clone();
    let mut text_changed = false;
    let mut labels: Vec<&'static str> = Vec::new();
    if let Some(text) = req.text {
        crate::redact::reject_injection("text", &text)?;
        // Bounded after redaction, as in `save`.
        let redaction = crate::redact::redact(text.trim());
        let text = redaction.text.trim().to_string();
        check_text(&text, in_turn)?;
        labels.extend(redaction.labels);
        // A text edit restates the fact; pin/tag/entity edits do not.
        text_changed = memory.text != text;
        if text_changed {
            memory.stated_at = Some(stated_now_ms());
        }
        memory.text = text;
    }
    // Redacted before normalization lowercases them (see `save`).
    if let Some(mut entities) = req.entities {
        labels.extend(crate::redact::drop_secrets(&mut entities));
        let entities = normalize_entities(entities);
        if in_turn {
            refuse_label(&entities, crate::scope::LAYER_ENTITY, "entity")?;
        }
        memory.entities = entities;
    }
    if let Some(mut tags) = req.tags {
        labels.extend(crate::redact::drop_secrets(&mut tags));
        let tags = normalize_entities(tags);
        if in_turn {
            refuse_label(&tags, crate::learned::DEMOTED_TAG, "tag")?;
        }
        memory.tags = tags;
    }
    if let Some(pinned) = req.pinned {
        memory.pinned = pinned;
    }
    memory.updated_at = now_ms();
    memory.revision += 1;
    if bank
        .commit_expected(memory.clone(), Some(&expected))
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let reason = crate::redact::reason(&labels);
    deps.emitter
        .item_with(ItemEvent::Updated, &bank_name, &memory, reason.as_deref())
        .await;
    // The commit dropped the vector of the old text; embed the new one.
    if text_changed {
        crate::embed_client::spawn_backfill(deps, &bank_name);
    }
    Ok(Some(MemoryResponse { memory }))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
}

pub async fn delete(deps: &Deps, req: DeleteRequest) -> Result<MemoryResponse, MemoryError> {
    // A write that lands between the read and the commit re-reads.
    for _ in 0..COMMIT_TRIES {
        if let Some(deleted) = delete_once(deps, &req).await? {
            return Ok(deleted);
        }
    }
    Err(still_changing(&req.id))
}

async fn delete_once(
    deps: &Deps,
    req: &DeleteRequest,
) -> Result<Option<MemoryResponse>, MemoryError> {
    let caller = crate::scope::Caller::from_baggage();
    let (bank_name, layer) =
        crate::scope::write_bank(deps, &caller, req.bank.clone(), true).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let mut memory = bank
        .get(&req.id)
        .await
        .ok_or_else(|| MemoryError::MemoryNotFound(req.id.clone()))?;
    // From a turn: an unpinned memory the turn reads ("forget X"); the
    // tombstone is recoverable.
    if caller.in_turn() && memory.pinned {
        return Err(pinned_from_a_turn(&memory.id));
    }
    if layer && !crate::scope::layer_keeps(&bank_name, &deps.config().await.default_bank, &memory) {
        return Err(crate::scope::not_from_a_turn(format_args!(
            "deleting memory `{}` of bank `{bank_name}`, which this turn does not read",
            memory.id
        )));
    }
    let expected = memory.clone();
    if memory.invalid_at.is_none() {
        memory.invalid_at = Some(now_ms());
        memory.updated_at = now_ms();
        memory.revision += 1;
        if bank
            .commit_expected(memory.clone(), Some(&expected))
            .await?
            .is_none()
        {
            return Ok(None);
        }
        deps.emitter
            .item(ItemEvent::Deleted, &bank_name, &memory)
            .await;
    }
    Ok(Some(MemoryResponse { memory }))
}

/// Tries of a read/modify/write before a still-changing record fails the call.
const COMMIT_TRIES: usize = 3;

fn still_changing(id: &str) -> MemoryError {
    MemoryError::InvalidInput(format!(
        "memory `{id}` kept changing during the write; retry"
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PinRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
    pub pinned: bool,
}

pub async fn pin(deps: &Deps, req: PinRequest) -> Result<MemoryResponse, MemoryError> {
    update(
        deps,
        UpdateRequest {
            bank: req.bank,
            id: req.id,
            text: None,
            entities: None,
            tags: None,
            pinned: Some(req.pinned),
        },
    )
    .await
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SupersedeRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    /// Memory to retire.
    pub id: String,
    /// Live memory that replaces it.
    pub superseded_by: String,
    /// Optional proof-bound snapshot of the loser; compared under the same
    /// storage lock as the winner snapshot and both writes.
    #[serde(default)]
    pub expected_memory: Option<serde_json::Value>,
    /// Optional proof-bound snapshot of the winner. Legacy callers may omit both.
    #[serde(default)]
    pub expected_superseded_by: Option<serde_json::Value>,
    /// Merge semantics: the survivor absorbs the retired record's
    /// observations (corroboration += retired + 1) in the same call. A
    /// pinned survivor is never written. Default false (a state change:
    /// the new record replaces the old, nothing to absorb).
    #[serde(default)]
    pub reinforce: bool,
    /// Why — carried on the `memory::on-item-changed` events this call emits
    /// (not persisted on the record). Trimmed, capped at 500 chars.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SupersedeResponse {
    /// The retired record.
    pub memory: Memory,
    /// True when the survivor was reinforced (`reinforce` asked and the
    /// survivor is not pinned).
    pub reinforced: bool,
    /// Post-commit survivor snapshot for a subsequent proof-bound absorption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub survivor: Option<Memory>,
}

const MAX_REASON_CHARS: usize = 500;

pub(crate) fn clamp_reason(reason: Option<String>) -> Option<String> {
    let reason: String = reason?.trim().chars().take(MAX_REASON_CHARS).collect();
    (!reason.is_empty()).then_some(reason)
}

/// The two records a supersede writes: the retired loser and, when
/// reinforcing a non-pinned survivor, the survivor absorbing the loser's
/// observations — `(s+1) + (r+1)` total, so corroboration `s + r + 1`,
/// stamped with the LATEST of the two `updated_at` (a merge is not a fresh
/// restatement).
pub(crate) fn supersede_transition(
    mut loser: Memory,
    mut winner: Memory,
    reinforce: bool,
    now: u64,
) -> (Memory, Option<Memory>) {
    let loser_updated_at = loser.updated_at;
    let loser_corroboration = loser.corroboration;
    let loser_stated_at = loser.stated_at;
    loser.invalid_at = Some(now);
    loser.superseded_by = Some(winner.id.clone());
    loser.updated_at = now;
    loser.revision += 1;
    if !reinforce || winner.pinned {
        return (loser, None);
    }
    winner.corroboration = winner
        .corroboration
        .saturating_add(loser_corroboration)
        .saturating_add(1);
    winner.updated_at = winner.updated_at.max(loser_updated_at);
    winner.stated_at = winner.stated_at.max(loser_stated_at);
    winner.revision += 1;
    (loser, Some(winner))
}

/// Retire one memory in favor of another: tombstone with a pointer, never a
/// plain delete. The consolidation seam — siblings merge duplicates through
/// this instead of touching files. Pinned memories cannot be superseded.
pub async fn supersede(
    deps: &Deps,
    req: SupersedeRequest,
) -> Result<SupersedeResponse, MemoryError> {
    crate::scope::human_only(super::SUPERSEDE_ID)?;
    if req.id == req.superseded_by {
        return Err(MemoryError::InvalidInput(
            "a memory cannot supersede itself".into(),
        ));
    }
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let reason = clamp_reason(req.reason);
    let (retired, survivor) = bank
        .supersede(
            &req.id,
            &req.superseded_by,
            req.reinforce,
            (
                req.expected_memory.as_ref(),
                req.expected_superseded_by.as_ref(),
            ),
        )
        .await?;
    deps.emitter
        .item_with(
            ItemEvent::Superseded,
            &bank_name,
            &retired,
            reason.as_deref(),
        )
        .await;
    let reinforced = survivor.is_some();
    if let Some(survivor) = &survivor {
        // Both records are already durably committed by Bank::supersede.
        deps.emitter
            .item_with(ItemEvent::Updated, &bank_name, survivor, reason.as_deref())
            .await;
    }
    Ok(SupersedeResponse {
        memory: retired,
        reinforced,
        survivor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entityless_save_into_the_default_bank_is_about_the_user() {
        assert!(about_the_user(&[], "main", "main", false));
        assert!(
            about_the_user(&[" ".into()], "main", "main", false),
            "blank entities name nothing"
        );
        assert!(!about_the_user(
            &["ledger-sync".into()],
            "main",
            "main",
            false
        ));
        assert!(!about_the_user(&[], "w-abc123", "main", false));
        assert!(
            !about_the_user(&[], "main", "main", true),
            "a save from a turn is the agent's note"
        );
    }

    #[test]
    fn a_save_from_a_turn_records_its_session_and_turn() {
        let in_turn = || crate::scope::Caller {
            session_id: Some("s".into()),
            turn_id: Some("t".into()),
        };
        let recorded = |source: Option<Provenance>| {
            source.map(|p| (p.session_id, p.turn_id, p.entry_id, p.agent))
        };
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            recorded(save_source(None, in_turn())),
            Some((some("s"), some("t"), None, None))
        );
        assert_eq!(
            recorded(save_source(some("other"), in_turn())),
            Some((some("s"), some("t"), None, None)),
            "inside a turn the turn wins"
        );
        let no_turn = crate::scope::Caller {
            session_id: Some("s".into()),
            turn_id: None,
        };
        assert_eq!(
            recorded(save_source(None, no_turn)),
            Some((some("s"), None, None, None))
        );
        assert_eq!(
            recorded(save_source(some("named"), crate::scope::Caller::default())),
            Some((some("named"), None, None, None))
        );
        assert_eq!(save_source(None, crate::scope::Caller::default()), None);
    }

    fn record(id: &str, revision: u64, live: bool, pinned: bool) -> Memory {
        Memory {
            id: id.into(),
            text: "some text".into(),
            entities: vec![],
            tags: vec![],
            confidence: Confidence::Stated,
            corroboration: 2,
            pinned,
            source: None,
            created_at: 10,
            updated_at: 10,
            invalid_at: if live { None } else { Some(20) },
            superseded_by: if live { None } else { Some("fpw".into()) },
            revision,
            stated_at: None,
        }
    }

    #[test]
    fn fresh_save_starts_at_revision_zero_with_normalized_entities() {
        let (m, event, created) = save_transition(
            None,
            SaveInput {
                id: "fp1".into(),
                text: "text".into(),
                entities: vec![" Mike ".into(), "".into(), "BLOG".into()],
                tags: vec![" Intro ".into()],
                pinned: true,
                source: Some(Provenance {
                    session_id: Some("s_1".into()),
                    turn_id: Some("t_1".into()),
                    ..Provenance::default()
                }),
                now: 99,
                merge_entities: false,
                corroborates: true,
            },
        );
        assert!(created);
        assert!(matches!(event, ItemEvent::Created));
        assert_eq!(m.revision, 0);
        assert_eq!(m.stated_at, Some(99), "a new record is stated now");
        assert_eq!(m.entities, vec!["mike", "blog"]);
        assert_eq!(m.tags, vec!["intro"]);
        assert!(m.pinned);
        let source = m.source.unwrap();
        assert_eq!(source.session_id.as_deref(), Some("s_1"));
        assert_eq!(source.turn_id.as_deref(), Some("t_1"));
    }

    #[test]
    fn entity_cap_holds_at_eight() {
        let many: Vec<String> = (0..12).map(|i| format!("e{i}")).collect();
        let (m, _, _) = save_transition(
            None,
            SaveInput {
                id: "fp".into(),
                text: "t".into(),
                entities: many,
                tags: vec![],
                pinned: false,
                source: None,
                now: 1,
                merge_entities: false,
                corroborates: true,
            },
        );
        assert_eq!(m.entities.len(), 8);
    }

    #[test]
    fn live_resave_reinforces_and_never_unpins() {
        let input = SaveInput {
            id: "fp1".into(),
            text: "some text".into(),
            entities: vec!["User".into()],
            tags: vec!["fresh-tag".into()],
            pinned: false,
            source: None,
            now: 99,
            merge_entities: false,
            corroborates: true,
        };
        let (m, event, created) =
            save_transition(Some(record("fp1", 4, true, true)), input.clone());
        assert!(!created);
        assert!(matches!(event, ItemEvent::Updated));
        assert_eq!(m.revision, 5);
        assert_eq!(m.corroboration, 3);
        assert!(m.pinned, "a resave without pinned must not unpin");
        assert_eq!(m.tags, vec!["fresh-tag"], "re-observation adds labels");
        assert_eq!(m.updated_at, 99);
        assert!(m.entities.is_empty(), "a plain re-save keeps its entities");
        let merged = SaveInput {
            merge_entities: true,
            ..input.clone()
        };
        let (m, ..) = save_transition(Some(record("fp1", 4, true, true)), merged);
        assert_eq!(m.entities, ["user"], "a demotion's re-save adds them");
        let echo = SaveInput {
            corroborates: false,
            ..input
        };
        let (m, ..) = save_transition(Some(record("fp1", 4, true, true)), echo);
        assert_eq!(
            m.corroboration, 2,
            "only the user's restatement corroborates"
        );
        assert_eq!(m.stated_at, Some(99));
    }

    #[test]
    fn retired_resave_resurrects_and_continues_the_revision_counter() {
        let (m, event, created) = save_transition(
            Some(record("fp1", 7, false, false)),
            SaveInput {
                id: "fp1".into(),
                text: "some text".into(),
                entities: vec![],
                tags: vec![],
                pinned: false,
                source: None,
                now: 99,
                merge_entities: false,
                corroborates: true,
            },
        );
        assert!(created, "a resurrection re-enters live sets");
        assert_eq!(m.stated_at, Some(99), "re-saving restates the fact");
        assert!(matches!(event, ItemEvent::Created));
        assert_eq!(
            m.revision, 8,
            "revision continues — rollback would lose to the tombstone on replay"
        );
        assert!(m.invalid_at.is_none());
        assert!(m.superseded_by.is_none());
    }

    #[test]
    fn supersede_retires_and_reinforce_carries_the_retired_count() {
        let mut loser = record("fpl", 3, true, false);
        loser.corroboration = 6;
        loser.updated_at = 50;
        loser.stated_at = Some(45);
        let mut winner = record("fpw", 1, true, false);
        winner.corroboration = 0;
        winner.updated_at = 40;
        winner.stated_at = Some(30);
        let (retired, survivor) = supersede_transition(loser, winner.clone(), true, 99);
        assert_eq!(retired.invalid_at, Some(99));
        assert_eq!(retired.superseded_by.as_deref(), Some("fpw"));
        assert_eq!(retired.revision, 4);
        let survivor = survivor.expect("reinforced");
        // (0+1) + (6+1) observations = corroboration 7: the survivor never
        // ends below the record it absorbed.
        assert_eq!(survivor.corroboration, 7);
        // Latest of the two, NOT now: a merge is not a fresh restatement.
        assert_eq!(survivor.updated_at, 50);
        assert_eq!(survivor.stated_at, Some(45), "latest statement of the two");
        assert_eq!(survivor.revision, 2);

        let (_, none) = supersede_transition(record("fpl", 0, true, false), winner, false, 99);
        assert!(none.is_none(), "reinforce off leaves the survivor alone");
    }

    #[test]
    fn supersede_never_writes_a_pinned_survivor() {
        let pinned = record("fpw", 1, true, true);
        let (retired, survivor) =
            supersede_transition(record("fpl", 0, true, false), pinned, true, 99);
        assert!(retired.invalid_at.is_some(), "the loser still retires");
        assert!(
            survivor.is_none(),
            "pinned is untouchable by automatic paths"
        );
    }

    #[test]
    fn reason_is_trimmed_capped_in_chars_and_empty_is_none() {
        assert_eq!(
            clamp_reason(Some("  same fact  ".into())).as_deref(),
            Some("same fact")
        );
        assert_eq!(clamp_reason(Some("   ".into())), None);
        assert_eq!(clamp_reason(None), None);
        let long = "ção".repeat(300); // 900 chars, multibyte
        let clamped = clamp_reason(Some(long)).unwrap();
        assert_eq!(clamped.chars().count(), 500);
    }

    fn deps(dir: &std::path::Path, max_memories_per_turn: usize) -> Deps {
        crate::deps::offline_deps(
            dir,
            crate::config::WorkerConfig {
                embeddings_enabled: false,
                max_memories_per_turn,
                ..crate::config::WorkerConfig::default()
            },
        )
    }

    /// A turn `t-<id>` of a `kind` session `s-<id>` injected with `bank`, as
    /// the hook records both.
    fn a_turn(id: &str, kind: &str, bank: crate::scope::Resolved) {
        let meta = serde_json::json!({ "kind": kind });
        crate::hooks::remember_session(&format!("s-{id}"), &meta);
        crate::scope::remember_turn(&format!("t-{id}"), Some(bank));
    }

    fn named(bank: &str) -> crate::scope::Resolved {
        crate::scope::Resolved::named(bank)
    }

    /// `call` made from inside the turn [`a_turn`] recorded as `id`.
    async fn turn<T>(id: &str, call: impl Future<Output = T>) -> T {
        crate::scope::in_turn(&format!("s-{id}"), &format!("t-{id}"), call).await
    }

    fn note(bank: Option<&str>, text: &str) -> SaveRequest {
        SaveRequest {
            bank: bank.map(str::to_string),
            text: text.into(),
            entities: vec![],
            tags: vec![],
            pinned: false,
            session_id: None,
        }
    }

    fn edit(bank: Option<&str>, id: &str) -> UpdateRequest {
        UpdateRequest {
            bank: bank.map(str::to_string),
            id: id.into(),
            text: None,
            entities: None,
            tags: None,
            pinned: None,
        }
    }

    fn forget(bank: Option<&str>, id: &str) -> DeleteRequest {
        DeleteRequest {
            bank: bank.map(str::to_string),
            id: id.into(),
        }
    }

    /// The message of a `forbidden` refusal.
    fn forbidden<T: std::fmt::Debug>(result: Result<T, MemoryError>) -> String {
        match result {
            Err(MemoryError::Forbidden(message)) => message,
            other => panic!("expected forbidden, got {other:?}"),
        }
    }

    /// An extracted memory of `text`, committed into `bank`.
    async fn extracted(deps: &Deps, bank: &str, text: &str) -> Memory {
        let memory = Memory {
            text: text.into(),
            confidence: Confidence::Extracted,
            ..record(&fingerprint(text), 0, true, false)
        };
        let (bank, _) = deps.store().await.ensure_bank(bank, None).await.unwrap();
        bank.commit(memory.clone()).await.unwrap();
        memory
    }

    async fn stored(deps: &Deps, bank: &str, id: &str) -> Memory {
        let bank = deps.store().await.bank(bank).await.unwrap();
        bank.get(id).await.unwrap()
    }

    #[tokio::test]
    async fn a_save_from_a_turn_lands_only_in_the_turns_own_bank() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        // A repository's bank with an agent: `main` and `agent-x` are read under it.
        let own = crate::scope::Resolved {
            bank: "w-abc123".into(),
            derived: true,
            agent: Some("x".into()),
        };
        a_turn("own", "user", own);
        for other in ["main", "agent-x", "blog"] {
            let refused =
                forbidden(turn("own", save(&deps, note(Some(other), "Ops owns it."))).await);
            assert!(refused.contains(&format!("bank `{other}`")), "{refused}");
        }
        for bank in [None, Some("w-abc123")] {
            let text = format!("The deploy script lives in ops/ ({bank:?}).");
            let saved = turn("own", save(&deps, note(bank, &text)))
                .await
                .unwrap()
                .memory;
            assert!(saved.is_agent_note());
            assert_eq!(stored(&deps, "w-abc123", &saved.id).await, saved);
        }
        assert!(
            save(&deps, note(Some("main"), "Ops owns it."))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_turn_updates_only_in_its_own_bank_and_deletes_only_what_it_reads() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        // A repository's bank: `main`'s memories about the user, and all of
        // `agent-x`, are read under it.
        let derived = crate::scope::Resolved {
            bank: "w-def456".into(),
            derived: true,
            agent: Some("x".into()),
        };
        a_turn("layer", "user", derived);
        let about = save(&deps, note(Some("main"), "The user prefers tea."))
            .await
            .unwrap()
            .memory;
        let project = SaveRequest {
            entities: vec!["ledger".into()],
            ..note(Some("main"), "The ledger repository indents with tabs.")
        };
        let project = save(&deps, project).await.unwrap().memory;
        let text = |id: &str| UpdateRequest {
            text: Some("The user prefers green tea.".into()),
            ..edit(Some("main"), id)
        };
        let refused = forbidden(turn("layer", update(&deps, text(&about.id))).await);
        assert!(refused.contains("bank `main`"), "{refused}");
        let refused =
            forbidden(turn("layer", delete(&deps, forget(Some("main"), &project.id))).await);
        assert!(refused.contains("does not read"), "{refused}");
        forbidden(turn("layer", delete(&deps, forget(Some("blog"), &about.id))).await);
        assert!(stored(&deps, "main", &project.id).await.is_live());

        // "Forget that I prefer tea" from a repository: the layer's memory.
        let forgotten = turn("layer", delete(&deps, forget(Some("main"), &about.id)))
            .await
            .unwrap()
            .memory;
        assert!(!forgotten.is_live());
        let habit = save(&deps, note(Some("agent-x"), "Review the diff first."))
            .await
            .unwrap()
            .memory;
        let reword = UpdateRequest {
            text: Some("Review the tests first.".into()),
            ..edit(Some("agent-x"), &habit.id)
        };
        let refused = forbidden(turn("layer", update(&deps, reword)).await);
        assert!(refused.contains("bank `agent-x`"), "{refused}");
        let forgotten = turn("layer", delete(&deps, forget(Some("agent-x"), &habit.id)))
            .await
            .unwrap()
            .memory;
        assert!(!forgotten.is_live());
        assert!(update(&deps, text(&project.id)).await.is_ok());
        let deleted = delete(&deps, forget(Some("main"), &project.id))
            .await
            .unwrap();
        assert!(!deleted.memory.is_live());
    }

    #[tokio::test]
    async fn a_save_from_a_turn_never_brings_a_deleted_bank_back() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        let store = deps.store().await;
        store.ensure_bank("gone-abc123", None).await.unwrap();
        store.trash_bank("gone-abc123", "main").await.unwrap();
        a_turn("gone", "user", named("gone-abc123"));
        for bank in [Some("gone-abc123"), None] {
            let refused = forbidden(turn("gone", save(&deps, note(bank, "A note."))).await);
            assert!(refused.contains("no memory bank"), "{refused}");
            assert!(!refused.contains("pass `bank`"), "{refused}");
        }
        assert!(store.is_deleted("gone-abc123"));
        save(&deps, note(Some("gone-abc123"), "A note."))
            .await
            .unwrap();
        assert!(!store.is_deleted("gone-abc123"), "a human may name it");
    }

    #[tokio::test]
    async fn a_save_from_a_turn_refuses_a_pin() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("pin", "user", named("blog"));
        let pinned = |bank| SaveRequest {
            pinned: true,
            ..note(bank, "Ship on Fridays.")
        };
        let refused = forbidden(turn("pin", save(&deps, pinned(None))).await);
        assert!(refused.contains("pinned"), "{refused}");
        assert!(deps.store().await.bank("blog").await.is_err());
        let saved = save(&deps, pinned(Some("blog"))).await.unwrap();
        assert!(saved.memory.pinned);
    }

    #[tokio::test]
    async fn an_update_or_a_pin_from_a_turn_refuses_the_pin_flag() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("unpinned", "user", named("blog"));
        let noted = turn(
            "unpinned",
            save(&deps, note(None, "The staging host is blue.")),
        )
        .await
        .unwrap()
        .memory;
        let flag = UpdateRequest {
            pinned: Some(true),
            ..edit(None, &noted.id)
        };
        forbidden(turn("unpinned", update(&deps, flag)).await);
        let pin_it = |bank: Option<&str>| PinRequest {
            bank: bank.map(str::to_string),
            id: noted.id.clone(),
            pinned: true,
        };
        let refused = forbidden(turn("unpinned", pin(&deps, pin_it(None))).await);
        assert!(refused.contains("pinning"), "{refused}");
        assert!(!stored(&deps, "blog", &noted.id).await.pinned);
        assert!(
            pin(&deps, pin_it(Some("blog")))
                .await
                .unwrap()
                .memory
                .pinned
        );
    }

    #[tokio::test]
    async fn a_save_from_a_turn_refuses_the_user_entity() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("user-entity", "user", named("blog"));
        let about = |bank| SaveRequest {
            entities: vec![" User ".into()],
            ..note(bank, "The user prefers tea.")
        };
        let refused = forbidden(turn("user-entity", save(&deps, about(None))).await);
        assert!(refused.contains("`user` entity"), "{refused}");
        let saved = save(&deps, about(Some("blog"))).await.unwrap().memory;
        assert_eq!(saved.entities, ["user"]);
    }

    #[tokio::test]
    async fn an_update_from_a_turn_cannot_move_the_user_entity() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("move-user", "user", named("blog"));
        let noted = turn("move-user", save(&deps, note(None, "The user drinks tea.")))
            .await
            .unwrap()
            .memory;
        let entities = |entities: &[&str]| UpdateRequest {
            entities: Some(entities.iter().map(|e| e.to_string()).collect()),
            ..edit(Some("blog"), &noted.id)
        };
        let refused = forbidden(turn("move-user", update(&deps, entities(&["user"]))).await);
        assert!(refused.contains("`user` entity"), "{refused}");
        let moved = update(&deps, entities(&["user"])).await.unwrap().memory;
        assert_eq!(moved.entities, ["user"]);
        // About the user now: no longer a turn's note to edit.
        let refused = forbidden(turn("move-user", update(&deps, entities(&[]))).await);
        assert!(
            refused.contains("not a note saved from a turn"),
            "{refused}"
        );
        assert!(update(&deps, entities(&[])).await.is_ok());
    }

    #[tokio::test]
    async fn a_turn_cannot_add_or_remove_the_demoted_rule_tag() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("demoted", "user", named("blog"));
        let tagged = |bank| SaveRequest {
            tags: vec!["demoted-rule".into()],
            ..note(bank, "Prefer small commits.")
        };
        let refused = forbidden(turn("demoted", save(&deps, tagged(None))).await);
        assert!(refused.contains("`demoted-rule` tag"), "{refused}");
        assert!(save(&deps, tagged(Some("blog"))).await.is_ok());

        let noted = turn("demoted", save(&deps, note(None, "Prefer short branches.")))
            .await
            .unwrap()
            .memory;
        let tags = |tags: &[&str]| UpdateRequest {
            tags: Some(tags.iter().map(|t| t.to_string()).collect()),
            ..edit(None, &noted.id)
        };
        let refused = forbidden(turn("demoted", update(&deps, tags(&["Demoted-Rule"]))).await);
        assert!(refused.contains("`demoted-rule` tag"), "{refused}");
        let human = |tags| UpdateRequest {
            bank: Some("blog".into()),
            ..tags
        };
        update(&deps, human(tags(&["demoted-rule"]))).await.unwrap();
        let refused = forbidden(turn("demoted", update(&deps, tags(&[]))).await);
        assert!(
            refused.contains("not a note saved from a turn"),
            "{refused}"
        );
        let cleared = update(&deps, human(tags(&[]))).await.unwrap().memory;
        assert!(cleared.tags.is_empty());
    }

    #[tokio::test]
    async fn a_note_from_a_turn_is_3_to_500_characters() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("size", "user", named("blog"));
        // Characters, not bytes: each `é` is two.
        let of = |chars: usize| format!("Note {}", "é".repeat(chars - 5));
        let too_long = turn("size", save(&deps, note(None, &of(501)))).await;
        assert!(
            matches!(&too_long, Err(MemoryError::InvalidInput(m)) if m.contains("3..500")),
            "{too_long:?}"
        );
        let noted = turn("size", save(&deps, note(None, &of(500))))
            .await
            .unwrap()
            .memory;
        let longer = UpdateRequest {
            text: Some(of(501)),
            ..edit(None, &noted.id)
        };
        let too_long = turn("size", update(&deps, longer)).await;
        assert!(
            matches!(too_long, Err(MemoryError::InvalidInput(_))),
            "{too_long:?}"
        );
        assert!(save(&deps, note(Some("blog"), &of(501))).await.is_ok());
    }

    #[tokio::test]
    async fn a_turn_saves_at_most_max_memories_per_turn() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 2);
        a_turn("cap", "user", named("blog"));
        a_turn("cap-next", "user", named("blog"));
        for text in ["First note.", "Second note."] {
            assert!(
                turn("cap", save(&deps, note(None, text)))
                    .await
                    .unwrap()
                    .created
            );
        }
        let refused = forbidden(turn("cap", save(&deps, note(None, "Third note."))).await);
        assert!(refused.contains("max_memories_per_turn"), "{refused}");
        let again = turn("cap", save(&deps, note(None, "First note."))).await;
        assert!(!again.unwrap().created, "a re-save is not a new note");
        assert!(
            turn("cap-next", save(&deps, note(None, "Third note.")))
                .await
                .is_ok()
        );

        let no_turn = iii_helpers::observability::run_with_baggage(
            &[("iii.session.id", "s-cap")],
            save(&deps, note(None, "A note without its turn.")),
        )
        .await;
        assert!(forbidden(no_turn).contains("turn id"));

        for text in ["Human one.", "Human two.", "Human three."] {
            assert!(save(&deps, note(Some("blog"), text)).await.is_ok());
        }
    }

    #[tokio::test]
    async fn four_saves_from_a_turn_leave_corroboration_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("echo", "user", named("blog"));
        let text = "The staging host is blue.";
        let first = turn("echo", save(&deps, note(None, text)))
            .await
            .unwrap()
            .memory;
        for _ in 0..3 {
            let again = turn("echo", save(&deps, note(None, text))).await.unwrap();
            assert!(!again.created);
        }
        let echoed = stored(&deps, "blog", &first.id).await;
        assert_eq!(echoed.corroboration, 0, "a model echo is no restatement");
        assert_eq!(echoed.revision, 3);
        assert!(echoed.last_stated() >= first.last_stated());
        let said = save(&deps, note(Some("blog"), text)).await.unwrap().memory;
        assert_eq!(said.corroboration, 1, "the user saying it again is");
    }

    #[tokio::test]
    async fn a_save_from_a_turn_leaves_a_memory_it_may_not_revise_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("echo-held", "user", named("blog"));
        let styled = SaveRequest {
            tags: vec!["style".into()],
            ..note(None, "Ship on Fridays.")
        };
        let noted = turn("echo-held", save(&deps, styled)).await.unwrap().memory;
        let pin_it = PinRequest {
            bank: Some("blog".into()),
            id: noted.id,
            pinned: true,
        };
        let pinned = pin(&deps, pin_it).await.unwrap().memory;
        let held = extracted(&deps, "blog", "The blog runs on Hugo.").await;
        for kept in [pinned, held.clone()] {
            let echo = SaveRequest {
                tags: vec!["fresh-tag".into()],
                ..note(None, &kept.text)
            };
            let echoed = turn("echo-held", save(&deps, echo)).await.unwrap();
            assert!(!echoed.created);
            assert_eq!(echoed.memory, kept);
            assert_eq!(stored(&deps, "blog", &kept.id).await, kept);
        }
        let said = save(&deps, note(Some("blog"), &held.text)).await.unwrap();
        assert_eq!(said.memory.corroboration, held.corroboration + 1);
    }

    #[tokio::test]
    async fn an_automation_or_test_session_writes_nothing_from_a_turn() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        let held = extracted(&deps, "blog", "The blog runs on Hugo.").await;
        for kind in ["automation", "e2e"] {
            a_turn(kind, kind, named("blog"));
            let text = UpdateRequest {
                text: Some("The blog runs on Zola.".into()),
                ..edit(None, &held.id)
            };
            for refused in [
                forbidden(turn(kind, save(&deps, note(None, "A scripted note."))).await),
                forbidden(turn(kind, update(&deps, text)).await),
                forbidden(turn(kind, delete(&deps, forget(None, &held.id))).await),
            ] {
                assert!(refused.contains("not a user's session"), "{refused}");
            }
        }
        // A sub-agent of the user's session saves its note.
        let meta = serde_json::json!({ "kind": "user", "metadata": { "parent_session_id": "p" } });
        crate::hooks::remember_session("s-sub", &meta);
        crate::scope::remember_turn("t-sub", Some(named("blog")));
        assert!(
            turn("sub", save(&deps, note(None, "A sub-agent's note.")))
                .await
                .is_ok()
        );
        assert!(stored(&deps, "blog", &held.id).await.is_live());
        assert!(delete(&deps, forget(Some("blog"), &held.id)).await.is_ok());
    }

    #[tokio::test]
    async fn an_update_from_a_turn_needs_a_note_saved_from_a_turn() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("note-only", "user", named("blog"));
        let read = extracted(&deps, "blog", "The office is in Lisbon.").await;
        let typed = save(&deps, note(Some("blog"), "The team meets on Mondays."))
            .await
            .unwrap()
            .memory;
        // A line demoted from `learned` by a job that carried a turn's baggage.
        let demoted = Memory {
            text: "Prefer rebase over merge.".into(),
            entities: vec!["user".into()],
            tags: vec!["demoted-rule".into()],
            source: Some(Provenance {
                session_id: Some("s-job".into()),
                turn_id: Some("t-job".into()),
                ..Provenance::default()
            }),
            ..record(&fingerprint("Prefer rebase over merge."), 0, true, false)
        };
        assert!(demoted.is_agent_note());
        let bank = deps.store().await.bank("blog").await.unwrap();
        bank.commit(demoted.clone()).await.unwrap();
        let reword = |id: &str| UpdateRequest {
            text: Some("Reworded by the agent.".into()),
            ..edit(None, id)
        };
        for id in [&read.id, &typed.id, &demoted.id] {
            let refused = forbidden(turn("note-only", update(&deps, reword(id))).await);
            assert!(refused.contains("delete it and save"), "{refused}");
            assert_eq!(bank.get(id).await.unwrap().revision, 0, "{id}");
        }
        let noted = turn(
            "note-only",
            save(&deps, note(None, "The office is in Porto.")),
        )
        .await
        .unwrap()
        .memory;
        let revised = turn("note-only", update(&deps, reword(&noted.id)))
            .await
            .unwrap()
            .memory;
        assert_eq!(revised.text, "Reworded by the agent.");
        let human = UpdateRequest {
            bank: Some("blog".into()),
            text: Some("The office is in Faro.".into()),
            ..edit(None, &read.id)
        };
        assert!(update(&deps, human).await.is_ok());
    }

    #[tokio::test]
    async fn a_turn_deletes_an_unpinned_memory_but_not_a_pinned_one() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("forget", "user", named("blog"));
        let read = extracted(&deps, "blog", "The cat is called Miso.").await;
        let deleted = turn("forget", delete(&deps, forget(None, &read.id)))
            .await
            .unwrap()
            .memory;
        assert!(!deleted.is_live(), "forget X: a recoverable tombstone");

        let pinned = SaveRequest {
            pinned: true,
            ..note(Some("blog"), "The release train leaves on Thursdays.")
        };
        let pinned = save(&deps, pinned).await.unwrap().memory;
        let refused = forbidden(turn("forget", delete(&deps, forget(None, &pinned.id))).await);
        assert!(refused.contains("pinned memory"), "{refused}");
        let text = UpdateRequest {
            text: Some("The release train leaves on Fridays.".into()),
            ..edit(None, &pinned.id)
        };
        assert!(forbidden(turn("forget", update(&deps, text)).await).contains("pinned memory"));
        assert!(stored(&deps, "blog", &pinned.id).await.is_live());
        let deleted = delete(&deps, forget(Some("blog"), &pinned.id))
            .await
            .unwrap();
        assert!(!deleted.memory.is_live());
    }

    #[tokio::test]
    async fn a_turn_brings_back_only_a_deleted_note_of_its_own_kind() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        a_turn("revive", "user", named("blog"));
        a_turn("revive-next", "user", named("blog"));
        // The user's pinned fact about themselves, deleted on the memory page.
        let fact = SaveRequest {
            pinned: true,
            entities: vec!["user".into()],
            ..note(Some("blog"), "The user is allergic to nuts.")
        };
        let fact = save(&deps, fact).await.unwrap().memory;
        delete(&deps, forget(Some("blog"), &fact.id)).await.unwrap();
        let again = |bank| note(bank, "The user is allergic to nuts.");
        let refused = forbidden(turn("revive", save(&deps, again(None))).await);
        assert!(refused.contains("bringing back"), "{refused}");
        assert!(!stored(&deps, "blog", &fact.id).await.is_live());

        // The agent's own note, deleted, comes back as the later turn's note.
        let noted = turn(
            "revive",
            save(&deps, note(None, "The build cache is in /var.")),
        )
        .await
        .unwrap()
        .memory;
        turn("revive", delete(&deps, forget(None, &noted.id)))
            .await
            .unwrap();
        let back = turn(
            "revive-next",
            save(&deps, note(None, "The build cache is in /var.")),
        )
        .await
        .unwrap();
        assert!(back.created);
        let source = back.memory.source.unwrap();
        assert_eq!(source.turn_id.as_deref(), Some("t-revive-next"));

        let revived = save(&deps, again(Some("blog"))).await.unwrap();
        assert!(revived.created && revived.memory.pinned);
    }

    async fn saved(deps: &Deps, text: &str) -> Memory {
        save(deps, note(Some("blog"), text)).await.unwrap().memory
    }

    /// `old` retired in favor of `new`, as consolidation or a correction does.
    async fn replace(deps: &Deps, old: &Memory, new: &Memory) {
        let req = SupersedeRequest {
            bank: Some("blog".into()),
            id: old.id.clone(),
            superseded_by: new.id.clone(),
            expected_memory: None,
            expected_superseded_by: None,
            reinforce: false,
            reason: None,
        };
        supersede(deps, req).await.unwrap();
    }

    /// The message of a `memory_retired` refusal.
    fn retired<T: std::fmt::Debug>(result: Result<T, MemoryError>) -> String {
        match result {
            Err(MemoryError::InvalidInput(m)) if m.starts_with(MEMORY_RETIRED) => m,
            other => panic!("expected memory_retired, got {other:?}"),
        }
    }

    async fn live_ids(deps: &Deps) -> Vec<String> {
        let bank = deps.store().await.bank("blog").await.unwrap();
        let (live, _) = bank.list(10, 0, false, None).await;
        live.into_iter().map(|m| m.id).collect()
    }

    #[tokio::test]
    async fn an_update_on_a_retired_memory_is_refused_naming_the_live_one() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        let a = saved(&deps, "The office is in Lisbon.").await;
        let b = saved(&deps, "The office is in Porto.").await;
        replace(&deps, &a, &b).await;
        let (old, live) = (
            stored(&deps, "blog", &a.id).await,
            stored(&deps, "blog", &b.id).await,
        );
        let reword = |id: &str| UpdateRequest {
            text: Some("The office is in Faro.".into()),
            ..edit(Some("blog"), id)
        };
        let refused = retired(update(&deps, reword(&a.id)).await);
        assert!(
            refused.contains(&format!("superseded by `{}`", b.id)),
            "{refused}"
        );
        let pinned = PinRequest {
            bank: Some("blog".into()),
            id: a.id.clone(),
            pinned: true,
        };
        retired(pin(&deps, pinned).await);
        assert_eq!(stored(&deps, "blog", &a.id).await, old);
        assert_eq!(stored(&deps, "blog", &b.id).await, live);

        // Corrected again: the refusal names the end of the chain.
        let c = saved(&deps, "The office is in Braga.").await;
        replace(&deps, &b, &c).await;
        let refused = retired(update(&deps, reword(&a.id)).await);
        assert!(
            refused.contains(&format!("superseded by `{}`", c.id)),
            "{refused}"
        );
        // From a turn the advice is what a turn may do.
        a_turn("retired-update", "user", named("blog"));
        let refused = retired(turn("retired-update", update(&deps, reword(&a.id))).await);
        assert!(refused.contains("save a new note"), "{refused}");

        let d = saved(&deps, "The office has a cat.").await;
        delete(&deps, forget(Some("blog"), &d.id)).await.unwrap();
        let refused = retired(update(&deps, reword(&d.id)).await);
        assert!(refused.contains("no live memory replaced it"), "{refused}");

        let revised = update(&deps, reword(&c.id)).await.unwrap().memory;
        assert_eq!(revised.text, "The office is in Faro.");
    }

    #[tokio::test]
    async fn a_superseded_text_saved_again_is_refused_and_one_line_stays_live() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        let a = saved(&deps, "The deploy runs on Fridays.").await;
        let b = saved(&deps, "The deploy runs on Thursdays.").await;
        replace(&deps, &a, &b).await;
        let refused = retired(save(&deps, note(Some("blog"), &a.text)).await);
        assert!(refused.contains(&format!("memory `{}`", b.id)), "{refused}");
        a_turn("retired-save", "user", named("blog"));
        let refused = retired(turn("retired-save", save(&deps, note(None, &a.text))).await);
        assert!(refused.contains("save a new note"), "{refused}");
        assert_eq!(live_ids(&deps).await, [b.id]);
    }

    #[tokio::test]
    async fn a_deleted_text_saved_again_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path(), 8);
        let a = saved(&deps, "The cat is called Miso.").await;
        delete(&deps, forget(Some("blog"), &a.id)).await.unwrap();
        let back = save(&deps, note(Some("blog"), &a.text)).await.unwrap();
        assert!(back.created && back.memory.is_live());
        assert_eq!(live_ids(&deps).await, std::slice::from_ref(&a.id));

        // Superseded, then its replacement deleted: no live line holds the fact.
        let b = saved(&deps, "The cat is called Tofu.").await;
        replace(&deps, &a, &b).await;
        delete(&deps, forget(Some("blog"), &b.id)).await.unwrap();
        let back = save(&deps, note(Some("blog"), &a.text)).await.unwrap();
        assert!(back.created);
        assert_eq!(live_ids(&deps).await, [a.id]);
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TagsRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TagCount {
    pub tag: String,
    pub count: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TagsResponse {
    pub bank: String,
    pub tags: Vec<TagCount>,
}

pub async fn tags(deps: &Deps, req: TagsRequest) -> Result<TagsResponse, MemoryError> {
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    Ok(TagsResponse {
        bank: bank_name,
        tags: bank
            .tags()
            .await
            .into_iter()
            .map(|(tag, count)| TagCount { tag, count })
            .collect(),
    })
}
