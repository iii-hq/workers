//! The public `memory::*` functions.
//!
//! Each `<area>.rs` holds the request/response types (serde +
//! `schemars::JsonSchema`, so the SDK emits request/response schemas) and
//! `pub async fn` handlers the registration closure wraps. Tests call the
//! same handlers directly, so engine-free tests exercise the exact
//! production code path.

pub mod banks;
pub mod correct;
pub mod items;
pub mod ops;
pub mod preview;
pub mod recall;
pub mod rest;
pub mod rules;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::deps::Deps;
use crate::error::MemoryError;

pub const BANK_CREATE_ID: &str = "memory::bank::create";
pub const BANK_CREATE_DESC: &str = "Create a named memory bank: an isolated scope of rules and memories, e.g. one per \
     project or persona; its description tells extraction what the bank is for, narrowing \
     what is kept. Idempotent; sessions select it via metadata `memory_bank`.";

pub const BANK_LIST_ID: &str = "memory::bank::list";
pub const BANK_LIST_DESC: &str = "List every memory bank with memory/rule/pin counts.";

pub const BANK_DELETE_ID: &str = "memory::bank::delete";
pub const BANK_DELETE_DESC: &str =
    "Delete a memory bank: the folder moves to the store's .trash, recoverable by hand.";

pub const SAVE_ID: &str = "memory::save";
pub const SAVE_DESC: &str = "Save one memory explicitly. Content-fingerprinted: re-saving the same text reinforces \
     the existing memory instead of duplicating it. A text a newer memory superseded is \
     refused with `memory_retired`, naming that memory. Called from a turn it is the agent's \
     note: 3 to 500 characters, in the turn's own bank, at most `max_memories_per_turn` new \
     notes per turn, with that session and turn as its source; a pin, another bank, the \
     `user` entity and the `demoted-rule` tag are refused there, and so is a save from an \
     automation or test session. From a turn a repeat adds no corroboration: it restates the \
     turn's own note and leaves any other memory as it is. Needed where the `# Memory` header asks for it (a \
     sub-agent, extraction off): save what a later session would need.";

pub const GET_ID: &str = "memory::get";
pub const GET_DESC: &str = "Fetch one memory by id, including superseded/tombstoned records.";

pub const LIST_ID: &str = "memory::list";
pub const LIST_DESC: &str = "List a bank's memories page by page, newest first. `include_superseded` shows history \
     too.";

pub const UPDATE_ID: &str = "memory::update";
pub const UPDATE_DESC: &str = "Revise a memory's text, entities, or pin flag in place (same id, bumped revision; the \
     prior revision stays in the on-disk log). A retired memory is refused with \
     `memory_retired`, naming the memory that replaced it. From a turn: only a note saved \
     from a turn, in the turn's own bank, never a pinned one, never the pin flag.";

pub const DELETE_ID: &str = "memory::delete";
pub const DELETE_DESC: &str = "Delete a memory (tombstone: sets invalid_at). It leaves recall immediately but stays \
     on disk, readable with include_superseded. From a turn: a memory the turn reads (its \
     bank's, or one recalled from a bank read under it); a pinned memory is refused.";

pub const PIN_ID: &str = "memory::pin";
pub const PIN_DESC: &str = "Pin or unpin a memory. A pinned memory is shown on every turn that reads its bank, \
     first in the memory message and under a cap (`pinned_limit`, `pinned_max_chars`), ranks \
     higher in recall and is untouchable by every automatic path. Refused from inside a turn.";

pub const SUPERSEDE_ID: &str = "memory::supersede";
pub const SUPERSEDE_DESC: &str = "Retire one memory in favor of another: tombstone with a superseded_by pointer, never a \
     plain delete. The consolidation seam; pinned memories cannot be superseded. reinforce \
     makes it a merge (the non-pinned survivor absorbs the retired record's observations); \
     reason is carried on the memory::on-item-changed events.";

pub const TAGS_ID: &str = "memory::tags";
pub const TAGS_DESC: &str = "List the tags across a bank's live memories with usage counts, most used first — the \
     filter source for tag-scoped list/recall.";

pub const PREVIEW_ID: &str = "memory::preview";
pub const PREVIEW_DESC: &str = "Preview the memory injection for a hypothetical chat message: the exact system-prompt \
     memory section a new session gets (rules, budgets, truncation markers; running sessions \
     receive changes as an appended update), the memories the turn would be handed (past \
     the relevance cut and token budget), and the appended message verbatim. Runs the \
     pre-generate hook's own code.";

pub const RECALL_ID: &str = "memory::recall";
pub const RECALL_DESC: &str = "Search a bank's memories relevant to a query (BM25 + entities + corroboration + \
     recency; no LLM). The same scorer the pre-generate hook uses; the hook injects only \
     the hits with evidence, one line per fact and the newer of a stated change, and \
     memory::preview shows exactly what it injects. mode lexical/semantic returns one raw \
     arm (plain BM25, or cosine).";

pub const RULE_LIST_ID: &str = "memory::rule::list";
pub const RULE_LIST_DESC: &str = "List a bank's rules (markdown, injected whole into the system prompt on every turn). \
     Stored as plain files under the bank's rules/ folder; editing them by hand is equivalent. \
     Content is returned redacted, as it is injected.";

pub const RULE_SET_ID: &str = "memory::rule::set";
pub const RULE_SET_DESC: &str = "Create or replace one rule (atomic write; empty content removes it). With `expected`, \
     the write is refused as `rule_changed` when the rule no longer holds that content. Rules \
     are plain markdown files under the bank's rules/ folder.";

pub const DOCTOR_ID: &str = "memory::doctor";
pub const DOCTOR_DESC: &str = "Self-test memory end to end: data-dir writability, a scratch-bank write/read/delete \
     roundtrip, sibling reachability (router, session-manager, state), whether embeddings \
     answer, whether recent extraction passes failed, which model extraction calls and why, \
     how many extraction jobs wait in the dead-letter queue, and what extraction spent (model \
     calls, tokens, the router's cost) and wrote since boot. Reports degraded states explicitly, not \
     a bare process-up health.";

pub const RELOAD_ID: &str = "memory::reload";
pub const RELOAD_DESC: &str = "Drop all in-RAM state and reload every bank from disk — the recovery hatch after editing \
     bank files by hand.";

/// Register one typed handler under `id`, mapping `MemoryError` into the
/// bus error shape (`code: message`). `internal` hides plumbing functions
/// from the discoverable list.
fn register<Req, Resp, F, Fut>(
    iii: &Arc<IIIClient>,
    deps: &Arc<Deps>,
    id: &str,
    description: &str,
    internal: bool,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, MemoryError>> + Send + 'static,
{
    let deps = deps.clone();
    let reg = RegisterFunction::new_async(move |req: Req| {
        let deps = deps.clone();
        let handler = handler.clone();
        async move { handler(deps, req).await.map_err(Error::from) }
    })
    .description(description);
    let reg = if internal {
        reg.metadata(json!({ "internal": true, "trace_hidden": true }))
    } else {
        reg
    };
    iii.register_function(id, reg);
}

pub fn register_all(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    register(
        iii,
        deps,
        BANK_CREATE_ID,
        BANK_CREATE_DESC,
        false,
        |d, r| async move { banks::create(&d, r).await },
    );
    register(
        iii,
        deps,
        BANK_LIST_ID,
        BANK_LIST_DESC,
        false,
        |d, r| async move { banks::list(&d, r).await },
    );
    register(
        iii,
        deps,
        BANK_DELETE_ID,
        BANK_DELETE_DESC,
        false,
        |d, r| async move { banks::delete(&d, r).await },
    );
    register(iii, deps, SAVE_ID, SAVE_DESC, false, |d, r| async move {
        items::save(&d, r).await
    });
    register(iii, deps, GET_ID, GET_DESC, false, |d, r| async move {
        items::get(&d, r).await
    });
    register(iii, deps, LIST_ID, LIST_DESC, false, |d, r| async move {
        items::list(&d, r).await
    });
    register(
        iii,
        deps,
        UPDATE_ID,
        UPDATE_DESC,
        false,
        |d, r| async move { items::update(&d, r).await },
    );
    register(
        iii,
        deps,
        DELETE_ID,
        DELETE_DESC,
        false,
        |d, r| async move { items::delete(&d, r).await },
    );
    register(iii, deps, PIN_ID, PIN_DESC, false, |d, r| async move {
        items::pin(&d, r).await
    });
    register(
        iii,
        deps,
        SUPERSEDE_ID,
        SUPERSEDE_DESC,
        false,
        |d, r| async move { items::supersede(&d, r).await },
    );
    register(iii, deps, TAGS_ID, TAGS_DESC, false, |d, r| async move {
        items::tags(&d, r).await
    });
    register(
        iii,
        deps,
        PREVIEW_ID,
        PREVIEW_DESC,
        false,
        |d, r| async move { preview::preview(&d, r).await },
    );
    register(
        iii,
        deps,
        RECALL_ID,
        RECALL_DESC,
        false,
        |d, r| async move { recall::recall(&d, r).await },
    );
    register(
        iii,
        deps,
        RULE_LIST_ID,
        RULE_LIST_DESC,
        false,
        |d, r| async move { rules::list(&d, r).await },
    );
    register(
        iii,
        deps,
        RULE_SET_ID,
        RULE_SET_DESC,
        false,
        |d, r| async move { rules::set(&d, r).await },
    );
    register(
        iii,
        deps,
        DOCTOR_ID,
        DOCTOR_DESC,
        false,
        |d, r| async move { ops::doctor(&d, r).await },
    );
    register(
        iii,
        deps,
        RELOAD_ID,
        RELOAD_DESC,
        false,
        |d, r| async move { ops::reload(&d, r).await },
    );
    // The verify tier's seams: internal, out of `catalog()` and `call_json`.
    register(
        iii,
        deps,
        correct::TRANSCRIPT_ID,
        correct::TRANSCRIPT_DESC,
        true,
        |d, r| async move { correct::transcript(&d, r).await },
    );
    register(
        iii,
        deps,
        correct::CORRECT_ID,
        correct::CORRECT_DESC,
        true,
        |d, r| async move { correct::correct(&d, r).await },
    );

    tracing::info!("all memory::* functions registered");
}

/// Run the public function `function_id` on a JSON input, as its
/// registration does: the input is decoded into the function's request
/// type (a mismatch is `invalid_input`) and the result encoded as JSON.
/// `None` when no public function has that id.
pub async fn call_json(
    deps: &Deps,
    function_id: &str,
    input: Value,
) -> Option<Result<Value, MemoryError>> {
    let result = match function_id {
        BANK_CREATE_ID => json_call(input, |r| banks::create(deps, r)).await,
        BANK_LIST_ID => json_call(input, |r| banks::list(deps, r)).await,
        BANK_DELETE_ID => json_call(input, |r| banks::delete(deps, r)).await,
        SAVE_ID => json_call(input, |r| items::save(deps, r)).await,
        GET_ID => json_call(input, |r| items::get(deps, r)).await,
        LIST_ID => json_call(input, |r| items::list(deps, r)).await,
        UPDATE_ID => json_call(input, |r| items::update(deps, r)).await,
        DELETE_ID => json_call(input, |r| items::delete(deps, r)).await,
        PIN_ID => json_call(input, |r| items::pin(deps, r)).await,
        SUPERSEDE_ID => json_call(input, |r| items::supersede(deps, r)).await,
        TAGS_ID => json_call(input, |r| items::tags(deps, r)).await,
        PREVIEW_ID => json_call(input, |r| preview::preview(deps, r)).await,
        RECALL_ID => json_call(input, |r| recall::recall(deps, r)).await,
        RULE_LIST_ID => json_call(input, |r| rules::list(deps, r)).await,
        RULE_SET_ID => json_call(input, |r| rules::set(deps, r)).await,
        DOCTOR_ID => json_call(input, |r| ops::doctor(deps, r)).await,
        RELOAD_ID => json_call(input, |r| ops::reload(deps, r)).await,
        _ => return None,
    };
    Some(result)
}

async fn json_call<Req, Resp, F, Fut>(input: Value, handler: F) -> Result<Value, MemoryError>
where
    Req: DeserializeOwned,
    Resp: Serialize,
    F: FnOnce(Req) -> Fut,
    Fut: Future<Output = Result<Resp, MemoryError>>,
{
    let request =
        serde_json::from_value(input).map_err(|e| MemoryError::InvalidInput(e.to_string()))?;
    let response = handler(request).await?;
    serde_json::to_value(response)
        .map_err(|e| MemoryError::Storage(format!("the response did not serialize: {e}")))
}

/// One function's complete agent-facing wire surface: id, registration
/// description, and the schemars-derived request/response schemas.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Schema generation MUST mirror iii-sdk's internal `json_schema_for`
/// (`SchemaSettings::draft07()` on the handler's request/response types)
/// so a catalog snapshot pins exactly what registration emits.
fn schema_of<T: schemars::JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req, Resp>(function_id: &'static str, description: &'static str) -> FunctionSpec
where
    Req: schemars::JsonSchema,
    Resp: schemars::JsonSchema,
{
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The full wire-surface catalog, in registration order. Golden-tested in
/// `tests/schemas.rs`; keep in lockstep with `register_all`.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<banks::BankCreateRequest, banks::BankCreateResponse>(
            BANK_CREATE_ID,
            BANK_CREATE_DESC,
        ),
        spec::<banks::BankListRequest, banks::BankListResponse>(BANK_LIST_ID, BANK_LIST_DESC),
        spec::<banks::BankDeleteRequest, banks::BankDeleteResponse>(
            BANK_DELETE_ID,
            BANK_DELETE_DESC,
        ),
        spec::<items::SaveRequest, items::SaveResponse>(SAVE_ID, SAVE_DESC),
        spec::<items::GetRequest, items::GetResponse>(GET_ID, GET_DESC),
        spec::<items::ListRequest, items::ListResponse>(LIST_ID, LIST_DESC),
        spec::<items::UpdateRequest, items::MemoryResponse>(UPDATE_ID, UPDATE_DESC),
        spec::<items::DeleteRequest, items::MemoryResponse>(DELETE_ID, DELETE_DESC),
        spec::<items::PinRequest, items::MemoryResponse>(PIN_ID, PIN_DESC),
        spec::<items::SupersedeRequest, items::SupersedeResponse>(SUPERSEDE_ID, SUPERSEDE_DESC),
        spec::<items::TagsRequest, items::TagsResponse>(TAGS_ID, TAGS_DESC),
        spec::<preview::PreviewRequest, preview::PreviewResponse>(PREVIEW_ID, PREVIEW_DESC),
        spec::<recall::RecallRequest, recall::RecallResponse>(RECALL_ID, RECALL_DESC),
        spec::<rules::RuleListRequest, rules::RuleListResponse>(RULE_LIST_ID, RULE_LIST_DESC),
        spec::<rules::RuleSetRequest, rules::RuleSetResponse>(RULE_SET_ID, RULE_SET_DESC),
        spec::<ops::DoctorRequest, ops::DoctorResponse>(DOCTOR_ID, DOCTOR_DESC),
        spec::<ops::ReloadRequest, ops::ReloadResponse>(RELOAD_ID, RELOAD_DESC),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;
    use crate::types::Memory;

    fn deps(dir: &std::path::Path) -> Deps {
        crate::deps::offline_deps(
            dir,
            WorkerConfig {
                embeddings_enabled: false,
                ..WorkerConfig::default()
            },
        )
    }

    /// `call` made from inside an agent turn: refused as `forbidden`, naming `id`.
    async fn refused<T: std::fmt::Debug>(
        id: &str,
        call: impl Future<Output = Result<T, MemoryError>>,
    ) {
        let err = crate::scope::in_turn("s-agent", "t-agent", call)
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::Forbidden(_)), "{err}");
        assert!(err.to_string().contains(id), "{err}");
    }

    async fn saved(deps: &Deps, text: &str) -> Memory {
        let req = items::SaveRequest {
            bank: Some("blog".into()),
            text: text.into(),
            entities: vec![],
            tags: vec![],
            pinned: false,
            session_id: None,
        };
        items::save(deps, req).await.unwrap().memory
    }

    #[tokio::test]
    async fn bank_create_is_refused_from_a_turn_and_runs_outside_one() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path());
        let req = || banks::BankCreateRequest {
            name: "blog".into(),
            description: None,
        };
        refused(BANK_CREATE_ID, banks::create(&deps, req())).await;
        assert!(deps.store().await.bank("blog").await.is_err());
        assert!(banks::create(&deps, req()).await.unwrap().created);
    }

    #[tokio::test]
    async fn bank_delete_is_refused_from_a_turn_and_runs_outside_one() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path());
        let store = deps.store().await;
        store.ensure_bank("blog", None).await.unwrap();
        let req = || banks::BankDeleteRequest {
            name: "blog".into(),
        };
        refused(BANK_DELETE_ID, banks::delete(&deps, req())).await;
        assert!(store.bank("blog").await.is_ok());
        assert!(banks::delete(&deps, req()).await.unwrap().ok);
        assert!(store.bank("blog").await.is_err());
    }

    #[tokio::test]
    async fn rule_set_is_refused_from_a_turn_and_runs_outside_one() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path());
        let req = || rules::RuleSetRequest {
            bank: Some("blog".into()),
            name: "style".into(),
            content: "No em-dashes.".into(),
            expected: None,
        };
        refused(RULE_SET_ID, rules::set(&deps, req())).await;
        assert!(
            deps.store().await.bank("blog").await.is_err(),
            "nothing written, not even the bank"
        );
        assert!(rules::set(&deps, req()).await.unwrap().exists);
        let bank = deps.store().await.bank("blog").await.unwrap();
        assert_eq!(bank.list_rules().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn supersede_is_refused_from_a_turn_and_runs_outside_one() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path());
        let loser = saved(&deps, "The blog deploys on Fridays.").await;
        let winner = saved(&deps, "The blog deploys on Mondays.").await;
        let req = || items::SupersedeRequest {
            bank: Some("blog".into()),
            id: loser.id.clone(),
            superseded_by: winner.id.clone(),
            expected_memory: None,
            expected_superseded_by: None,
            reinforce: false,
            reason: None,
        };
        refused(SUPERSEDE_ID, items::supersede(&deps, req())).await;
        let bank = deps.store().await.bank("blog").await.unwrap();
        assert_eq!(bank.get(&loser.id).await, Some(loser.clone()));
        items::supersede(&deps, req()).await.unwrap();
        let retired = bank.get(&loser.id).await.unwrap();
        assert_eq!(retired.superseded_by, Some(winner.id));
    }

    #[tokio::test]
    async fn correct_is_refused_from_a_turn_and_runs_outside_one() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path());
        let old = saved(&deps, "Amber instruments require copper calibration.").await;
        let req = || correct::CorrectRequest {
            bank: Some("blog".into()),
            id: old.id.clone(),
            text: "Amber instruments require silver calibration.".into(),
            reason: "the source conversation says silver".into(),
            expected_memory: json!({
                "id": old.id,
                "revision": old.revision,
                "updated_at": old.updated_at,
                "text": old.text,
            }),
        };
        refused(correct::CORRECT_ID, correct::correct(&deps, req())).await;
        let bank = deps.store().await.bank("blog").await.unwrap();
        assert_eq!(bank.get(&old.id).await, Some(old.clone()));
        let fixed = correct::correct(&deps, req()).await.unwrap();
        assert!(fixed.corrected, "{:?}", fixed.skipped);
    }

    #[tokio::test]
    async fn transcript_is_refused_from_a_turn_and_runs_outside_one() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path());
        deps.store().await.ensure_bank("blog", None).await.unwrap();
        let req = || correct::TranscriptRequest {
            bank: Some("blog".into()),
            id: "nope".into(),
        };
        refused(correct::TRANSCRIPT_ID, correct::transcript(&deps, req())).await;
        let reply = correct::transcript(&deps, req()).await.unwrap();
        assert_eq!(reply.missing.as_deref(), Some("not_found"));
    }

    #[tokio::test]
    async fn reload_is_refused_from_a_turn_and_runs_outside_one() {
        let dir = tempfile::tempdir().unwrap();
        let deps = deps(dir.path());
        refused(RELOAD_ID, ops::reload(&deps, ops::ReloadRequest {})).await;
        assert!(ops::reload(&deps, ops::ReloadRequest {}).await.unwrap().ok);
    }
}
