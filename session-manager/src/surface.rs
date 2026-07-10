//! Wire-surface catalog for every function this worker registers — the single
//! source of truth for each function's id and schemars-derived request/response
//! schemas.
//!
//! Golden-tested in `tests/schemas.rs`; keep in lockstep with the boot
//! registration in `main.rs`: `store_protocol::register_store_protocol`, then
//! `functions::register_all`, then the two `configuration::register_*`
//! handlers. Schema generation MUST mirror iii-sdk's internal `json_schema_for`
//! (`SchemaSettings::draft07()` on the handler's request/response types) so a
//! catalog snapshot pins exactly what registration emits.
//!
//! Descriptions are intentionally not snapshotted here (they live inline at
//! each registration site); the goldens pin the request/response schemas — the
//! product surface this convention exists to keep typed, never the permissive
//! `AnyValue` ("unknown") schema a `Value` handler would emit.

use crate::configuration::{
    ConfigStatusRequest, OnConfigChangeEvent, OnConfigChangeResponse, ReloadStatus,
};
use crate::functions::{
    append, append_many, create, delete, ensure, fork, get, get_message, list, messages,
    set_active_leaf, set_draft, set_meta, set_status, store_protocol, update_message,
};
use crate::types::{SessionEntry, SessionMeta};

/// One function's wire surface: id plus the schemars-derived request/response
/// schemas.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

fn schema_of<T: schemars::JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req, Resp>(function_id: &'static str) -> FunctionSpec
where
    Req: schemars::JsonSchema,
    Resp: schemars::JsonSchema,
{
    FunctionSpec {
        function_id,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The full wire-surface catalog, in registration order. Golden-tested in
/// `tests/schemas.rs`; keep in lockstep with the boot sequence in `main.rs`.
pub fn catalog() -> Vec<FunctionSpec> {
    use store_protocol as sp;
    vec![
        // 1. internal store protocol (register_store_protocol order)
        spec::<sp::SessionIdRequest, Option<SessionMeta>>(sp::GET_META),
        spec::<sp::PutMetaRequest, sp::OkResponse>(sp::PUT_META),
        spec::<sp::SessionIdRequest, sp::OkResponse>(sp::DELETE_META),
        spec::<sp::ListMetasRequest, sp::ListMetasResponse>(sp::LIST_METAS),
        spec::<sp::EntryIdRequest, Option<SessionEntry>>(sp::GET_ENTRY),
        spec::<sp::PutEntryRequest, sp::OkResponse>(sp::PUT_ENTRY),
        spec::<sp::SessionIdRequest, sp::ListEntriesResponse>(sp::LIST_ENTRIES),
        spec::<sp::SessionIdRequest, sp::OkResponse>(sp::DELETE_ENTRIES),
        spec::<sp::SessionIdRequest, sp::ActiveLeafResponse>(sp::GET_ACTIVE_LEAF),
        spec::<sp::EntryIdRequest, sp::OkResponse>(sp::SET_ACTIVE_LEAF),
        spec::<sp::SessionIdRequest, sp::OkResponse>(sp::DELETE_ACTIVE_LEAF),
        spec::<sp::PublishEventsRequest, sp::PublishEventsResponse>(sp::PUBLISH_EVENTS),
        // 2. agent-facing session::* functions (register_all order)
        spec::<create::CreateRequest, create::CreateResponse>("session::create"),
        spec::<ensure::EnsureRequest, ensure::EnsureResponse>("session::ensure"),
        spec::<get::GetRequest, Option<get::GetResponse>>("session::get"),
        spec::<list::ListRequest, list::ListResponse>("session::list"),
        spec::<set_meta::SetMetaRequest, set_meta::SetMetaResponse>("session::set-meta"),
        spec::<set_draft::SetDraftRequest, set_draft::SetDraftResponse>("session::set-draft"),
        spec::<set_status::SetStatusRequest, set_status::SetStatusResponse>("session::set-status"),
        spec::<delete::DeleteRequest, delete::DeleteResponse>("session::delete"),
        spec::<append::AppendRequest, append::AppendResponse>("session::append"),
        spec::<append_many::AppendManyRequest, append_many::AppendManyResponse>(
            "session::append-many",
        ),
        spec::<update_message::UpdateMessageRequest, update_message::UpdateMessageResponse>(
            "session::update-message",
        ),
        spec::<messages::MessagesRequest, messages::MessagesResponse>("session::messages"),
        spec::<get_message::GetMessageRequest, Option<get_message::GetMessageResponse>>(
            "session::get-message",
        ),
        spec::<fork::ForkRequest, fork::ForkResponse>("session::fork"),
        spec::<set_active_leaf::SetActiveLeafRequest, set_active_leaf::SetActiveLeafResponse>(
            "session::set-active-leaf",
        ),
        // 3. configuration handlers (register_config_trigger, then register_config_status)
        spec::<OnConfigChangeEvent, OnConfigChangeResponse>("session::on-config-change"),
        spec::<ConfigStatusRequest, ReloadStatus>("session::config-status"),
    ]
}
