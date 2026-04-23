use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::config::AgentConfig;
use crate::discovery;
use crate::llm::{LlmClient, LlmRequest, Message, MessageContent};

pub fn build_handler(
    iii: III,
    config: Arc<AgentConfig>,
    llm: Arc<LlmClient>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        let config = config.clone();
        let llm = llm.clone();

        Box::pin(async move { handle_plan(iii, config, llm, payload).await })
    }
}

async fn handle_plan(
    iii: III,
    config: Arc<AgentConfig>,
    llm: Arc<LlmClient>,
    payload: Value,
) -> Result<Value, IIIError> {
    let query = payload
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing 'query' field".to_string()))?
        .to_string();

    // Planner output is fed to downstream executors via function_id, so the
    // capabilities block must name engine ids (eval::metrics) not the
    // sanitized tool names (eval__metrics) the chat handler uses.
    let capabilities = discovery::build_planner_capabilities(&iii).await;

    let system = format!(
        "You are a planning agent for the iii engine. Given a user query, generate an execution \
         plan as a DAG of iii.trigger() calls. Do NOT execute anything.\n\
         \n\
         Return a JSON object with:\n\
         - \"steps\": an array of step objects, each with:\n\
           - \"id\": unique step identifier (e.g. \"step_1\")\n\
           - \"function_id\": the function to call\n\
           - \"payload\": the payload object\n\
           - \"depends_on\": array of step IDs this step depends on (empty if root)\n\
           - \"description\": human-readable description of what this step does\n\
         - \"summary\": a brief description of the overall plan\n\
         \n\
         {capabilities}"
    );

    let messages = vec![Message {
        role: "user".to_string(),
        content: MessageContent::Text(query),
    }];

    let request = LlmRequest {
        model: config.anthropic_model.clone(),
        max_tokens: config.max_tokens,
        system,
        messages,
        tools: None,
    };

    let response = llm
        .send(&request)
        .await
        .map_err(|e| IIIError::Handler(format!("LLM request failed: {}", e)))?;

    let text = LlmClient::extract_text(&response);

    let plan: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
        json!({
            "steps": [],
            "summary": text
        })
    });

    Ok(plan)
}
