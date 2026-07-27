use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error as SdkError;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use crate::completion::{validate_success, CompletionBinding, CompletionEventV1, CompletionInbox};
use crate::error::{EvalError, Phase};
use crate::subject::ResolvedE2eSubjectV1;

const TURN_COMPLETED: &str = "harness::turn-completed";

pub struct ScenarioContext {
    client: Arc<IIIClient>,
    subject: Arc<ResolvedE2eSubjectV1>,
    inbox: CompletionInbox,
    invocation_timeout: Duration,
    completion_timeout: Duration,
    callback_id: String,
}

impl ScenarioContext {
    pub async fn connect(
        url: &str,
        run_id: &str,
        subject: ResolvedE2eSubjectV1,
        invocation_timeout: Duration,
        completion_timeout: Duration,
    ) -> Result<Self, EvalError> {
        let client = Arc::new(register_worker(
            url,
            InitOptions {
                metadata: Some(WorkerMetadata {
                    runtime: "rust".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    name: "harness-e2e".to_string(),
                    os: std::env::consts::OS.to_string(),
                    pid: Some(std::process::id()),
                    ..WorkerMetadata::default()
                }),
                ..InitOptions::default()
            },
        ));
        let callback_id = format!("harness-e2e::{run_id}::on-turn-completed");
        let inbox = CompletionInbox::default();
        let callback_inbox = inbox.clone();
        client.register_function(
            &callback_id,
            RegisterFunction::new_async(move |event: CompletionEventV1| {
                let inbox = callback_inbox.clone();
                async move {
                    inbox.push(event);
                    Ok::<Value, SdkError>(json!({ "accepted": true }))
                }
            })
            .description("E2E runner sink for terminal harness turn events")
            .metadata(json!({ "internal": true })),
        );

        let context = Self {
            client,
            subject: Arc::new(subject),
            inbox,
            invocation_timeout,
            completion_timeout,
            callback_id,
        };
        context.wait_for_callback_registration().await?;
        Ok(context)
    }

    pub fn subject(&self) -> &ResolvedE2eSubjectV1 {
        &self.subject
    }

    pub async fn trigger<I, O>(&self, function_id: &str, payload: I) -> Result<O, EvalError>
    where
        I: Serialize + Send,
        O: DeserializeOwned,
    {
        self.trigger_phase(Phase::Assert, function_id, payload)
            .await
    }

    pub(crate) async fn trigger_phase<I, O>(
        &self,
        phase: Phase,
        function_id: &str,
        payload: I,
    ) -> Result<O, EvalError>
    where
        I: Serialize + Send,
        O: DeserializeOwned,
    {
        let payload = serde_json::to_value(payload).map_err(|error| {
            EvalError::serialization(phase, function_id, format!("serialize request: {error}"))
        })?;
        let request = TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(self.invocation_timeout.as_millis().min(u64::MAX as u128) as u64),
        };
        let value =
            match tokio::time::timeout(self.invocation_timeout, self.client.trigger(request)).await
            {
                Err(_) => {
                    return Err(EvalError::timeout(
                        phase,
                        format!("local deadline invoking {function_id}"),
                    ));
                }
                Ok(Err(error)) => return Err(EvalError::from_sdk(phase, function_id, error)),
                Ok(Ok(value)) => value,
            };
        serde_json::from_value(value).map_err(|error| {
            EvalError::serialization(phase, function_id, format!("decode response: {error}"))
        })
    }

    pub async fn bind_completion(&self, session_id: &str) -> Result<CompletionBinding, EvalError> {
        let config = json!({ "session_id": session_id });
        let trigger = self
            .client
            .register_trigger(RegisterTriggerInput {
                trigger_type: TURN_COMPLETED.to_string(),
                function_id: self.callback_id.clone(),
                config: config.clone(),
                metadata: None,
            })
            .map_err(|error| {
                EvalError::from_sdk(Phase::Setup, "engine::register_trigger", error)
            })?;
        let binding = CompletionBinding::new(trigger);
        if let Err(error) = self.wait_for_completion_binding(&config).await {
            binding.unregister();
            return Err(error);
        }
        Ok(binding)
    }

    pub async fn await_completion(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<CompletionEventV1, EvalError> {
        let event = self
            .inbox
            .wait_terminal(session_id, turn_id, self.completion_timeout)
            .await?;
        validate_success(&event)?;
        Ok(event)
    }

    pub async fn metrics(
        &self,
        root_session_id: &str,
    ) -> Result<harness::functions::metrics::SessionMetricsResponseV1, EvalError> {
        self.trigger_phase(
            Phase::Collect,
            "harness::metrics",
            json!({ "root_session_id": root_session_id }),
        )
        .await
    }

    pub async fn transcript(&self, session_id: &str) -> Result<Value, EvalError> {
        let mut cursor: Option<String> = None;
        let mut messages = Vec::new();
        loop {
            let response: Value = self
                .trigger_phase(
                    Phase::Collect,
                    "session::messages",
                    json!({
                        "session_id": session_id,
                        "limit": 500,
                        "cursor": cursor,
                        "include_custom": true,
                    }),
                )
                .await?;
            let page = response
                .get("messages")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    EvalError::evidence(
                        "session::messages",
                        format!("malformed transcript page for {session_id}"),
                    )
                })?;
            messages.extend(page.iter().cloned());
            let next = response
                .get("next_cursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if next.is_none() {
                break;
            }
            if next == cursor {
                return Err(EvalError::evidence(
                    "session::messages",
                    format!("repeated transcript cursor for {session_id}"),
                ));
            }
            cursor = next;
        }
        Ok(json!({ "messages": messages }))
    }

    pub async fn stop_session(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
    ) -> Result<(), EvalError> {
        let _: harness::functions::stop::StopResponse = self
            .trigger_phase(
                Phase::Cleanup,
                "harness::stop",
                json!({ "session_id": session_id, "turn_id": turn_id }),
            )
            .await?;
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.client.shutdown_async().await;
    }

    async fn wait_for_callback_registration(&self) -> Result<(), EvalError> {
        self.wait_until("runner function registration", || async {
            let response = self
                .raw_trigger(
                    "engine::functions::list",
                    json!({ "include_internal": true }),
                )
                .await
                .ok()?;
            response
                .get("functions")?
                .as_array()?
                .iter()
                .any(|item| {
                    item.get("function_id").and_then(Value::as_str)
                        == Some(self.callback_id.as_str())
                })
                .then_some(())
        })
        .await
    }

    async fn wait_for_completion_binding(&self, config: &Value) -> Result<(), EvalError> {
        self.wait_until("completion trigger registration", || async {
            let response = self
                .raw_trigger(
                    "engine::registered-triggers::list",
                    json!({
                        "include_internal": true,
                        "function_id": self.callback_id,
                        "trigger_type": TURN_COMPLETED,
                    }),
                )
                .await
                .ok()?;
            response
                .get("registered_triggers")?
                .as_array()?
                .iter()
                .any(|item| {
                    item.get("trigger_type").and_then(Value::as_str) == Some(TURN_COMPLETED)
                        && item.get("function_id").and_then(Value::as_str)
                            == Some(self.callback_id.as_str())
                        && item.get("config") == Some(config)
                })
                .then_some(())
        })
        .await
    }

    async fn wait_until<F, Fut>(&self, label: &str, mut check: F) -> Result<(), EvalError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Option<()>>,
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if check().await.is_some() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(EvalError::setup(format!("timed out waiting for {label}")));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn raw_trigger(&self, function_id: &str, payload: Value) -> Result<Value, SdkError> {
        let timeout_ms = self.invocation_timeout.as_millis().min(u64::MAX as u128) as u64;
        tokio::time::timeout(
            self.invocation_timeout,
            self.client.trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            }),
        )
        .await
        .map_err(|_| SdkError::Timeout)?
    }
}
