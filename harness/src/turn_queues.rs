use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::trigger::Trigger;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::time::Instant;

use crate::types::turn::TurnLane;

pub const ROOT_TOPIC: &str = "harness-turn";
pub const SUBAGENT_TOPIC: &str = "harness-subagent";
pub const REACTIVE_TOPIC: &str = "harness-reactive";

pub fn topic_for_lane(lane: TurnLane) -> &'static str {
    match lane {
        TurnLane::Root => ROOT_TOPIC,
        TurnLane::Subagent => SUBAGENT_TOPIC,
        TurnLane::Reactive => REACTIVE_TOPIC,
    }
}

const TURN_FUNCTION_ID: &str = "harness::turn";
const DURABLE_SUBSCRIBER: &str = "durable:subscriber";
const LIST_TOPICS_FUNCTION_ID: &str = "engine::queue::list_topics";
const BOOT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

struct QueueSubscriberSpec {
    topic: &'static str,
    request: RegisterTriggerInput,
}

fn subscriber_specs() -> Vec<QueueSubscriberSpec> {
    [ROOT_TOPIC, SUBAGENT_TOPIC, REACTIVE_TOPIC]
        .into_iter()
        .map(|topic| QueueSubscriberSpec {
            topic,
            request: RegisterTriggerInput {
                trigger_type: DURABLE_SUBSCRIBER.to_string(),
                function_id: TURN_FUNCTION_ID.to_string(),
                config: json!({
                    "queue": topic,
                    "queue_config": {
                        "type": "standard",
                        "concurrency": 10,
                        "maxRetries": 3,
                        "backoffType": "exponential",
                        "backoffDelayMs": 1000
                    }
                }),
                metadata: Some(json!({ "internal": true, "owner": "harness" })),
            },
        })
        .collect()
}

trait BindingHandle {
    fn unregister(&self);
}

impl BindingHandle for Trigger {
    fn unregister(&self) {
        Trigger::unregister(self);
    }
}

fn register_all_with<H, E, F>(mut register: F) -> std::result::Result<Vec<H>, E>
where
    H: BindingHandle,
    F: FnMut() -> std::result::Result<H, E>,
{
    let mut handles = Vec::with_capacity(3);
    for _ in 0..3 {
        match register() {
            Ok(handle) => handles.push(handle),
            Err(error) => {
                for handle in &handles {
                    handle.unregister();
                }
                return Err(error);
            }
        }
    }
    Ok(handles)
}

fn topics_ready(value: &Value) -> bool {
    let topics = value
        .as_array()
        .or_else(|| value.get("topics").and_then(Value::as_array));
    let Some(topics) = topics else {
        return false;
    };
    [ROOT_TOPIC, SUBAGENT_TOPIC, REACTIVE_TOPIC]
        .iter()
        .all(|required| {
            topics.iter().any(|topic| {
                topic.as_str() == Some(required)
                    || topic.get("name").and_then(Value::as_str) == Some(required)
            })
        })
}

pub struct TurnQueueBindings {
    handles: Vec<Trigger>,
}

impl TurnQueueBindings {
    pub async fn bind(iii: &Arc<IIIClient>) -> Result<Self> {
        let mut specs = subscriber_specs().into_iter();
        let handles = register_all_with(|| {
            let spec = specs
                .next()
                .expect("subscriber spec count matches registration count");
            tracing::debug!(topic = spec.topic, "registering harness queue subscriber");
            iii.register_trigger(spec.request)
        })
        .map_err(|error| anyhow!("registering harness queue subscribers: {error}"))?;

        let bindings = Self { handles };
        if let Err(error) = wait_for_topics(iii).await {
            bindings.unregister_all();
            return Err(error);
        }
        Ok(bindings)
    }

    fn unregister_all(&self) {
        for handle in &self.handles {
            handle.unregister();
        }
    }

    pub fn shutdown(self) {
        self.unregister_all();
    }
}

async fn wait_for_topics(iii: &IIIClient) -> Result<()> {
    let deadline = Instant::now() + BOOT_TIMEOUT;
    let mut last_error = "queue topics were not reported".to_string();

    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(anyhow!(
                "harness queue provisioning timed out after 10 seconds: {last_error}"
            ));
        }
        let remaining = deadline.saturating_duration_since(now);
        let request = iii.trigger(TriggerRequest {
            function_id: LIST_TOPICS_FUNCTION_ID.to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(1000),
        });
        match tokio::time::timeout(remaining.min(Duration::from_secs(1)), request).await {
            Ok(Ok(value)) if topics_ready(&value) => return Ok(()),
            Ok(Ok(_)) => last_error = "not all harness topics are ready".to_string(),
            Ok(Err(error)) => last_error = error.to_string(),
            Err(_) => last_error = "topic readiness request timed out".to_string(),
        }
        tokio::time::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn boot_builds_three_independent_standard_subscribers() {
        let specs = subscriber_specs();
        assert_eq!(specs.len(), 3);
        assert_eq!(
            specs.iter().map(|spec| spec.topic).collect::<Vec<_>>(),
            vec![ROOT_TOPIC, SUBAGENT_TOPIC, REACTIVE_TOPIC]
        );
        for spec in specs {
            assert_eq!(spec.request.trigger_type, "durable:subscriber");
            assert_eq!(spec.request.function_id, "harness::turn");
            assert_eq!(spec.request.config["queue"], spec.topic);
            assert_eq!(spec.request.config["queue_config"]["type"], "standard");
            assert_eq!(spec.request.config["queue_config"]["concurrency"], 10);
            assert_eq!(spec.request.config["queue_config"]["maxRetries"], 3);
            assert_eq!(
                spec.request.config["queue_config"]["backoffType"],
                "exponential"
            );
            assert_eq!(spec.request.config["queue_config"]["backoffDelayMs"], 1000);
        }
    }

    #[test]
    fn readiness_requires_every_harness_topic() {
        assert!(!topics_ready(&serde_json::json!([
            {"name": ROOT_TOPIC},
            {"name": SUBAGENT_TOPIC}
        ])));
        assert!(topics_ready(&serde_json::json!([
            {"name": REACTIVE_TOPIC},
            {"name": ROOT_TOPIC},
            {"name": SUBAGENT_TOPIC}
        ])));
    }

    #[derive(Clone, Debug)]
    struct FakeHandle(Arc<Mutex<Vec<&'static str>>>, &'static str);

    impl BindingHandle for FakeHandle {
        fn unregister(&self) {
            self.0.lock().unwrap().push(self.1);
        }
    }

    #[test]
    fn partial_registration_failure_rolls_back_existing_bindings() {
        let unregistered = Arc::new(Mutex::new(Vec::new()));
        let mut calls = 0;
        let result = register_all_with(|| {
            calls += 1;
            if calls == 3 {
                Err("registration failed")
            } else {
                Ok(FakeHandle(
                    unregistered.clone(),
                    if calls == 1 { "root" } else { "subagent" },
                ))
            }
        });

        assert_eq!(result.unwrap_err(), "registration failed");
        assert_eq!(*unregistered.lock().unwrap(), vec!["root", "subagent"]);
    }
}
