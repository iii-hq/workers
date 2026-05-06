//! Non-streaming MVP client for the AWS Bedrock Converse API.
//!
//! Status: stub. The full implementation drives `aws-sdk-bedrockruntime`'s
//! `converse` method (single-shot, non-streaming) and synthesises the
//! Start + TextDelta + Stop + Done event sequence so the harness loop sees a
//! standard provider stream. The 1.x AWS SDK requires `rustc >= 1.91.1`,
//! which is ahead of the workspace toolchain (1.90). Until the toolchain
//! catches up, this crate compiles a stable config surface and any call to
//! `stream` returns a single classified error event — no panics, no hangs.
//!
//! When the toolchain is bumped, drop `aws-config` + `aws-sdk-bedrockruntime`
//! into `Cargo.toml`, replace [`stream_inner`] with a real `converse` call,
//! and the public surface stays the same. Streaming via Converse-Stream lands
//! after that — the AWS event-stream binary frames need bespoke parsing that
//! is not in scope for the 0.1.x line.

use std::sync::Arc;

use harness_types::{
    AgentMessage, AgentTool, AssistantMessage, AssistantMessageEvent, ContentBlock, ErrorKind,
    StopReason, TextContent,
};
use provider_base::error_event;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Provider name reported on every emitted `AssistantMessage`.
pub const PROVIDER_NAME: &str = "bedrock";

/// Default AWS region when neither the env nor the caller specifies one.
pub const DEFAULT_REGION: &str = "us-east-1";

/// Configuration for a single Bedrock Converse call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BedrockConfig {
    /// Bedrock model id, e.g. `anthropic.claude-3-5-sonnet-20240620-v1:0`.
    pub model_id: String,
    /// AWS region. `None` defers to `AWS_REGION` / SDK default chain.
    pub region: Option<String>,
    /// Primary AWS credential (access-key id). Empty when populated via
    /// [`BedrockConfig::from_env`] — the legacy path lets the AWS SDK
    /// default chain resolve credentials at call time. Populated via
    /// [`BedrockConfig::with_credential`] in P5, where `auth::get_token`
    /// supplies the access-key id.
    #[serde(default)]
    pub access_key_id: String,
    /// AWS secret access key. Empty when populated via
    /// [`BedrockConfig::from_env`]. Populated via
    /// [`BedrockConfig::with_credential`] from the `AWS_SECRET_ACCESS_KEY`
    /// companion env var — composite AWS creds are out of scope for P5.
    #[serde(default)]
    pub secret_access_key: String,
    /// Hard ceiling on response tokens passed via `inferenceConfig.maxTokens`.
    pub max_tokens: u32,
}

impl BedrockConfig {
    /// Build a config from the environment.
    ///
    /// Region resolution: `AWS_REGION` env var if set, else
    /// [`DEFAULT_REGION`]. Credentials are picked up by the AWS SDK default
    /// chain (env vars, `~/.aws/credentials`, IMDS, etc.) at call time, so
    /// this function only fails when the env layer itself is unreachable —
    /// which it is not on a normal Unix process. Returning a `Result` keeps
    /// the surface aligned with the other provider crates.
    pub fn from_env(model_id: impl Into<String>) -> Result<Self, std::env::VarError> {
        let region = std::env::var("AWS_REGION")
            .ok()
            .or_else(|| Some(DEFAULT_REGION.into()));
        Ok(Self {
            model_id: model_id.into(),
            region,
            access_key_id: String::new(),
            secret_access_key: String::new(),
            max_tokens: 4096,
        })
    }

    /// Build a config from a credential resolved via `auth::get_token`.
    /// The primary credential (AWS_ACCESS_KEY_ID equivalent) flows through
    /// `auth::get_token`; AWS_SECRET_ACCESS_KEY and AWS_REGION remain env
    /// reads — composite AWS creds are out of scope for P5. A future
    /// extension to `Credential` could carry the secret + region.
    pub fn with_credential(
        model_id: impl Into<String>,
        cred: &auth_credentials::Credential,
    ) -> anyhow::Result<Self> {
        let access_key_id = match cred {
            auth_credentials::Credential::ApiKey { key } => key.clone(),
            auth_credentials::Credential::OAuth { access_token, .. } => access_token.clone(),
        };
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")
            .map_err(|e| anyhow::anyhow!("missing AWS_SECRET_ACCESS_KEY: {e}"))?;
        let region =
            std::env::var("AWS_REGION").map_err(|e| anyhow::anyhow!("missing AWS_REGION: {e}"))?;
        Ok(Self {
            model_id: model_id.into(),
            region: Some(region),
            access_key_id,
            secret_access_key,
            max_tokens: 4096,
        })
    }

    pub fn with_max_tokens(mut self, max: u32) -> Self {
        self.max_tokens = max;
        self
    }

    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }
}

/// Stream a response from Bedrock Converse. Returns an event stream that
/// closes with `done` on success or `error` on failure. Never throws.
///
/// Current behaviour: emits a single classified `Error` event explaining
/// that the AWS SDK dependency is pending. The signature is final; the
/// internals will swap to a real `converse` call once the toolchain bump
/// lands. See module docs.
pub async fn stream(
    cfg: Arc<BedrockConfig>,
    system_prompt: String,
    messages: Vec<AgentMessage>,
    tools: Vec<AgentTool>,
) -> ReceiverStream<AssistantMessageEvent> {
    let (tx, rx) = mpsc::channel(8);
    tokio::spawn(async move {
        stream_inner(cfg, system_prompt, messages, tools, tx).await;
    });
    ReceiverStream::new(rx)
}

async fn stream_inner(
    cfg: Arc<BedrockConfig>,
    _system_prompt: String,
    _messages: Vec<AgentMessage>,
    _tools: Vec<AgentTool>,
    tx: mpsc::Sender<AssistantMessageEvent>,
) {
    let _ = tx
        .send(error_event(
            "Bedrock support pending; install aws-sdk-bedrockruntime",
            None,
            cfg.model_id.clone(),
            PROVIDER_NAME,
        ))
        .await;
}

/// Register `provider::bedrock::complete` on the iii bus.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()> {
    provider_base::register_provider_complete::<BedrockConfig, _, _, _, _>(
        iii,
        PROVIDER_NAME,
        |model: &str, cred: &auth_credentials::Credential| {
            BedrockConfig::with_credential(model, cred)
        },
        stream,
    );
    Ok(())
}

/// Convenience: collect a stream into a final `AssistantMessage`.
pub async fn collect(mut stream: ReceiverStream<AssistantMessageEvent>) -> AssistantMessage {
    let mut last: Option<AssistantMessage> = None;
    while let Some(ev) = stream.next().await {
        match ev {
            AssistantMessageEvent::Done { message } => return message,
            AssistantMessageEvent::Error { error } => return error,
            AssistantMessageEvent::Start { partial }
            | AssistantMessageEvent::TextStart { partial }
            | AssistantMessageEvent::TextDelta { partial, .. }
            | AssistantMessageEvent::TextEnd { partial }
            | AssistantMessageEvent::ToolcallStart { partial }
            | AssistantMessageEvent::ToolcallDelta { partial, .. }
            | AssistantMessageEvent::ToolcallEnd { partial }
            | AssistantMessageEvent::ThinkingStart { partial }
            | AssistantMessageEvent::ThinkingDelta { partial, .. }
            | AssistantMessageEvent::ThinkingEnd { partial } => {
                last = Some(partial);
            }
            _ => {}
        }
    }
    last.unwrap_or_else(|| AssistantMessage {
        content: vec![ContentBlock::Text(TextContent {
            text: "stream closed without final".into(),
        })],
        stop_reason: StopReason::Error,
        error_message: Some("stream closed without final".into()),
        error_kind: Some(ErrorKind::Transient),
        usage: None,
        model: String::new(),
        provider: PROVIDER_NAME.into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Combined env-mutating test. Cargo runs tests in parallel within a
    /// crate, so splitting these caused a race on `AWS_REGION`. Coverage is
    /// identical when sequenced. Now also exercises `with_credential`,
    /// which reads `AWS_SECRET_ACCESS_KEY` + `AWS_REGION`.
    #[test]
    fn from_env_region_resolution() {
        let prev_region = std::env::var("AWS_REGION").ok();
        let prev_secret = std::env::var("AWS_SECRET_ACCESS_KEY").ok();

        std::env::set_var("AWS_REGION", "eu-west-2");
        let cfg = BedrockConfig::from_env("anthropic.claude-3-5-sonnet-20240620-v1:0")
            .expect("env layer reachable");
        assert_eq!(cfg.region.as_deref(), Some("eu-west-2"));
        assert_eq!(cfg.model_id, "anthropic.claude-3-5-sonnet-20240620-v1:0");
        assert_eq!(cfg.max_tokens, 4096);

        std::env::remove_var("AWS_REGION");
        let cfg2 = BedrockConfig::from_env("anthropic.claude-3-haiku-20240307-v1:0").expect("ok");
        assert_eq!(cfg2.region.as_deref(), Some(DEFAULT_REGION));

        // with_credential — primary cred via auth::get_token; secret + region
        // remain env reads (composite AWS creds are out of scope for P5).
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "secret-fixture");
        std::env::set_var("AWS_REGION", "us-east-1");

        let cred = auth_credentials::Credential::ApiKey {
            key: "AKIA-fixture".into(),
        };
        let cfg = BedrockConfig::with_credential("the-model", &cred).unwrap();
        assert_eq!(cfg.access_key_id, "AKIA-fixture");
        assert_eq!(cfg.secret_access_key, "secret-fixture");
        assert_eq!(cfg.region.as_deref(), Some("us-east-1"));
        assert_eq!(cfg.model_id, "the-model");

        // OAuth path
        let cred_oauth = auth_credentials::Credential::OAuth {
            access_token: "AKIA-oauth".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            provider_extra: serde_json::Value::Null,
        };
        let cfg = BedrockConfig::with_credential("the-model", &cred_oauth).unwrap();
        assert_eq!(cfg.access_key_id, "AKIA-oauth");

        // Missing-companion-env errors.
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        assert!(BedrockConfig::with_credential("m", &cred).is_err());

        match prev_region {
            Some(v) => std::env::set_var("AWS_REGION", v),
            None => std::env::remove_var("AWS_REGION"),
        }
        match prev_secret {
            Some(v) => std::env::set_var("AWS_SECRET_ACCESS_KEY", v),
            None => std::env::remove_var("AWS_SECRET_ACCESS_KEY"),
        }
    }

    #[tokio::test]
    async fn stream_emits_classified_error_event() {
        let cfg = Arc::new(
            BedrockConfig::from_env("anthropic.claude-3-haiku-20240307-v1:0").expect("ok"),
        );
        let s = stream(cfg, String::new(), Vec::new(), Vec::new()).await;
        let final_msg = collect(s).await;
        assert!(matches!(final_msg.stop_reason, StopReason::Error));
        assert_eq!(final_msg.provider, PROVIDER_NAME);
        assert!(final_msg.error_message.is_some());
    }

    #[tokio::test]
    #[ignore = "requires AWS_ACCESS_KEY_ID"]
    async fn live_smoke() {
        if std::env::var("AWS_ACCESS_KEY_ID").is_err() {
            eprintln!("skipping: AWS_ACCESS_KEY_ID unset");
            return;
        }
        let cfg = Arc::new(
            BedrockConfig::from_env("anthropic.claude-3-haiku-20240307-v1:0")
                .expect("env reachable"),
        );
        let s = stream(cfg, "be terse".into(), Vec::new(), Vec::new()).await;
        let _ = collect(s).await;
    }
}
