//! A protocol-faithful fake provider: declares with backoff until acked
//! (`router::provider::register`), re-declares on the `router::ready` trigger,
//! resolves per request (`router::provider::resolve`), streams via script, and
//! reconciles discovery results (`router::models::reconcile`) — every
//! interaction goes through the same iii functions a real provider worker uses.
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::types::model::Model;
use serde_json::{json, Value};

use super::fake_bus::FakeBus;
use super::fake_channels::{ChannelHub, FakeWriter};
use crate::bus::{handler, Bus};

type StreamScript =
    Arc<dyn Fn(Value, FakeWriter) -> futures::future::BoxFuture<'static, Value> + Send + Sync>;

pub struct ScriptedProvider {
    pub id: String,
    pub token: Arc<Mutex<Option<String>>>,
    pub declare_attempts: Arc<AtomicU32>,
    pub refresh_calls: Arc<AtomicU32>,
}

pub struct ScriptedOptions {
    pub id: String,
    pub worker_id: String,
    pub models: Option<Vec<Model>>,
    pub supports_model_listing: bool,
    pub stream: Option<StreamScript>,
    pub discover: Option<Arc<dyn Fn() -> Option<Vec<Model>> + Send + Sync>>, // None = transient, don't reconcile
    pub retry_ms: u64,
}

impl Default for ScriptedOptions {
    fn default() -> Self {
        ScriptedOptions {
            id: "scripted".into(),
            worker_id: "worker-scripted".into(),
            models: None,
            supports_model_listing: false,
            stream: None,
            discover: None,
            retry_ms: 5,
        }
    }
}

pub fn start_scripted_provider(
    bus: Arc<FakeBus>,
    hub: Arc<ChannelHub>,
    opts: ScriptedOptions,
) -> ScriptedProvider {
    let state = ScriptedProvider {
        id: opts.id.clone(),
        token: Arc::new(Mutex::new(None)),
        declare_attempts: Arc::new(AtomicU32::new(0)),
        refresh_calls: Arc::new(AtomicU32::new(0)),
    };

    // declare with backoff until acknowledged
    let declare = {
        let bus = bus.clone();
        let token = state.token.clone();
        let attempts = state.declare_attempts.clone();
        let opts_id = opts.id.clone();
        let worker_id = opts.worker_id.clone();
        let models = opts.models.clone();
        let listing = opts.supports_model_listing;
        let retry_ms = opts.retry_ms;
        Arc::new(move || {
            let (bus, token, attempts) = (bus.clone(), token.clone(), attempts.clone());
            let (opts_id, worker_id, models) = (opts_id.clone(), worker_id.clone(), models.clone());
            tokio::spawn(async move {
                loop {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    let payload = json!({
                        "id": opts_id, "worker_id": worker_id, "supports_model_listing": listing,
                        "models": models, "token": *token.lock().unwrap(),
                    });
                    match bus
                        .trigger("router::provider::register", payload, None)
                        .await
                    {
                        Ok(res) => {
                            *token.lock().unwrap() =
                                res["registration_token"].as_str().map(String::from);
                            break;
                        }
                        Err(_) => {
                            tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await
                        }
                    }
                }
            });
        })
    };

    // re-declare on router::ready (works whether the router boots before or after us)
    {
        let declare = declare.clone();
        let fid = format!("prov::{}::on_ready", opts.id);
        bus.register_function(
            &fid,
            handler(move |_| {
                let declare = declare.clone();
                async move {
                    declare();
                    Ok(Value::Null)
                }
            }),
        );
        bus.register_trigger("router::ready", &fid, json!({}));
    }

    // provider::<id>::stream — resolve per request, then run the script
    {
        let (bus2, hub, token) = (bus.clone(), hub.clone(), state.token.clone());
        let id = opts.id.clone();
        let script = opts.stream.clone();
        bus.register_function(
            &format!("provider::{}::stream", opts.id),
            handler(move |input: Value| {
                let (bus, hub, token, id, script) = (
                    bus2.clone(),
                    hub.clone(),
                    token.clone(),
                    id.clone(),
                    script.clone(),
                );
                async move {
                    let tok = token.lock().unwrap().clone();
                    bus.trigger(
                        "router::provider::resolve",
                        json!({ "id": id, "token": tok }),
                        None,
                    )
                    .await?;
                    let r: crate::types::channel::StreamChannelRef =
                        serde_json::from_value(input["writer_ref"].clone()).expect("writer_ref");
                    let writer = hub.writer_for(&r).expect("forwarded ref resolves");
                    match script {
                        Some(s) => Ok(s(input, writer).await),
                        None => {
                            use crate::chat::relay::FrameSink;
                            let message = json!({
                                "role": "assistant", "content": [{ "type": "text", "text": "scripted" }],
                                "stop_reason": "end", "model": input["model"], "provider": id, "timestamp": 2
                            });
                            writer
                                .send(&json!({ "type": "done", "message": message }).to_string())
                                .ok();
                            writer.close();
                            Ok(json!({ "ok": true }))
                        }
                    }
                }
            }),
        );
    }

    // provider::<id>::refresh_models — discovery → reconcile (or transient skip)
    {
        let (bus2, token, refreshes) = (
            bus.clone(),
            state.token.clone(),
            state.refresh_calls.clone(),
        );
        let id = opts.id.clone();
        let discover = opts.discover.clone();
        let static_models = opts.models.clone();
        bus.register_function(
            &format!("provider::{}::refresh_models", opts.id),
            handler(move |_| {
                let (bus, token, refreshes, id) =
                    (bus2.clone(), token.clone(), refreshes.clone(), id.clone());
                let discover = discover.clone();
                let static_models = static_models.clone();
                async move {
                    refreshes.fetch_add(1, Ordering::SeqCst);
                    let models = match discover {
                        Some(d) => match d() {
                            Some(m) => m,
                            None => return Ok(json!({ "ok": false })), // transient: keep the previous slice
                        },
                        None => static_models.unwrap_or_default(),
                    };
                    let tok = token.lock().unwrap().clone();
                    bus.trigger(
                        "router::models::reconcile",
                        json!({ "provider": id, "models": models, "token": tok }),
                        None,
                    )
                    .await?;
                    Ok(json!({ "ok": true }))
                }
            }),
        );
    }

    declare();
    state
}
