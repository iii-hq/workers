//! The public `memory::*` functions.
//!
//! Each `<area>.rs` holds the request/response types (serde +
//! `schemars::JsonSchema`, so the SDK emits request/response schemas) and
//! `pub async fn` handlers the registration closure wraps. Tests call the
//! same handlers directly, so engine-free tests exercise the exact
//! production code path.

pub mod banks;
pub mod items;
pub mod ops;
pub mod recall;
pub mod rules;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;

use crate::deps::Deps;
use crate::error::MemoryError;

pub const BANK_CREATE_ID: &str = "memory::bank::create";
pub const BANK_CREATE_DESC: &str =
    "Create (or idempotently ensure) a named memory bank — an isolated scope of rules and \
     memories, e.g. one per project or persona. Sessions select it via metadata `memory_bank`.";

pub const BANK_LIST_ID: &str = "memory::bank::list";
pub const BANK_LIST_DESC: &str = "List every memory bank with memory/rule/pin counts.";

pub const BANK_DELETE_ID: &str = "memory::bank::delete";
pub const BANK_DELETE_DESC: &str =
    "Move a bank's folder into the store's .trash (recoverable by hand); never destroys data.";

pub const SAVE_ID: &str = "memory::save";
pub const SAVE_DESC: &str =
    "Save one memory explicitly. Content-fingerprinted: re-saving the same text reinforces \
     the existing memory instead of duplicating it. Use when the user says `remember this`.";

pub const GET_ID: &str = "memory::get";
pub const GET_DESC: &str = "Fetch one memory by id, including superseded/tombstoned records.";

pub const LIST_ID: &str = "memory::list";
pub const LIST_DESC: &str =
    "Page through a bank's memories, newest first. `include_superseded` shows history too.";

pub const UPDATE_ID: &str = "memory::update";
pub const UPDATE_DESC: &str =
    "Revise a memory's text, entities, or pin flag in place (same id, bumped revision; the \
     prior revision stays in the on-disk log).";

pub const DELETE_ID: &str = "memory::delete";
pub const DELETE_DESC: &str =
    "Tombstone a memory (sets invalid_at). It leaves recall immediately but stays on disk \
     and readable with include_superseded.";

pub const PIN_ID: &str = "memory::pin";
pub const PIN_DESC: &str =
    "Pin or unpin a memory. Pinned memories rank higher in recall and are untouchable by \
     every automatic path.";

pub const SUPERSEDE_ID: &str = "memory::supersede";
pub const SUPERSEDE_DESC: &str =
    "Retire one memory in favor of another: tombstone with a superseded_by pointer, never a \
     plain delete. The consolidation seam; pinned memories cannot be superseded.";

pub const TAGS_ID: &str = "memory::tags";
pub const TAGS_DESC: &str =
    "Distinct tags across a bank's live memories with usage counts, most used first — the \
     filter source for tag-scoped list/recall.";

pub const RECALL_ID: &str = "memory::recall";
pub const RECALL_DESC: &str =
    "Rank a bank's memories against a query (BM25 + entity match + corroboration + recency; \
     no LLM). The same scorer the pre-generate hook uses — call it to preview an injection.";

pub const RULE_LIST_ID: &str = "memory::rule::list";
pub const RULE_LIST_DESC: &str =
    "List a bank's rules (markdown, injected whole into the system prompt on every turn). \
     Stored as plain files under the bank's rules/ folder; editing them by hand is equivalent.";

pub const RULE_SET_ID: &str = "memory::rule::set";
pub const RULE_SET_DESC: &str =
    "Create or replace one rule (atomic write; empty content removes it). Rules are plain \
     markdown files under the bank's rules/ folder.";

pub const DOCTOR_ID: &str = "memory::doctor";
pub const DOCTOR_DESC: &str =
    "End-to-end self-test: data-dir writability, a save→recall→trash roundtrip in a scratch \
     bank, and sibling reachability (router, session-manager). Reports degraded states \
     explicitly instead of a bare process-up health.";

pub const RELOAD_ID: &str = "memory::reload";
pub const RELOAD_DESC: &str =
    "Drop all in-RAM state and reload every bank from disk — the recovery hatch after editing \
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

    tracing::info!("all memory::* functions registered");
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
        spec::<items::SupersedeRequest, items::MemoryResponse>(SUPERSEDE_ID, SUPERSEDE_DESC),
        spec::<items::TagsRequest, items::TagsResponse>(TAGS_ID, TAGS_DESC),
        spec::<recall::RecallRequest, recall::RecallResponse>(RECALL_ID, RECALL_DESC),
        spec::<rules::RuleListRequest, rules::RuleListResponse>(RULE_LIST_ID, RULE_LIST_DESC),
        spec::<rules::RuleSetRequest, rules::RuleSetResponse>(RULE_SET_ID, RULE_SET_DESC),
        spec::<ops::DoctorRequest, ops::DoctorResponse>(DOCTOR_ID, DOCTOR_DESC),
        spec::<ops::ReloadRequest, ops::ReloadResponse>(RELOAD_ID, RELOAD_DESC),
    ]
}
