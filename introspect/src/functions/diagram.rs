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

// Short stable digest appended to node IDs so two IDs that would collapse
// under character normalization (e.g. "foo::bar" and "foo--bar") still get
// distinct Mermaid nodes. DefaultHasher is deterministic within a single
// process for the same input, which is all Mermaid output needs.
fn id_digest(raw: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    raw.hash(&mut h);
    format!("{:x}", h.finish())
}

fn sanitize_id_kind(kind: &str, id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{}_{}_{}", kind, safe, &id_digest(id)[..8])
}

fn fn_node_id(id: &str) -> String {
    sanitize_id_kind("fn", id)
}

fn worker_node_id(id: &str) -> String {
    sanitize_id_kind("worker", id)
}

fn trigger_node_id(id: &str) -> String {
    sanitize_id_kind("trigger", id)
}

// Escape a user-supplied string for safe placement inside Mermaid quoted
// labels. Mermaid breaks on literal double-quotes, backticks, brackets,
// pipes, and newlines inside "..." labels.
fn mermaid_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("&quot;"),
            '\\' => out.push_str("&#92;"),
            '`' => out.push_str("&#96;"),
            '[' => out.push_str("&#91;"),
            ']' => out.push_str("&#93;"),
            '{' => out.push_str("&#123;"),
            '}' => out.push_str("&#125;"),
            '|' => out.push_str("&#124;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' | '\r' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

const ENGINE_INTERNAL_PREFIXES: &[&str] = &["state::", "stream::", "engine::", "iii::"];

const ENGINE_INTERNAL_EXACT: &[&str] = &["publish", "iii::durable::publish"];

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
        let safe_id = worker_node_id(&w.id);
        diagram.push_str(&format!(
            "    subgraph {}[\"{}\"]\n",
            safe_id,
            mermaid_label(&worker_name)
        ));

        for f in user_fns {
            let func_safe = fn_node_id(f);
            diagram.push_str(&format!(
                "        {}[\"{}\"]\n",
                func_safe,
                mermaid_label(f)
            ));
        }

        diagram.push_str("    end\n");
    }

    for t in &user_triggers {
        let trigger_safe = trigger_node_id(&t.id);
        let func_safe = fn_node_id(&t.function_id);
        diagram.push_str(&format!(
            "    {}{{\"{}\"}} -->|{}| {}\n",
            trigger_safe,
            mermaid_label(&t.id),
            mermaid_label(&t.trigger_type),
            func_safe
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
        let safe_id = worker_node_id(&w.id);
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
            safe_id,
            mermaid_label(&worker_name),
            user_fn_count
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
            isolation: None,
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
        // Type-prefixed node id (fn_…) and visible label (`test::echo`)
        // both must appear.
        assert!(result.contains("fn_test"));
        assert!(result.contains("test::echo"));
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
            isolation: None,
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
        // User function is present; engine-internal state/stream ones are
        // filtered. Check against the visible Mermaid labels (raw ids),
        // since node ids are now type-prefixed + hashed.
        assert!(result.contains("test::echo"));
        assert!(!result.contains("state::get"));
        assert!(!result.contains("stream::set"));
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
    fn node_ids_are_type_prefixed_and_collision_safe() {
        // Different raw IDs that collapse under simple char normalization
        // must still produce distinct node IDs.
        let a = fn_node_id("foo::bar");
        let b = fn_node_id("foo--bar");
        assert_ne!(a, b);
        assert!(a.starts_with("fn_"));
        assert!(b.starts_with("fn_"));

        let w = worker_node_id("w1");
        assert!(w.starts_with("worker_"));

        let t = trigger_node_id("t1");
        assert!(t.starts_with("trigger_"));
    }

    #[test]
    fn mermaid_label_escapes_breakers() {
        assert_eq!(mermaid_label("hi"), "hi");
        assert_eq!(mermaid_label("a\"b"), "a&quot;b");
        let bad = mermaid_label("x[y]|z\nfoo");
        assert!(!bad.contains('['));
        assert!(!bad.contains(']'));
        assert!(!bad.contains('|'));
        assert!(!bad.contains('\n'));
    }
}
