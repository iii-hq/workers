use axum::http::StatusCode;

use crate::case::{ProtocolFamily, ProviderCase};
use crate::stub::StubResponse;

pub(crate) fn happy_sse(family: ProtocolFamily) -> &'static str {
    match family {
        ProtocolFamily::AnthropicMessages => concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"provider contract ok\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        ),
        ProtocolFamily::OpenAiChatCompletions => concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"provider contract ok\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n"
        ),
        ProtocolFamily::OpenAiResponses => concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"provider contract ok\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":3}}}\n\n"
        ),
    }
}

pub(crate) fn models_body(family: ProtocolFamily) -> &'static str {
    match family {
        ProtocolFamily::AnthropicMessages => {
            r#"{"data":[{"id":"claude-sonnet-4-6","display_name":"Claude Sonnet 4.6","max_input_tokens":200000,"max_tokens":8192}]}"#
        }
        ProtocolFamily::OpenAiChatCompletions => {
            r#"{"data":[{"id":"provider-contract-model","object":"model","owned_by":"provider-contract"}]}"#
        }
        ProtocolFamily::OpenAiResponses => {
            r#"{"data":[{"id":"gpt-5.2","object":"model"}],"models":[{"slug":"gpt-5.2","display_name":"GPT 5.2","visibility":"list","priority":1,"context_window":128000,"supported_reasoning_levels":[],"input_modalities":["text"]}]}"#
        }
    }
}

pub(crate) fn auth_response(family: ProtocolFamily) -> StubResponse {
    match family {
        ProtocolFamily::AnthropicMessages => StubResponse::json(
            StatusCode::UNAUTHORIZED,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid test credential"}}"#,
        ),
        ProtocolFamily::OpenAiChatCompletions | ProtocolFamily::OpenAiResponses => {
            StubResponse::json(
                StatusCode::UNAUTHORIZED,
                r#"{"error":{"message":"invalid test credential","type":"authentication_error","code":"invalid_api_key"}}"#,
            )
        }
    }
}

pub(crate) fn quota_response(case: ProviderCase) -> StubResponse {
    match case.id {
        "anthropic" | "claude-code" => StubResponse::json(
            StatusCode::BAD_REQUEST,
            r#"{"type":"error","error":{"type":"billing_error","message":"credit balance is too low"}}"#,
        ),
        "deepseek" => StubResponse::json(
            StatusCode::PAYMENT_REQUIRED,
            r#"{"error":{"message":"insufficient balance","type":"invalid_request_error","code":"insufficient_balance"}}"#,
        ),
        "openrouter" => StubResponse::json(
            StatusCode::PAYMENT_REQUIRED,
            r#"{"error":{"message":"insufficient credits","code":"insufficient_credits"}}"#,
        ),
        "kimi" => StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"quota exhausted","type":"exceeded_current_quota_error","code":"insufficient_quota"}}"#,
        ),
        "xai" => StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"quota exhausted","type":"invalid_request_error","code":"insufficient_quota"}}"#,
        ),
        "zai" => StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"insufficient balance","code":"1113"}}"#,
        ),
        _ => StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"You have no credits remaining.","type":"insufficient_quota","code":"credit_balance_exhausted"}}"#,
        ),
    }
}
