use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{FunctionInfo, IIIError, TriggerInfo, III};
use serde_json::{json, Value};

pub fn build_handler(
    iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        Box::pin(async move { handle(&iii, payload).await })
    }
}

pub async fn handle(iii: &III, payload: Value) -> Result<Value, IIIError> {
    let function_id = payload
        .get("function_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let worker_name = payload
        .get("worker_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if function_id.is_none() && worker_name.is_none() {
        return Err(IIIError::Handler(
            "provide either function_id or worker_name".to_string(),
        ));
    }

    let (functions_result, workers_result, triggers_result) = tokio::join!(
        iii.list_functions(),
        iii.list_workers(),
        iii.list_triggers(false),
    );

    let functions = functions_result?;
    let workers = workers_result?;
    let triggers = triggers_result?;

    let func_map: HashMap<String, &FunctionInfo> = functions
        .iter()
        .map(|f| (f.function_id.clone(), f))
        .collect();

    let triggers_by_function: HashMap<String, Vec<&TriggerInfo>> = {
        let mut map: HashMap<String, Vec<&TriggerInfo>> = HashMap::new();
        for t in &triggers {
            map.entry(t.function_id.clone()).or_default().push(t);
        }
        map
    };

    let func_to_worker: HashMap<String, String> = {
        let mut map = HashMap::new();
        for w in &workers {
            if let Some(name) = &w.name {
                for f in &w.functions {
                    map.insert(f.clone(), name.clone());
                }
            }
        }
        map
    };

    if let Some(ref fid) = function_id {
        let func = func_map
            .get(fid.as_str())
            .ok_or_else(|| IIIError::Handler(format!("function '{}' not found", fid)))?;

        let func_triggers = triggers_by_function
            .get(fid.as_str())
            .cloned()
            .unwrap_or_default();

        let worker = func_to_worker
            .get(fid.as_str())
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());

        return Ok(build_function_explanation(func, &func_triggers, &worker, &triggers));
    }

    if let Some(ref wname) = worker_name {
        let worker_info = workers
            .iter()
            .find(|w| w.name.as_deref() == Some(wname.as_str()))
            .ok_or_else(|| IIIError::Handler(format!("worker '{}' not found", wname)))?;

        let worker_functions: Vec<&FunctionInfo> = worker_info
            .functions
            .iter()
            .filter_map(|fid| func_map.get(fid.as_str()).copied())
            .collect();

        let mut function_explanations: Vec<Value> = Vec::new();
        for func in &worker_functions {
            let func_triggers = triggers_by_function
                .get(func.function_id.as_str())
                .cloned()
                .unwrap_or_default();

            function_explanations.push(build_function_explanation(
                func,
                &func_triggers,
                wname,
                &triggers,
            ));
        }

        let worker_summary = format!(
            "Worker '{}' hosts {} function(s). {}",
            wname,
            worker_functions.len(),
            if worker_functions.is_empty() {
                "It has no registered functions.".to_string()
            } else {
                let names: Vec<&str> = worker_functions
                    .iter()
                    .map(|f| f.function_id.as_str())
                    .collect();
                format!("Functions: {}", names.join(", "))
            }
        );

        return Ok(json!({
            "worker": wname,
            "summary": worker_summary,
            "function_count": worker_functions.len(),
            "functions": function_explanations
        }));
    }

    unreachable!()
}

fn describe_trigger(trigger: &TriggerInfo) -> String {
    match trigger.trigger_type.as_str() {
        "http" => {
            let method = trigger
                .config
                .get("http_method")
                .and_then(|v| v.as_str())
                .unwrap_or("POST");
            let path = trigger
                .config
                .get("api_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            format!("HTTP {} /{}", method, path)
        }
        "cron" => {
            let expr = trigger
                .config
                .get("expression")
                .or_else(|| trigger.config.get("cron"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown schedule");
            format!("Cron schedule: {}", expr)
        }
        "subscribe" => {
            let topic = trigger
                .config
                .get("topic")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown topic");
            format!("Subscribes to topic '{}'", topic)
        }
        "state" => {
            let scope = trigger
                .config
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            format!("Triggered by state change in scope '{}'", scope)
        }
        other => format!("Trigger type: {}", other),
    }
}

fn describe_schema_fields(schema: &Value) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
        let required: Vec<&str> = schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        for (key, val) in props {
            let type_str = val
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("any");
            let suffix = if required.contains(&key.as_str()) {
                " (required)"
            } else {
                ""
            };
            fields.insert(key.clone(), format!("{}{}", type_str, suffix));
        }
    }
    fields
}

fn build_function_explanation(
    func: &FunctionInfo,
    func_triggers: &[&TriggerInfo],
    worker: &str,
    all_triggers: &[TriggerInfo],
) -> Value {
    let description = func
        .description
        .as_deref()
        .unwrap_or("No description available");

    let trigger_descriptions: Vec<String> = func_triggers.iter().map(|t| describe_trigger(t)).collect();

    let trigger_details: Vec<Value> = func_triggers
        .iter()
        .map(|t| {
            json!({
                "type": t.trigger_type,
                "config": t.config
            })
        })
        .collect();

    let inputs = func
        .request_format
        .as_ref()
        .map(|s| describe_schema_fields(s))
        .unwrap_or_default();

    let outputs = func
        .response_format
        .as_ref()
        .map(|s| describe_schema_fields(s))
        .unwrap_or_default();

    let inbound: Vec<String> = all_triggers
        .iter()
        .filter(|t| t.trigger_type == "subscribe")
        .filter(|t| {
            let topic = t.config.get("topic").and_then(|v| v.as_str());
            let our_topics: Vec<&str> = func_triggers
                .iter()
                .filter(|ft| ft.trigger_type == "subscribe")
                .filter_map(|ft| ft.config.get("topic").and_then(|v| v.as_str()))
                .collect();
            if let Some(topic) = topic {
                our_topics.contains(&topic) && t.function_id != func.function_id
            } else {
                false
            }
        })
        .map(|t| t.function_id.clone())
        .collect();

    let mut explanation_parts: Vec<String> = Vec::new();

    explanation_parts.push(format!(
        "{} {}.",
        func.function_id, description.to_lowercase()
    ));

    if !trigger_descriptions.is_empty() {
        explanation_parts.push(format!(
            "It is triggered via: {}.",
            trigger_descriptions.join("; ")
        ));
    } else {
        explanation_parts.push("It has no registered triggers (invoked directly).".to_string());
    }

    if !inputs.is_empty() {
        let input_strs: Vec<String> = inputs
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect();
        explanation_parts.push(format!("Takes as input: {}.", input_strs.join(", ")));
    }

    if !outputs.is_empty() {
        let output_strs: Vec<String> = outputs
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect();
        explanation_parts.push(format!("Returns: {}.", output_strs.join(", ")));
    }

    if !inbound.is_empty() {
        explanation_parts.push(format!(
            "Connected to: {}.",
            inbound.join(", ")
        ));
    }

    explanation_parts.push(format!("Hosted on worker '{}'.", worker));

    let explanation = explanation_parts.join(" ");

    json!({
        "explanation": explanation,
        "function_id": func.function_id,
        "worker": worker,
        "triggers": trigger_details,
        "inputs": inputs,
        "outputs": outputs
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_describe_trigger_http() {
        let trigger = TriggerInfo {
            id: "t1".to_string(),
            trigger_type: "http".to_string(),
            function_id: "eval::metrics".to_string(),
            config: json!({ "api_path": "eval/metrics", "http_method": "POST" }),
        };
        let desc = describe_trigger(&trigger);
        assert_eq!(desc, "HTTP POST /eval/metrics");
    }

    #[test]
    fn test_describe_trigger_cron() {
        let trigger = TriggerInfo {
            id: "t2".to_string(),
            trigger_type: "cron".to_string(),
            function_id: "eval::drift".to_string(),
            config: json!({ "expression": "0 */10 * * * *" }),
        };
        let desc = describe_trigger(&trigger);
        assert_eq!(desc, "Cron schedule: 0 */10 * * * *");
    }

    #[test]
    fn test_describe_trigger_subscribe() {
        let trigger = TriggerInfo {
            id: "t3".to_string(),
            trigger_type: "subscribe".to_string(),
            function_id: "eval::ingest".to_string(),
            config: json!({ "topic": "telemetry.spans" }),
        };
        let desc = describe_trigger(&trigger);
        assert_eq!(desc, "Subscribes to topic 'telemetry.spans'");
    }

    #[test]
    fn test_describe_trigger_state() {
        let trigger = TriggerInfo {
            id: "t4".to_string(),
            trigger_type: "state".to_string(),
            function_id: "some::fn".to_string(),
            config: json!({ "scope": "eval:spans" }),
        };
        let desc = describe_trigger(&trigger);
        assert_eq!(desc, "Triggered by state change in scope 'eval:spans'");
    }

    #[test]
    fn test_describe_trigger_unknown() {
        let trigger = TriggerInfo {
            id: "t5".to_string(),
            trigger_type: "custom".to_string(),
            function_id: "some::fn".to_string(),
            config: json!({}),
        };
        let desc = describe_trigger(&trigger);
        assert_eq!(desc, "Trigger type: custom");
    }

    #[test]
    fn test_describe_schema_fields() {
        let schema = json!({
            "type": "object",
            "properties": {
                "function_id": { "type": "string" },
                "limit": { "type": "integer" }
            },
            "required": ["function_id"]
        });
        let fields = describe_schema_fields(&schema);
        assert_eq!(fields.get("function_id").unwrap(), "string (required)");
        assert_eq!(fields.get("limit").unwrap(), "integer");
    }

    #[test]
    fn test_describe_schema_fields_empty() {
        let schema = json!({ "type": "object" });
        let fields = describe_schema_fields(&schema);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_build_function_explanation() {
        let func = FunctionInfo {
            function_id: "eval::metrics".to_string(),
            description: Some("Calculate metrics for a tracked function".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string" }
                },
                "required": ["function_id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "p50_ms": { "type": "integer" }
                }
            })),
            metadata: None,
        };

        let trigger = TriggerInfo {
            id: "t1".to_string(),
            trigger_type: "http".to_string(),
            function_id: "eval::metrics".to_string(),
            config: json!({ "api_path": "eval/metrics", "http_method": "POST" }),
        };

        let result = build_function_explanation(&func, &[&trigger], "iii-eval", &[]);
        assert!(result.get("explanation").is_some());
        let explanation = result["explanation"].as_str().unwrap();
        assert!(explanation.contains("eval::metrics"));
        assert!(explanation.contains("HTTP POST"));
        assert!(explanation.contains("iii-eval"));
        assert_eq!(result["function_id"], "eval::metrics");
        assert_eq!(result["worker"], "iii-eval");
    }
}
