use serde_json::{Value, json};

pub fn list() -> Value {
    json!({
        "prompts": [
            { "name": "register-function", "description": "Guide to register a function", "arguments": [
                { "name": "language", "description": "node or python", "required": true },
                { "name": "function_id", "description": "e.g. myservice::process", "required": true }
            ]},
            { "name": "build-api", "description": "Expose a function as HTTP endpoint", "arguments": [
                { "name": "method", "description": "GET, POST, PUT, DELETE", "required": true },
                { "name": "path", "description": "e.g. /users", "required": true }
            ]},
            { "name": "setup-cron", "description": "Set up a scheduled cron job", "arguments": [
                { "name": "schedule", "description": "Cron expression", "required": true }
            ]},
            { "name": "event-pipeline", "description": "Build an event-driven pipeline", "arguments": [] }
        ]
    })
}

fn require_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, Value> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            json!({ "messages": [{ "role": "user", "content": { "type": "text", "text": format!("Missing required argument: {key}") } }] })
        })
}

pub fn get(params: Option<Value>) -> Value {
    let name = params
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let args = params
        .as_ref()
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(json!({}));

    let text = match name {
        "register-function" => {
            let lang = match require_arg(&args, "language") {
                Ok(v) => v,
                Err(e) => return e,
            };
            let fid = match require_arg(&args, "function_id") {
                Ok(v) => v,
                Err(e) => return e,
            };
            // Only node/python are supported by the worker manager — anything
            // else would generate a misleading node-flavored prompt and then
            // fail at spawn time. Reject up front with a clear message.
            match lang {
                "python" => format!(
                    "Register Python function `{fid}`:\n\
                     1. `iii_worker_register` with language='python'\n\
                     2. Code: `async def handler(input): ...`\n\
                     3. Wire trigger via `iii_trigger_register`\n\n\
                     ```python\nfrom iii_sdk import register_worker, Logger\n\
                     iii = register_worker('ws://localhost:49134')\n\
                     # metadata={{'mcp.expose': True}} so the function appears in tools/list\n\
                     iii.register_function('{fid}', handler, metadata={{'mcp.expose': True}})\n\
                     ```"
                ),
                "node" => format!(
                    "Register Node.js function `{fid}`:\n\
                     1. `iii_worker_register` with language='node'\n\
                     2. Code: `async (input) => {{ ... }}`\n\
                     3. Wire trigger via `iii_trigger_register`\n\n\
                     ```js\nimport {{ registerWorker, Logger }} from 'iii-sdk'\n\
                     const iii = registerWorker('ws://localhost:49134')\n\
                     // metadata: {{ 'mcp.expose': true }} so the function appears in tools/list\n\
                     iii.registerFunction('{fid}', handler, {{ metadata: {{ 'mcp.expose': true }} }})\n\
                     ```"
                ),
                _ => format!(
                    "Unsupported language `{lang}`. Pass `language=node` or `language=python`."
                ),
            }
        }
        "build-api" => {
            let method = match require_arg(&args, "method") {
                Ok(v) => v,
                Err(e) => return e,
            };
            let path = match require_arg(&args, "path") {
                Ok(v) => v,
                Err(e) => return e,
            };
            format!("Expose HTTP {method} {path}:\n1. Register function\n2. Register trigger:\n```json\n{{ \"trigger_type\": \"http\", \"function_id\": \"api::handler\", \"config\": {{ \"api_path\": \"{path}\", \"http_method\": \"{method}\" }} }}\n```\n3. Input: {{ body, query_params, path_params, headers }}\n4. Return: {{ status_code, headers, body }}")
        }
        "setup-cron" => {
            let schedule = match require_arg(&args, "schedule") {
                Ok(v) => v,
                Err(e) => return e,
            };
            format!("Cron `{schedule}`:\n1. Register function\n2. Register trigger:\n```json\n{{ \"trigger_type\": \"cron\", \"function_id\": \"jobs::task\", \"config\": {{ \"expression\": \"{schedule}\" }} }}\n```\n3. `iii_trigger_unregister` to stop")
        }
        "event-pipeline" => "Event pipeline:\n```\nHTTP \u{2192} Fn A \u{2192} emit('order.created') \u{2192} Fn B \u{2192} emit('done') \u{2192} Fn C\n```\n1. Register functions per stage\n2. Wire HTTP trigger to entry\n3. Queue triggers: `{ \"trigger_type\": \"queue\", \"config\": { \"topic\": \"order.created\" } }`\n4. `iii_trigger_enqueue` for async, `iii_trigger_void` for fire-and-forget".to_string(),
        _ => format!("Unknown prompt: {name}"),
    };

    json!({ "messages": [{ "role": "user", "content": { "type": "text", "text": text } }] })
}
