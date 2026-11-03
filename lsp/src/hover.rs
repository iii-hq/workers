use crate::engine_client::{engine_source, EngineClient};
use crate::engine_introspection::FunctionInfo;
use std::sync::Arc;
use tower_lsp_server::ls_types::*;

/// How many observer targets a hover lists before summarizing the rest.
const MAX_LISTED_TARGETS: usize = 5;

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

        content.push_str(&observe_markdown(
            &func,
            &engine.observer_targets(&func.function_id),
        ));

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

        if let Some(args) = &func.args_format {
            if let Ok(pretty) = serde_json::to_string_pretty(args) {
                content.push_str(&format!("\n\n**Args format:**\n```json\n{}\n```", pretty));
            }
        }

        return Some(markdown_hover(content));
    }

    // Try an engine-hosted source
    if let Some(description) = engine_source(word) {
        return Some(markdown_hover(format!(
            "**Engine source:** `{word}`\n\n{description}"
        )));
    }

    None
}

fn markdown_hover(value: String) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    }
}

/// What a function says about calling and observing it: private, not
/// observable, and who observes it.
fn observe_markdown(func: &FunctionInfo, targets: &[String]) -> String {
    let mut content = String::new();
    if func.private {
        content.push_str(
            "\n\n**Private:** only its worker calls it; observe it to receive its calls.",
        );
    }
    if !func.observable {
        content.push_str("\n\n**Not observable.**");
    }
    if func.observer_count > 0 {
        let mut line = format!("\n\n**Observers:** {}", func.observer_count);
        if !targets.is_empty() {
            let listed: Vec<String> = targets
                .iter()
                .take(MAX_LISTED_TARGETS)
                .map(|target| format!("`{target}`"))
                .collect();
            line.push_str(&format!(" (targets {}", listed.join(", ")));
            if targets.len() > MAX_LISTED_TARGETS {
                line.push_str(&format!(", {} more", targets.len() - MAX_LISTED_TARGETS));
            }
            line.push(')');
        }
        content.push_str(&line);
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn observe_markdown_of_a_private_observed_source() {
        let func = FunctionInfo {
            function_id: "cron::on-tick".to_string(),
            private: true,
            observer_count: 7,
            ..Default::default()
        };
        let targets: Vec<String> = (1..=7).map(|n| format!("jobs::run-{n}")).collect();
        let content = observe_markdown(&func, &targets);
        assert!(content.contains("**Private:** only its worker calls it"));
        assert!(content.contains("**Observers:** 7 (targets `jobs::run-1`"));
        assert!(content.contains("`jobs::run-5`, 2 more)"));
        assert!(!content.contains("jobs::run-6"));
        assert!(!content.contains("Not observable"));
    }

    #[test]
    fn observe_markdown_of_an_unobserved_function() {
        let func = FunctionInfo {
            function_id: "secrets::resolve".to_string(),
            observable: false,
            ..Default::default()
        };
        let content = observe_markdown(&func, &[]);
        assert_eq!(content, "\n\n**Not observable.**");
        assert!(observe_markdown(&FunctionInfo::default(), &[]).is_empty());
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
