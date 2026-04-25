//! A single connected remote A2A agent.
//!
//! Holds the agent card, an HTTP client, and the dispatch helpers used by
//! `registration.rs` to wire each remote skill to a local iii function.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures_util::{stream::StreamExt, Stream};
use reqwest::Client;
use reqwest_eventsource::{Event, EventSource};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::types::*;

/// One remote A2A agent. Cloneable handle (`Arc` inside) so the registration
/// layer can share it between every skill closure.
pub struct Session {
    pub name: String,
    pub base_url: String,
    pub card: Arc<RwLock<AgentCard>>,
    http: Client,
}

impl Session {
    /// Fetch the agent card and build a session.
    pub async fn connect(base_url: impl Into<String>) -> Result<Arc<Self>> {
        let base_url = base_url.into();
        let base_url = base_url.trim_end_matches('/').to_string();
        let http = Client::builder()
            .user_agent(concat!("iii-a2a-client/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("build reqwest client")?;

        let card_url = format!("{}/.well-known/agent-card.json", base_url);
        let card: AgentCard = http
            .get(&card_url)
            .send()
            .await
            .with_context(|| format!("fetching agent card from {card_url}"))?
            .error_for_status()
            .with_context(|| format!("agent card {card_url} returned non-2xx"))?
            .json()
            .await
            .with_context(|| format!("parsing agent card from {card_url}"))?;

        let name = derive_name(&card);

        tracing::info!(
            name = %name,
            base_url = %base_url,
            skills = card.skills.len(),
            streaming = card.capabilities.streaming,
            "a2a-client: agent connected"
        );

        Ok(Arc::new(Self {
            name,
            base_url,
            card: Arc::new(RwLock::new(card)),
            http,
        }))
    }

    /// Re-fetch the agent card. Used by the poll loop to detect skill drift.
    pub async fn refresh_card(&self) -> Result<AgentCard> {
        let card_url = format!("{}/.well-known/agent-card.json", self.base_url);
        let card: AgentCard = self
            .http
            .get(&card_url)
            .send()
            .await
            .with_context(|| format!("re-fetching agent card from {card_url}"))?
            .error_for_status()
            .with_context(|| format!("agent card {card_url} returned non-2xx"))?
            .json()
            .await
            .with_context(|| format!("parsing agent card from {card_url}"))?;
        *self.card.write().await = card.clone();
        Ok(card)
    }

    /// Send a `message/send` JSON-RPC call carrying a data part shaped as
    /// `{"function_id": skill_id, "payload": <handler input>}`. Returns the
    /// remote `Task` (terminal or otherwise). The caller decides how to map
    /// the task back to a local `Result<Value, IIIError>`.
    pub async fn send_message(&self, skill_id: &str, payload: Value) -> Result<Task> {
        let req = build_send_request(skill_id, payload, "message/send");
        let resp: A2AResponse = self
            .http
            .post(format!("{}/a2a", self.base_url))
            .json(&req)
            .send()
            .await
            .with_context(|| format!("POST {}/a2a", self.base_url))?
            .error_for_status()
            .with_context(|| format!("/a2a returned non-2xx for {skill_id}"))?
            .json()
            .await
            .with_context(|| format!("parse /a2a response for {skill_id}"))?;

        if let Some(err) = resp.error {
            return Err(anyhow!(
                "remote A2A error code={} message={}",
                err.code,
                err.message
            ));
        }

        let result = resp
            .result
            .ok_or_else(|| anyhow!("remote A2A response missing both result and error"))?;
        // some servers return Task directly, others wrap in {task: ...}
        let task: Task = serde_json::from_value(result.get("task").cloned().unwrap_or(result))
            .context("parse Task from /a2a result")?;
        Ok(task)
    }

    /// Send a `message/stream` JSON-RPC call and yield each event the remote
    /// agent emits. Returns a typed error early if the remote agent's card
    /// does not advertise `capabilities.streaming` — opening the SSE stream
    /// against a JSON-only endpoint would otherwise surface as an opaque
    /// `EventSource::InvalidContentType` wrapper error.
    pub async fn stream_message(
        &self,
        skill_id: &str,
        payload: Value,
    ) -> Result<impl Stream<Item = Result<StreamEvent>> + Send + 'static> {
        if !self.card.read().await.capabilities.streaming {
            return Err(anyhow!(
                "Remote agent does not advertise streaming capability"
            ));
        }
        let req = build_send_request(skill_id, payload, "message/stream");
        let request_builder = self
            .http
            .post(format!("{}/a2a", self.base_url))
            .header("accept", "text/event-stream")
            .json(&req);

        let es =
            EventSource::new(request_builder).context("create EventSource for message/stream")?;

        let stream = es.filter_map(|ev| async move {
            match ev {
                Ok(Event::Open) => None,
                Ok(Event::Message(msg)) => Some(parse_sse_event(&msg.data)),
                Err(reqwest_eventsource::Error::StreamEnded) => None,
                Err(e) => Some(Err(anyhow!("SSE error: {e}"))),
            }
        });

        Ok(stream)
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Task> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": Uuid::new_v4().to_string(),
            "method": "tasks/get",
            "params": { "id": task_id },
        });
        let resp: A2AResponse = self
            .http
            .post(format!("{}/a2a", self.base_url))
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if let Some(err) = resp.error {
            return Err(anyhow!("remote tasks/get failed: {}", err.message));
        }
        let result = resp
            .result
            .ok_or_else(|| anyhow!("tasks/get missing result"))?;
        let task: Task = serde_json::from_value(result.get("task").cloned().unwrap_or(result))?;
        Ok(task)
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<Task> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": Uuid::new_v4().to_string(),
            "method": "tasks/cancel",
            "params": { "id": task_id },
        });
        let resp: A2AResponse = self
            .http
            .post(format!("{}/a2a", self.base_url))
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if let Some(err) = resp.error {
            return Err(anyhow!("remote tasks/cancel failed: {}", err.message));
        }
        let result = resp
            .result
            .ok_or_else(|| anyhow!("tasks/cancel missing result"))?;
        let task: Task = serde_json::from_value(result.get("task").cloned().unwrap_or(result))?;
        Ok(task)
    }
}

fn build_send_request(skill_id: &str, payload: Value, method: &str) -> Value {
    let message_id = Uuid::new_v4().to_string();
    json!({
        "jsonrpc": "2.0",
        "id": Uuid::new_v4().to_string(),
        "method": method,
        "params": {
            "message": {
                "messageId": message_id,
                "role": "user",
                "parts": [{
                    "data": {
                        "function_id": skill_id,
                        "payload": payload,
                    }
                }],
            }
        },
    })
}

fn parse_sse_event(data: &str) -> Result<StreamEvent> {
    let raw: Value = serde_json::from_str(data).context("parse SSE JSON-RPC envelope")?;
    let payload = raw.get("result").cloned().unwrap_or(raw);

    if let Ok(ev) = serde_json::from_value::<TaskStatusUpdateEvent>(payload.clone()) {
        return Ok(StreamEvent::Status(ev));
    }
    if let Ok(ev) = serde_json::from_value::<TaskArtifactUpdateEvent>(payload.clone()) {
        return Ok(StreamEvent::Artifact(ev));
    }
    if let Ok(task) = serde_json::from_value::<Task>(payload) {
        return Ok(StreamEvent::Task(task));
    }
    Err(anyhow!("unrecognised SSE event payload: {data}"))
}

/// Sanitise `<provider.org>__<agent.name>` into an iii-namespace-safe slug.
/// Falls back to just the agent name if no provider org is present.
//
// TODO: two distinct agents with identical (provider_org, agent.name) tuples
// slug to the same name and collide on registration. Disambiguate via base_url
// hash or per-connect index when this ships beyond local dev.
fn derive_name(card: &AgentCard) -> String {
    let provider = card
        .provider
        .as_ref()
        .map(|p| p.organization.as_str())
        .unwrap_or("");
    let raw = if provider.is_empty() {
        card.name.clone()
    } else {
        format!("{}_{}", provider, card.name)
    };
    sanitize_slug(&raw)
}

fn sanitize_slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '_' || ch == '-' || ch.is_whitespace() || ch == '.' || ch == '/' {
            out.push('_');
        }
    }
    let trimmed: String = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_lowercases_and_collapses() {
        assert_eq!(sanitize_slug("Acme Corp"), "acme_corp");
        assert_eq!(sanitize_slug("Acme.Corp/AI"), "acme_corp_ai");
        assert_eq!(sanitize_slug("___"), "agent");
        assert_eq!(sanitize_slug("simple"), "simple");
    }

    #[test]
    fn derive_name_with_provider() {
        let mut card = AgentCard {
            name: "Pricing".into(),
            description: "x".into(),
            version: "1".into(),
            supported_interfaces: vec![],
            provider: Some(AgentProvider {
                organization: "Acme".into(),
                url: String::new(),
            }),
            documentation_url: None,
            capabilities: AgentCapabilities::default(),
            default_input_modes: vec![],
            default_output_modes: vec![],
            skills: vec![],
        };
        assert_eq!(derive_name(&card), "acme_pricing");
        card.provider = None;
        assert_eq!(derive_name(&card), "pricing");
    }
}
