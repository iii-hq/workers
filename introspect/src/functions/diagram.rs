use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{FunctionInfo, IIIError, TriggerInfo, WorkerInfo, III};
use serde_json::Value;

pub fn build_handler(
    iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();
        Box::pin(async move { handle(&iii).await })
    }
}

pub async fn handle(iii: &III) -> Result<Value, IIIError> {
    let (functions_result, workers_result, triggers_result) = tokio::join!(
        iii.list_functions(),
        iii.list_workers(),
        iii.list_triggers(false),
    );

    let functions = functions_result?;
    let workers = workers_result?;
    let triggers = triggers_result?;

    let content = generate_mermaid(&functions, &workers, &triggers);

    Ok(serde_json::json!({
        "format": "mermaid",
        "content": content
    }))
}

fn sanitize_id(id: &str) -> String {
    id.replace("::", "_")
        .replace('-', "_")
        .replace('.', "_")
        .replace(' ', "_")
}

const ENGINE_INTERNAL_PREFIXES: &[&str] = &[
    "state::", "stream::", "engine::", "iii::",
];

const ENGINE_INTERNAL_EXACT: &[&str] = &[
    "publish", "iii::durable::publish",
];

fn is_engine_internal(function_id: &str) -> bool {
    if ENGINE_INTERNAL_PREFIXES
        .iter()
        .any(|prefix| function_id.starts_with(prefix))
    {
        return true;
    }
    if ENGINE_INTERNAL_EXACT.contains(&function_id) {
        return true;
    }
    if function_id.starts_with("iii.on_functions_available") {
        return true;
    }
    false
}

fn generate_mermaid(
    functions: &[FunctionInfo],
    workers: &[WorkerInfo],
    triggers: &[TriggerInfo],
) -> String {
    let named_workers: Vec<&WorkerInfo> = workers
        .iter()
        .filter(|w| w.name.is_some() || w.function_count > 0)
        .collect();

    let user_functions: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|f| !is_engine_internal(&f.function_id))
        .collect();

    let user_triggers: Vec<&TriggerInfo> = triggers
        .iter()
        .filter(|t| !is_engine_internal(&t.function_id))
        .collect();

    let total_nodes = user_functions.len() + user_triggers.len();

    if total_nodes > 30 {
        return generate_summary_diagram(&named_workers, user_functions.len(), user_triggers.len());
    }

    let mut diagram = String::from("graph TD\n");

    for w in &named_workers {
        let user_fns: Vec<&String> = w
            .functions
            .iter()
            .filter(|f| !is_engine_internal(f))
            .collect();

        if user_fns.is_empty() {
            continue;
        }

        let worker_name = w.name.clone().unwrap_or_else(|| w.id.clone());
        let safe_id = sanitize_id(&w.id);
        diagram.push_str(&format!("    subgraph {}[\"{}\"]\n", safe_id, worker_name));

        for f in user_fns {
            let func_safe = sanitize_id(f);
            diagram.push_str(&format!("        {}[\"{}\"]\n", func_safe, f));
        }

        diagram.push_str("    end\n");
    }

    for t in &user_triggers {
        let trigger_safe = sanitize_id(&t.id);
        let func_safe = sanitize_id(&t.function_id);
        let label = &t.trigger_type;
        diagram.push_str(&format!(
            "    {}{{\"{}\"}} -->|{}| {}\n",
            trigger_safe, t.id, label, func_safe
        ));
    }

    diagram
}

fn generate_summary_diagram(
    workers: &[&WorkerInfo],
    total_functions: usize,
    total_triggers: usize,
) -> String {
    let mut diagram = String::from("graph TD\n");
    diagram.push_str(&format!(
        "    summary[\"System Summary: {} functions, {} triggers\"]\n",
        total_functions, total_triggers
    ));

    for w in workers {
        let worker_name = w.name.clone().unwrap_or_else(|| w.id.clone());
        let safe_id = sanitize_id(&w.id);
        let user_fn_count = w
            .functions
            .iter()
            .filter(|f| !is_engine_internal(f))
            .count();

        if user_fn_count == 0 {
            continue;
        }

        diagram.push_str(&format!(
            "    {}[\"{}\\n({} functions)\"]\n",
            safe_id, worker_name, user_fn_count
        ));
        diagram.push_str(&format!("    summary --> {}\n", safe_id));
    }

    diagram
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_mermaid_empty() {
        let result = generate_mermaid(&[], &[], &[]);
        assert_eq!(result, "graph TD\n");
    }

    #[test]
    fn test_generate_mermaid_with_worker_and_function() {
        let workers = vec![WorkerInfo {
            id: "w1".to_string(),
            name: Some("my-worker".to_string()),
            runtime: None,
            version: None,
            os: None,
            ip_address: None,
            status: "connected".to_string(),
            connected_at_ms: 0,
            function_count: 1,
            functions: vec!["test::echo".to_string()],
            active_invocations: 0,
        }];
        let functions = vec![FunctionInfo {
            function_id: "test::echo".to_string(),
            description: None,
            request_format: None,
            response_format: None,
            metadata: None,
        }];
        let triggers = vec![TriggerInfo {
            id: "t1".to_string(),
            trigger_type: "http".to_string(),
            function_id: "test::echo".to_string(),
            config: serde_json::json!({}),
            metadata: None,
        }];

        let result = generate_mermaid(&functions, &workers, &triggers);
        assert!(result.contains("graph TD"));
        assert!(result.contains("my-worker"));
        assert!(result.contains("test_echo"));
        assert!(result.contains("http"));
    }

    #[test]
    fn test_engine_internals_are_filtered() {
        let workers = vec![WorkerInfo {
            id: "w1".to_string(),
            name: Some("my-worker".to_string()),
            runtime: None,
            version: None,
            os: None,
            ip_address: None,
            status: "connected".to_string(),
            connected_at_ms: 0,
            function_count: 3,
            functions: vec![
                "test::echo".to_string(),
                "state::get".to_string(),
                "stream::set".to_string(),
            ],
            active_invocations: 0,
        }];
        let functions = vec![
            FunctionInfo {
                function_id: "test::echo".to_string(),
                description: None,
                request_format: None,
                response_format: None,
                metadata: None,
            },
            FunctionInfo {
                function_id: "state::get".to_string(),
                description: None,
                request_format: None,
                response_format: None,
                metadata: None,
            },
            FunctionInfo {
                function_id: "stream::set".to_string(),
                description: None,
                request_format: None,
                response_format: None,
                metadata: None,
            },
        ];

        let result = generate_mermaid(&functions, &workers, &[]);
        assert!(result.contains("test_echo"));
        assert!(!result.contains("state_get"));
        assert!(!result.contains("stream_set"));
        assert!(!result.contains("Unassigned"));
    }

    #[test]
    fn test_is_engine_internal() {
        assert!(is_engine_internal("state::get"));
        assert!(is_engine_internal("state::set"));
        assert!(is_engine_internal("stream::set"));
        assert!(is_engine_internal("engine::health"));
        assert!(is_engine_internal("iii::config"));
        assert!(is_engine_internal("publish"));
        assert!(is_engine_internal("iii::durable::publish"));
        assert!(is_engine_internal("iii.on_functions_available.abc"));
        assert!(!is_engine_internal("eval::metrics"));
        assert!(!is_engine_internal("introspect::functions"));
        assert!(!is_engine_internal("agent::chat"));
    }

    #[test]
    fn test_sanitize_id() {
        assert_eq!(sanitize_id("test::echo"), "test_echo");
        assert_eq!(sanitize_id("my-worker"), "my_worker");
        assert_eq!(sanitize_id("a.b.c"), "a_b_c");
    }
}
