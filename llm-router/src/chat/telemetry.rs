//! gen_ai span attributes for LLM calls.
//!
//! The iii SDK opens an `execute router::chat` span around the handler; this
//! module stamps token usage, cost, and response metadata onto that active
//! span so the traces UI can show what each LLM call consumed. Attribute
//! names follow the OTel GenAI semantic conventions (`gen_ai.*`);
//! `gen_ai.usage.cost_usd` and the cache/reasoning token counts are
//! extensions — the conventions define no cost attribute yet.

use iii_helpers::observability::opentelemetry::trace::{Status, TraceContextExt as _};
use iii_helpers::observability::opentelemetry::{Context, KeyValue};

use crate::types::events::{StopReason, Usage};
use crate::types::router::ErrorShape;

/// Wire value of the terminal stop reason (matches `StopReason`'s serde
/// snake_case encoding).
fn finish_reason(stop: StopReason) -> &'static str {
    match stop {
        StopReason::End => "end",
        StopReason::Length => "length",
        StopReason::FunctionCall => "function_call",
        StopReason::Aborted => "aborted",
        StopReason::Error => "error",
    }
}

/// Build the `gen_ai.*` attribute list for one finished LLM call. Token
/// counts stay numeric (i64) and cost stays f64 so the trace store keeps
/// them queryable; absent usage fields are omitted, never zeroed.
pub fn genai_attributes(
    provider: &str,
    model: &str,
    stop_reason: Option<StopReason>,
    usage: Option<&Usage>,
) -> Vec<KeyValue> {
    let mut attrs = vec![
        KeyValue::new("gen_ai.system", provider.to_string()),
        KeyValue::new("gen_ai.request.model", model.to_string()),
    ];
    if let Some(stop) = stop_reason {
        attrs.push(KeyValue::new(
            "gen_ai.response.finish_reason",
            finish_reason(stop),
        ));
    }
    if let Some(u) = usage {
        let tokens: [(&str, Option<u64>); 5] = [
            ("gen_ai.usage.input_tokens", u.input),
            ("gen_ai.usage.output_tokens", u.output),
            ("gen_ai.usage.cache_read_tokens", u.cache_read),
            ("gen_ai.usage.cache_write_tokens", u.cache_write),
            ("gen_ai.usage.reasoning_tokens", u.reasoning),
        ];
        for (key, value) in tokens {
            if let Some(v) = value {
                attrs.push(KeyValue::new(key, v as i64));
            }
        }
        if let Some(cost) = u.cost_usd {
            attrs.push(KeyValue::new("gen_ai.usage.cost_usd", cost));
        }
    }
    attrs
}

/// Stamp one finished LLM call onto the current span. No-op when no span is
/// recording (OTel disabled, unit tests).
pub fn record_llm_call(
    provider: &str,
    model: &str,
    stop_reason: Option<StopReason>,
    usage: Option<&Usage>,
    error: Option<&ErrorShape>,
) {
    let cx = Context::current();
    let span = cx.span();
    if !span.span_context().is_valid() {
        return;
    }
    for attr in genai_attributes(provider, model, stop_reason, usage) {
        span.set_attribute(attr);
    }
    if stop_reason == Some(StopReason::Error) {
        let error_type = error.map(|e| e.code.as_str()).unwrap_or("llm.stream_error");
        let error_message = error
            .map(|e| e.message.as_str())
            .unwrap_or("LLM stream ended with an error");
        span.set_attribute(KeyValue::new("error.type", error_type.to_string()));
        span.set_attribute(KeyValue::new("error.message", error_message.to_string()));
        span.set_attribute(KeyValue::new("iii.tag.outcome", "failed"));
        span.set_status(Status::error(error_message.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iii_helpers::observability::opentelemetry::Value;

    fn get<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a Value> {
        attrs
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| &kv.value)
    }

    #[test]
    fn full_usage_maps_to_numeric_genai_attributes() {
        let usage = Usage {
            input: Some(1240),
            output: Some(342),
            cache_read: Some(512),
            cache_write: Some(64),
            reasoning: Some(128),
            cost_usd: Some(0.0421),
        };
        let attrs = genai_attributes(
            "anthropic",
            "claude-opus-4",
            Some(StopReason::End),
            Some(&usage),
        );

        assert_eq!(
            get(&attrs, "gen_ai.system"),
            Some(&Value::String("anthropic".into()))
        );
        assert_eq!(
            get(&attrs, "gen_ai.request.model"),
            Some(&Value::String("claude-opus-4".into()))
        );
        assert_eq!(
            get(&attrs, "gen_ai.response.finish_reason"),
            Some(&Value::String("end".into()))
        );
        assert_eq!(
            get(&attrs, "gen_ai.usage.input_tokens"),
            Some(&Value::I64(1240))
        );
        assert_eq!(
            get(&attrs, "gen_ai.usage.output_tokens"),
            Some(&Value::I64(342))
        );
        assert_eq!(
            get(&attrs, "gen_ai.usage.cache_read_tokens"),
            Some(&Value::I64(512))
        );
        assert_eq!(
            get(&attrs, "gen_ai.usage.cache_write_tokens"),
            Some(&Value::I64(64))
        );
        assert_eq!(
            get(&attrs, "gen_ai.usage.reasoning_tokens"),
            Some(&Value::I64(128))
        );
        assert_eq!(
            get(&attrs, "gen_ai.usage.cost_usd"),
            Some(&Value::F64(0.0421))
        );
    }

    #[test]
    fn absent_usage_fields_are_omitted_not_zeroed() {
        let usage = Usage {
            input: Some(9),
            ..Default::default()
        };
        let attrs = genai_attributes("openai", "gpt-5", None, Some(&usage));
        assert_eq!(
            get(&attrs, "gen_ai.usage.input_tokens"),
            Some(&Value::I64(9))
        );
        assert!(get(&attrs, "gen_ai.usage.output_tokens").is_none());
        assert!(get(&attrs, "gen_ai.usage.cost_usd").is_none());
        assert!(get(&attrs, "gen_ai.response.finish_reason").is_none());
    }

    #[test]
    fn no_usage_still_records_system_and_model() {
        let attrs = genai_attributes("openai", "gpt-5", Some(StopReason::Aborted), None);
        assert_eq!(attrs.len(), 3);
        assert_eq!(
            get(&attrs, "gen_ai.response.finish_reason"),
            Some(&Value::String("aborted".into()))
        );
    }

    #[test]
    fn finish_reason_matches_wire_encoding() {
        for (stop, want) in [
            (StopReason::End, "end"),
            (StopReason::Length, "length"),
            (StopReason::FunctionCall, "function_call"),
            (StopReason::Aborted, "aborted"),
            (StopReason::Error, "error"),
        ] {
            assert_eq!(finish_reason(stop), want);
        }
    }

    #[test]
    fn record_llm_call_is_safe_without_a_span() {
        record_llm_call("openai", "gpt-5", Some(StopReason::End), None, None);
    }
}
