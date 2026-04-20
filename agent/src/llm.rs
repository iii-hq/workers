use anyhow::{Result, anyhow};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Serialize)]
pub struct LlmRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    #[allow(dead_code)]
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub delta: Option<StreamDelta>,
    #[serde(default)]
    pub content_block: Option<ContentBlock>,
    #[serde(default)]
    #[allow(dead_code)]
    pub index: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamDelta {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub delta_type: Option<String>,
    pub text: Option<String>,
    pub partial_json: Option<String>,
}

pub struct LlmClient {
    client: Client,
    api_key: String,
}

impl LlmClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow!("ANTHROPIC_API_KEY environment variable not set"))?;
        Ok(Self::new(api_key))
    }

    pub async fn send(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Anthropic API error {}: {}", status, body));
        }

        let llm_response: LlmResponse = response.json().await?;
        Ok(llm_response)
    }

    pub async fn send_stream(
        &self,
        request: &LlmRequest,
    ) -> Result<impl futures_util::Stream<Item = Result<StreamEvent>>> {
        let mut stream_request = serde_json::to_value(request)?;
        stream_request["stream"] = serde_json::Value::Bool(true);

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&stream_request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Anthropic API error {}: {}", status, body));
        }

        let byte_stream = response.bytes_stream();

        let event_stream = byte_stream.map(|chunk_result| {
            let chunk = chunk_result.map_err(|e| anyhow!("stream read error: {}", e))?;
            let text = String::from_utf8_lossy(&chunk);
            let mut events = Vec::new();

            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        continue;
                    }
                    if let Ok(event) = serde_json::from_str::<StreamEvent>(data) {
                        events.push(event);
                    }
                }
            }

            Ok(events)
        });

        Ok(event_stream.flat_map(|result| {
            let items: Vec<Result<StreamEvent>> = match result {
                Ok(events) => events.into_iter().map(Ok).collect(),
                Err(e) => vec![Err(e)],
            };
            futures_util::stream::iter(items)
        }))
    }

    pub fn extract_text(response: &LlmResponse) -> String {
        let mut text = String::new();
        for block in &response.content {
            if let ContentBlock::Text { text: t } = block {
                text.push_str(t);
            }
        }
        text
    }

    pub fn extract_tool_uses(response: &LlmResponse) -> Vec<ToolUse> {
        let mut tool_uses = Vec::new();
        for block in &response.content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                tool_uses.push(ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }
        }
        tool_uses
    }
}
