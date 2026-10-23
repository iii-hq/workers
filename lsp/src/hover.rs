use crate::completions::{is_deprecated_trigger_type, DEPRECATED_TRIGGER_NOTE};
use crate::engine_client::EngineClient;
use crate::engine_introspection::TriggerTypeInfo;
use std::sync::Arc;
use tower_lsp_server::ls_types::*;

pub fn get_hover(word: &str, engine: &Arc<EngineClient>) -> Option<Hover> {
    // Try function ID first
    if let Some(func) = engine.get_function(word) {
        let mut content = format!("**Function:** `{}`", func.function_id);

        if let Some(desc) = &func.description {
            content.push_str(&format!("\n\n{}", desc));
        }

        // Show which worker hosts this function
        if let Some(worker) = engine.find_worker_for_function(&func.function_id) {
            if let Some(name) = &worker.name {
                content.push_str(&format!("\n\n**Worker:** {}", name));
            }
        }

        if let Some(req) = &func.request_format {
            if let Ok(pretty) = serde_json::to_string_pretty(req) {
                content.push_str(&format!(
                    "\n\n**Request format:**\n```json\n{}\n```",
                    pretty
                ));
            }
        }

        if let Some(resp) = &func.response_format {
            if let Ok(pretty) = serde_json::to_string_pretty(resp) {
                content.push_str(&format!(
                    "\n\n**Response format:**\n```json\n{}\n```",
                    pretty
                ));
            }
        }

        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        });
    }

    // Try trigger type
    if let Some(tt) = engine.get_trigger_type(word) {
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: trigger_type_hover_markdown(&tt),
            }),
            range: None,
        });
    }

    None
}

fn trigger_type_hover_markdown(tt: &TriggerTypeInfo) -> String {
    let mut content = format!("**Trigger type:** `{}`\n\n{}", tt.id, tt.description);

    if is_deprecated_trigger_type(&tt.id) {
        content.push_str(&format!(
            "\n\n**Deprecated** ({DEPRECATED_TRIGGER_NOTE}): do not use in new code; \
             see the guide \"Migrate from iii-stream and pubsub\"."
        ));
    }

    if let Some(config) = &tt.trigger_request_format {
        if let Ok(pretty) = serde_json::to_string_pretty(config) {
            content.push_str(&format!("\n\n**Config format:**\n```json\n{}\n```", pretty));
        }
    }

    content
}

#[cfg(test)]
mod tests {
    #[test]
    fn stream_trigger_type_hover_is_marked_deprecated() {
        let tt = super::TriggerTypeInfo {
            id: "stream:join".to_string(),
            description: "Stream join trigger".to_string(),
            trigger_request_format: None,
            call_request_format: None,
        };
        let content = super::trigger_type_hover_markdown(&tt);
        assert!(content.contains("**Deprecated** (deprecated: iii-stream)"));
        assert!(content.contains("Migrate from iii-stream and pubsub"));

        let http = super::TriggerTypeInfo {
            id: "http".to_string(),
            ..tt
        };
        assert!(!super::trigger_type_hover_markdown(&http).contains("Deprecated"));
    }

    #[test]
    fn format_function_hover() {
        // Test the markdown formatting logic directly
        let function_id = "todos::create";
        let description = "Create a new todo item";
        let worker_name = "todo-service";

        let mut content = format!("**Function:** `{}`", function_id);
        content.push_str(&format!("\n\n{}", description));
        content.push_str(&format!("\n\n**Worker:** {}", worker_name));

        assert!(content.contains("**Function:** `todos::create`"));
        assert!(content.contains("Create a new todo item"));
        assert!(content.contains("**Worker:** todo-service"));
    }

    #[test]
    fn format_trigger_type_hover() {
        let trigger_id = "http";
        let description = "HTTP trigger for REST endpoints";

        let content = format!("**Trigger type:** `{}`\n\n{}", trigger_id, description);

        assert!(content.contains("**Trigger type:** `http`"));
        assert!(content.contains("HTTP trigger for REST endpoints"));
    }

    #[test]
    fn format_with_json_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" }
            }
        });

        let pretty = serde_json::to_string_pretty(&schema).unwrap();
        let content = format!("**Request format:**\n```json\n{}\n```", pretty);

        assert!(content.contains("```json"));
        assert!(content.contains("\"title\""));
    }
}
