use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{FunctionInfo, IIIError, TriggerInfo, III};
use serde_json::Value;

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

    let trigger_id = payload
        .get("trigger_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if function_id.is_none() && trigger_id.is_none() {
        return Err(IIIError::Handler(
            "provide either function_id or trigger_id".to_string(),
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

    let root_function_id = if let Some(fid) = function_id {
        if !func_map.contains_key(&fid) {
            return Err(IIIError::Handler(format!(
                "function '{}' not found",
                fid
            )));
        }
        fid
    } else if let Some(tid) = trigger_id {
        let trigger = triggers
            .iter()
            .find(|t| t.id == tid)
            .ok_or_else(|| IIIError::Handler(format!("trigger '{}' not found", tid)))?;
        trigger.function_id.clone()
    } else {
        unreachable!()
    };

    let mut chain: Vec<Value> = Vec::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut queue: Vec<String> = vec![root_function_id.clone()];
    let mut step = 0u32;

    while let Some(current_fid) = queue.pop() {
        if visited.contains(&current_fid) {
            continue;
        }
        visited.insert(current_fid.clone());
        step += 1;

        let func_info = func_map.get(&current_fid);
        let func_triggers = triggers_by_function
            .get(&current_fid)
            .cloned()
            .unwrap_or_default();

        let worker_name = func_to_worker
            .get(&current_fid)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());

        let trigger_descriptions: Vec<Value> = func_triggers
            .iter()
            .map(|t| {
                serde_json::json!({
                    "trigger_id": t.id,
                    "trigger_type": t.trigger_type,
                    "config": t.config,
                })
            })
            .collect();

        let description = func_info
            .and_then(|f| f.description.as_deref())
            .unwrap_or("");

        chain.push(serde_json::json!({
            "step": step,
            "function_id": current_fid,
            "worker": worker_name,
            "description": description,
            "triggers": trigger_descriptions,
            "inputs": func_info.and_then(|f| f.request_format.as_ref()),
            "outputs": func_info.and_then(|f| f.response_format.as_ref()),
        }));

        for t in &func_triggers {
            if t.trigger_type == "subscribe" {
                if let Some(topic) = t.config.get("topic").and_then(|v| v.as_str()) {
                    for other_t in &triggers {
                        if other_t.trigger_type == "subscribe"
                            && other_t
                                .config
                                .get("topic")
                                .and_then(|v| v.as_str())
                                == Some(topic)
                            && other_t.function_id != current_fid
                        {
                            queue.push(other_t.function_id.clone());
                        }
                    }
                }
            }
        }
    }

    let diagram = build_trace_mermaid(&chain, &triggers_by_function);

    Ok(serde_json::json!({
        "function_id": chain.first().and_then(|c| c.get("function_id")).and_then(|v| v.as_str()).unwrap_or(""),
        "chain": chain,
        "diagram": diagram,
    }))
}

fn sanitize_id(id: &str) -> String {
    id.replace("::", "_")
        .replace('-', "_")
        .replace('.', "_")
        .replace(' ', "_")
}

fn build_trace_mermaid(
    chain: &[Value],
    triggers_by_function: &HashMap<String, Vec<&TriggerInfo>>,
) -> String {
    let mut diagram = String::from("graph TD\n");

    for entry in chain {
        let fid = entry
            .get("function_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let worker = entry
            .get("worker")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let safe_fid = sanitize_id(fid);

        diagram.push_str(&format!(
            "    {}[\"{}\\n({})\"]",
            safe_fid, fid, worker
        ));
        diagram.push('\n');

        if let Some(triggers) = triggers_by_function.get(fid) {
            for t in triggers {
                let trigger_safe = sanitize_id(&t.id);
                let label = &t.trigger_type;
                diagram.push_str(&format!(
                    "    {}{{\"{}\"}} -->|{}| {}\n",
                    trigger_safe, label, label, safe_fid
                ));
            }
        }
    }

    diagram
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_id() {
        assert_eq!(sanitize_id("introspect::functions"), "introspect_functions");
    }

    #[test]
    fn test_build_trace_mermaid_empty() {
        let triggers: HashMap<String, Vec<&TriggerInfo>> = HashMap::new();
        let result = build_trace_mermaid(&[], &triggers);
        assert_eq!(result, "graph TD\n");
    }
}
