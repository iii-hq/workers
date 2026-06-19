use schemars::schema::RootSchema;
use schemars::JsonSchema;

use crate::configuration::{ConfigChangeAck, ConfigChangeRequest};
use crate::functions::bindings::message_added::BindingAck;
use crate::functions::set_webhook::{SetWebhookRequest, SetWebhookResponse};
use crate::functions::webhook::WebhookResponse;
use crate::types::{
    HttpTriggerRequest, MessageAddedEvent, MessageUpdatedEvent, PendingApprovalRecord,
    PendingResolvedEvent, StatusChangedEvent, TurnCompletedEvent,
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
    vec![
        spec::<HttpTriggerRequest, WebhookResponse>(
            "telegram-bot::webhook",
            "Receive Telegram updates; route commands, messages, and callbacks.",
        ),
        spec::<SetWebhookRequest, SetWebhookResponse>(
            "telegram-bot::set-webhook",
            "Register the Telegram webhook URL from configuration.",
        ),
        spec::<MessageAddedEvent, BindingAck>(
            "telegram-bot::on-message-added",
            "Create a Telegram message for each new assistant or function_result entry.",
        ),
        spec::<MessageUpdatedEvent, BindingAck>(
            "telegram-bot::on-message-updated",
            "Stream assistant edits into Telegram, throttled by revision.",
        ),
        spec::<StatusChangedEvent, BindingAck>(
            "telegram-bot::on-status-changed",
            "Observe session status changes.",
        ),
        spec::<TurnCompletedEvent, BindingAck>(
            "telegram-bot::on-turn-completed",
            "Drain FIFO queue and post turn outcome toasts.",
        ),
        spec::<PendingApprovalRecord, BindingAck>(
            "telegram-bot::on-pending-created",
            "Send an inline approval keyboard when a function call is held.",
        ),
        spec::<PendingResolvedEvent, BindingAck>(
            "telegram-bot::on-pending-resolved",
            "Clear the approval prompt when a held call is resolved.",
        ),
        spec::<ConfigChangeRequest, ConfigChangeAck>(
            "telegram-bot::on-config-change",
            "Internal: reload telegram-bot configuration on change.",
        ),
    ]
}
