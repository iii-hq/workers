//! The registered function surface — the published interface. Each entry pairs a
//! function id with its typed request and response JSON schemas, so a golden test
//! (`tests/schemas.rs`) can snapshot them and assert none is the permissive
//! AnyValue schema (see `docs/sops/new-worker.md` §5).
//!
//! The HTTP ingress handlers (`slack::events`, `slack::interactions`) take a raw
//! channel payload (like `mcp::handler`) and are operator-only ingress, so they
//! are not part of this agent-facing typed catalog.

use schemars::schema::RootSchema;
use schemars::JsonSchema;

use crate::functions::admin::{ConfigStatus, ConfigStatusReq};
use crate::functions::bindings::BindingAck;
use crate::functions::files::{UploadReq, UploadResponse};
use crate::functions::notify::{NotifyRequest, NotifyResponse};
use crate::response::SlackResponse;
use crate::types::{
    MessageAddedEvent, MessageUpdatedEvent, PendingApprovalRecord, PendingResolvedEvent,
    TurnCompletedEvent,
};

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: RootSchema,
    pub response_schema: RootSchema,
}

fn schema_of<T: JsonSchema>() -> RootSchema {
    schemars::gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

/// A `slack::*` Web API method: typed request, `SlackResponse` response.
fn api<Req: JsonSchema>(function_id: &'static str, description: &'static str) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<SlackResponse>(),
    }
}

/// A function with a bespoke typed response.
fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

pub fn catalog() -> Vec<FunctionSpec> {
    use crate::functions::{chat, conversations, pins, reactions, users, views};
    vec![
        // chat
        api::<chat::PostMessageReq>("slack::chat::post-message", "Post a message."),
        api::<chat::UpdateReq>("slack::chat::update", "Edit a message."),
        api::<chat::DeleteReq>("slack::chat::delete", "Delete a message."),
        api::<chat::PostEphemeralReq>("slack::chat::post-ephemeral", "Post an ephemeral message."),
        api::<chat::ScheduleMessageReq>("slack::chat::schedule-message", "Schedule a message."),
        api::<chat::GetPermalinkReq>("slack::chat::get-permalink", "Get a message permalink."),
        // conversations
        api::<conversations::ListReq>("slack::conversations::list", "List conversations."),
        api::<conversations::ChannelReq>("slack::conversations::info", "Get conversation info."),
        api::<conversations::HistoryReq>("slack::conversations::history", "Fetch history."),
        api::<conversations::RepliesReq>("slack::conversations::replies", "Fetch thread replies."),
        api::<conversations::CreateReq>("slack::conversations::create", "Create a channel."),
        api::<conversations::InviteReq>("slack::conversations::invite", "Invite users."),
        api::<conversations::ChannelReq>("slack::conversations::join", "Join a channel."),
        api::<conversations::ChannelReq>("slack::conversations::members", "List members."),
        api::<conversations::OpenReq>("slack::conversations::open", "Open a DM/MPIM."),
        api::<conversations::SetTopicReq>("slack::conversations::set-topic", "Set topic."),
        api::<conversations::SetPurposeReq>("slack::conversations::set-purpose", "Set purpose."),
        api::<conversations::ChannelReq>("slack::conversations::archive", "Archive a channel."),
        // reactions
        api::<reactions::ReactionReq>("slack::reactions::add", "Add a reaction."),
        api::<reactions::ReactionReq>("slack::reactions::remove", "Remove a reaction."),
        api::<reactions::GetReq>("slack::reactions::get", "Get reactions."),
        // files
        spec::<UploadReq, UploadResponse>("slack::files::upload", "Upload a file."),
        api::<crate::functions::files::InfoReq>("slack::files::info", "Get file info."),
        api::<crate::functions::files::ListReq>("slack::files::list", "List files."),
        // users
        api::<users::ListReq>("slack::users::list", "List users."),
        api::<users::InfoReq>("slack::users::info", "Get user info."),
        api::<users::LookupByEmailReq>("slack::users::lookup-by-email", "Find a user by email."),
        api::<users::ProfileGetReq>("slack::users::profile-get", "Get a user profile."),
        // views
        api::<views::OpenReq>("slack::views::open", "Open a modal."),
        api::<views::PublishReq>("slack::views::publish", "Publish App Home."),
        api::<views::UpdateReq>("slack::views::update", "Update a view."),
        api::<views::PushReq>("slack::views::push", "Push a modal."),
        // pins / bookmarks
        api::<pins::PinReq>("slack::pins::add", "Pin a message."),
        api::<pins::PinsListReq>("slack::pins::list", "List pins."),
        api::<pins::BookmarkAddReq>("slack::bookmarks::add", "Add a bookmark."),
        // search
        api::<crate::functions::search::MessagesReq>("slack::search::messages", "Search messages."),
        // assistant
        api::<crate::functions::assistant::SetStatusReq>(
            "slack::assistant::set-status",
            "Set status.",
        ),
        api::<crate::functions::assistant::SetTitleReq>(
            "slack::assistant::set-title",
            "Set title.",
        ),
        api::<crate::functions::assistant::SetSuggestedPromptsReq>(
            "slack::assistant::set-suggested-prompts",
            "Set suggested prompts.",
        ),
        // admin / escape hatch
        api::<crate::functions::admin::AuthTestReq>("slack::auth::test", "Auth test."),
        spec::<ConfigStatusReq, ConfigStatus>("slack::config-status", "Config + identity status."),
        api::<crate::functions::call::CallReq>("slack::call", "Call any Slack method."),
        // bridge
        spec::<NotifyRequest, NotifyResponse>("slack::notify", "Out-of-band session send."),
        spec::<MessageAddedEvent, BindingAck>("slack::on-message-added", "Stream binding."),
        spec::<MessageUpdatedEvent, BindingAck>("slack::on-message-updated", "Stream binding."),
        spec::<TurnCompletedEvent, BindingAck>("slack::on-turn-completed", "Finalize binding."),
        spec::<PendingApprovalRecord, BindingAck>("slack::on-pending-created", "Approval binding."),
        spec::<PendingResolvedEvent, BindingAck>("slack::on-pending-resolved", "Approval binding."),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_namespaced() {
        let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
        let set: HashSet<&&str> = ids.iter().collect();
        assert_eq!(set.len(), ids.len(), "duplicate function id");
        for id in ids {
            assert!(id.starts_with("slack::"), "id not namespaced: {id}");
        }
    }
}
