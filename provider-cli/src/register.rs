//! Register `provider::cli::complete` and `provider::cli::list_models`.

use iii_sdk::{FunctionRef, IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

use crate::shapes::{lookup_by_model, CLI_SHAPES};

pub struct ProviderCliFunctionRefs {
    pub complete: FunctionRef,
    pub list_models: FunctionRef,
}

impl ProviderCliFunctionRefs {
    pub fn unregister_all(self) {
        for r in [self.complete, self.list_models] {
            r.unregister();
        }
    }
}

async fn cli_complete_handler(iii: III, payload: Value) -> Result<Value, IIIError> {
    let model = required_str(&payload, "model")?;
    let prompt = required_str(&payload, "prompt")?;
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(120_000);

    let Some(shape) = lookup_by_model(&model) else {
        return Ok(json!({
            "ok": false,
            "text": "",
            "model": model,
            "error": format!("unsupported cli provider: {}", model.split('/').next().unwrap_or(""))
        }));
    };

    let which = iii
        .trigger(TriggerRequest {
            function_id: "shell::bash::which".into(),
            payload: json!({ "bin": shape.bin }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| IIIError::Handler(e.to_string()))?;
    let path = which
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if path.is_none() {
        return Ok(json!({
            "ok": false,
            "text": "",
            "model": model,
            "error": format!("{} not installed", shape.bin)
        }));
    }

    let argv = (shape.args)(&prompt);
    let exec = iii
        .trigger(TriggerRequest {
            function_id: "shell::bash::exec".into(),
            payload: json!({ "cmd": shape.bin, "args": argv, "timeout_ms": timeout_ms }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| IIIError::Handler(e.to_string()))?;

    let code = exec.get("code").and_then(Value::as_i64).unwrap_or(-1);
    let stdout = exec
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let stderr = exec
        .get("stderr")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if code == 0 {
        Ok(json!({ "ok": true, "text": stdout, "model": model }))
    } else {
        Ok(
            json!({ "ok": false, "text": "", "model": model, "error": format!("exit {code}: {stderr}") }),
        )
    }
}

pub async fn register_with_iii(iii: &III) -> anyhow::Result<ProviderCliFunctionRefs> {
    let iii_for_complete = iii.clone();
    let complete = iii.register_function((
        RegisterFunctionMessage::with_id("provider::cli::complete".into()).with_description(
            "Wrap an installed CLI as a provider. Calls shell::bash::which then shell::bash::exec."
                .into(),
        ),
        move |payload: Value| {
            let iii = iii_for_complete.clone();
            async move { cli_complete_handler(iii, payload).await }
        },
    ));

    let iii_for_list = iii.clone();
    let list_models = iii.register_function((
        RegisterFunctionMessage::with_id("provider::cli::list_models".into())
            .with_description("Probe each known CLI; report installed status.".into()),
        move |_payload: Value| {
            let iii = iii_for_list.clone();
            async move {
                let mut models = Vec::with_capacity(CLI_SHAPES.len());
                for shape in CLI_SHAPES {
                    let resp = iii
                        .trigger(TriggerRequest {
                            function_id: "shell::bash::which".into(),
                            payload: json!({ "bin": shape.bin }),
                            action: None,
                            timeout_ms: None,
                        })
                        .await;
                    let installed = resp
                        .ok()
                        .and_then(|v| v.get("path").and_then(Value::as_str).map(str::to_string))
                        .is_some_and(|s| !s.is_empty());
                    models.push(json!({
                        "id": format!("{}/default", shape.tag),
                        "bin": shape.bin,
                        "installed": installed
                    }));
                }
                Ok(json!({ "models": models }))
            }
        },
    ));

    Ok(ProviderCliFunctionRefs {
        complete,
        list_models,
    })
}

fn required_str(payload: &Value, field: &str) -> Result<String, IIIError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| IIIError::Handler(format!("missing required field: {field}")))
}
