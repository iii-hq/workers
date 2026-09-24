//! Durable, opt-in pull-request webhook monitor. Independent from github::called.
mod lifecycle;
#[cfg(test)]
mod lifecycle_tests;
pub mod normalize;
pub mod notifications;
pub mod store;
#[cfg(test)]
mod tests;
pub mod types;
mod wiring;
pub use types::*;
pub use wiring::register;

use crate::configuration::ConfigCell;
use chrono::Utc;
use hmac::{Hmac, Mac};
use iii_sdk::{
    channel::{ChannelReader, StreamChannelRef},
    protocol::TriggerRequest,
    IIIClient,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{collections::HashMap, path::Path, sync::Arc};
use store::Store;
use tokio::sync::Mutex;
static SELF_LOGIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static SELF_ATTEMPT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

#[derive(Debug, thiserror::Error)]
pub enum Failure {
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("webhooks disabled or storage unavailable; enable and restart after configuring dependencies")]
    Disabled,
    #[error("watch not found")]
    NotFound,
    #[error("watch_id already used with a different request")]
    Conflict,
    #[error("signature rejected")]
    Signature,
    #[error("payload too large")]
    Oversize,
    #[error("pending inbox/outbox capacity reached")]
    Capacity,
    #[error("blocking storage task failed: {0}")]
    StorageTask(#[from] tokio::task::JoinError),
    #[error("storage lock poisoned")]
    StoragePoisoned,
    #[error("storage failure: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("filesystem failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization failure: {0}")]
    Json(#[from] serde_json::Error),
    #[error("engine operation failed: {0}")]
    Engine(#[from] iii_sdk::Error),
    #[error("GitHub API failed ({0}); check Webhooks write and repository read permissions")]
    Github(String),
    #[error("hook creation outcome ambiguous; manual inspection required, no hook adopted")]
    AmbiguousHook,
    #[error("managed hook ownership/configuration changed; refusing mutation")]
    Ownership,
}
pub type Result<T> = std::result::Result<T, Failure>;
impl From<Failure> for iii_sdk::Error {
    fn from(e: Failure) -> Self {
        Self::Handler(e.to_string())
    }
}

pub struct Service {
    iii: Arc<IIIClient>,
    cell: ConfigCell,
    config: WebhookConfig,
    engine_url: String,
    store: Option<Store>,
    operations: Mutex<()>,
    #[cfg(test)]
    bus: Option<lifecycle_tests::MockBus>,
}
impl Service {
    pub fn store(&self) -> Result<&Store> {
        self.store.as_ref().ok_or(Failure::Disabled)
    }
    async fn invoke(&self, function_id: &str, payload: Value) -> Result<Value> {
        self.invoke_target(function_id, payload, None, None).await
    }
    async fn invoke_target(
        &self,
        function_id: &str,
        payload: Value,
        namespace: Option<&str>,
        metadata: Option<Value>,
    ) -> Result<Value> {
        #[cfg(test)]
        if let Some(bus) = &self.bus {
            return bus.invoke(function_id, payload, namespace, metadata);
        }
        let mut request = iii_sdk::protocol::TriggerRequestWithMetadata::from(TriggerRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(8_000),
        });
        if let Some(namespace) = namespace {
            request = request.namespace(namespace);
        }
        if let Some(metadata) = metadata {
            request = request.metadata(metadata);
        }
        Ok(self.iii.trigger(request).await?)
    }
    fn record_failure(&self, error: &Failure) -> Result<()> {
        self.store()?.change(|d| {
            d.last_error = Some(error.to_string());
            Ok(())
        })
    }
    async fn api(&self, method: &str, endpoint: &str, body: Option<Value>) -> Result<Value> {
        let output = self.api_output(method, endpoint, body, false).await?;
        if output.trim().is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_str(&output)?)
        }
    }
    async fn api_output(
        &self,
        method: &str,
        endpoint: &str,
        body: Option<Value>,
        include_headers: bool,
    ) -> Result<String> {
        let cfg = self.cell.read().await.clone();
        let mut args = vec![
            "api".into(),
            "--method".into(),
            method.into(),
            endpoint.into(),
        ];
        if include_headers {
            args.push("--include".into());
        }
        if body.is_some() {
            args.extend(["--input".into(), "-".into()]);
        }
        let out = crate::gh::run(&cfg, &args, body.map(|b| b.to_string()), Some(8_000))
            .await
            .map_err(|e| Failure::Github(e.code.into()))?;
        if out.exit_code != Some(0) || out.timed_out || out.stdout_truncated {
            // Do not surface gh stderr: hook create/PATCH payloads contain secrets.
            return Err(Failure::Github(format!(
                "exit {:?}, timeout {}",
                out.exit_code, out.timed_out
            )));
        }
        Ok(out.stdout)
    }
    pub fn status(&self, id: &str) -> Result<WatchResponse> {
        let d = self.store()?.read()?;
        let w = d.watches.get(id).ok_or(Failure::NotFound)?;
        let hook = d.repos.get(&w.spec.repo);
        let hook_ready = d.tunnel_status == "ready"
            && hook.is_some_and(|h| {
                h.hook_id.is_some() && h.error.is_none() && h.pending_url.is_none()
            })
            && w.error.is_none();
        let status = if w.status == WatchState::Active && !hook_ready {
            WatchState::Preparing
        } else {
            w.status
        };
        Ok(WatchResponse {
            watch_id: id.into(), repo: w.spec.repo.clone(), number: w.spec.number,
            status, snapshot: w.snapshot.clone(),
            health: Health {
                enabled: self.config.enabled, pending_jobs: d.jobs.len(),
                last_error: w.error.clone().or_else(|| hook.and_then(|h| h.error.clone())).or(d.last_error),
                hook_ready, tunnel_status: d.tunnel_status,
                cleanup_attempts: hook.map_or(0, |h| h.cleanup_attempts),
                delivery_guarantee: "at-least-once after local durable acceptance; Quick Tunnel ingress is not zero-loss".into(),
            },
        })
    }
    /// Only called by queue jobs. The job and effects are committed together;
    /// callback completion is committed before returning the queue ack.
    async fn process(&self, id: &str) -> Result<OperationResponse> {
        let _guard = self.operations.lock().await;
        let d = self.store()?.read()?;
        let Some(job) = d.jobs.get(id) else {
            return self.operation_response();
        };
        if id.starts_with(lifecycle::TUNNEL_JOB_PREFIX) {
            let Job::Inbox(inbox) = job else {
                return Err(Failure::Invalid("invalid tunnel lifecycle job".into()));
            };
            let event = serde_json::from_value(inbox.body.clone())?;
            if let Err(e) = self.process_tunnel(&event).await {
                self.record_failure(&e)?;
                return Err(e);
            }
            self.store()?.change(|d| {
                d.jobs.remove(id);
                d.publications.remove(id);
                Ok(())
            })?;
            return self.operation_response();
        }
        let mut policy = self.cell.read().await.webhooks.notifications.clone();
        if notifications::wants_self(&policy, &d) {
            policy.resolved_self = self.discover_self().await;
        }
        let publishes_notifications = matches!(job, Job::Inbox(_))
            || matches!(job, Job::Notify { target, .. } if d.subscribers.get(&target.id).is_some_and(|sub| notifications::effective(sub, &policy).profile != notifications::NotificationProfile::All));
        {
            match job {
                Job::Notify { event, target } => {
                    let Some(current_target) = d.subscribers.get(&target.id) else {
                        self.ack_job(id)?;
                        return self.operation_response();
                    };
                    let policy = &notifications::effective(current_target, &policy);
                    if policy.profile != notifications::NotificationProfile::All {
                        self.store()?.change(|data| {
                            data.event_history
                                .entry(event.event_id.clone())
                                .or_insert_with(|| notifications::StoredEvent {
                                    event: event.clone(),
                                    created_at: Utc::now().timestamp_millis(),
                                });
                            notifications::route(
                                data,
                                event,
                                current_target,
                                policy,
                                Utc::now().timestamp_millis(),
                            );
                            data.jobs.remove(id);
                            data.publications.remove(id);
                            Ok(())
                        })?;
                    } else {
                        // Legacy subscribers retain their original full-event contract.
                        // None is the legacy/default namespace, not this provider's.
                        // Crash before local ack can repeat event_id (at-least-once).
                        self.invoke_target(
                            &target.function_id,
                            serde_json::to_value(event)?,
                            Some(target.namespace.as_deref().unwrap_or("default")),
                            target.metadata.clone(),
                        )
                        .await?;
                        self.store()?.change(|d| {
                            d.jobs.remove(id);
                            d.publications.remove(id);
                            Ok(())
                        })?;
                    }
                }
                Job::ReviewNotify {
                    event,
                    target,
                    payload,
                    due_at,
                    ..
                } => {
                    if *due_at > Utc::now().timestamp_millis() {
                        self.store()?.change(|data| {
                            data.publications.remove(id);
                            Ok(())
                        })?;
                        return self.operation_response();
                    }
                    let Some(current_target) = d.subscribers.get(&target.id) else {
                        self.ack_job(id)?;
                        return self.operation_response();
                    };
                    if !current_target.filter.matches(event) {
                        self.ack_job(id)?;
                        return self.operation_response();
                    }
                    let current_policy = &notifications::effective(current_target, &policy);
                    if payload.kind == notifications::DIGEST_KIND {
                        let head = d
                            .watches
                            .get(&event.watch_id)
                            .and_then(|w| w.snapshot.head_sha.as_deref());
                        let mut digest = (**payload).clone();
                        if notifications::prune_digest(&mut digest, head) {
                            self.invoke_target(
                                &current_target.function_id,
                                serde_json::to_value(&digest)?,
                                Some(current_target.namespace.as_deref().unwrap_or("default")),
                                current_target.metadata.clone(),
                            )
                            .await?;
                        }
                        self.ack_job(id)?;
                        self.cleanup().await?;
                        return self.operation_response();
                    }
                    if payload.category == Category::Ci
                        && ((payload.kind == "ci.failed" && !current_policy.notify_ci_failures)
                            || (payload.kind == "ci.passed" && !current_policy.notify_ci_success))
                    {
                        self.ack_job(id)?;
                        return self.operation_response();
                    }
                    if payload.category == Category::Ci {
                        let Some(watch) = d.watches.get(&event.watch_id) else {
                            self.ack_job(id)?;
                            return self.operation_response();
                        };
                        if watch.snapshot.head_sha != payload.head_sha {
                            self.ack_job(id)?;
                            return self.operation_response();
                        }
                        if payload.kind == "ci.passed" {
                            // One event-driven confirmation, never polling. Refuse success if
                            // the PR moved or any selected check is pending/missing/non-success.
                            let pr = self
                                .api(
                                    "GET",
                                    &format!("repos/{}/pulls/{}", event.repo, event.number),
                                    None,
                                )
                                .await?;
                            let snapshot = normalize::snapshot(&pr, &watch.snapshot)?;
                            let snapshot = self.reconcile_ci(&event.repo, snapshot).await?;
                            let snapshot = self
                                .confirm_selected_workflows(&event.repo, snapshot, current_policy)
                                .await?;
                            if snapshot.head_sha != payload.head_sha
                                || !notifications::selected_passed(&snapshot, current_policy)
                            {
                                self.store()?.change(|data| {
                                    notifications::release_success(
                                        data,
                                        current_target,
                                        event,
                                        current_policy,
                                    );
                                    Ok(())
                                })?;
                                self.ack_job(id)?;
                                return self.operation_response();
                            }
                        }
                    }
                    self.invoke_target(
                        &current_target.function_id,
                        serde_json::to_value(payload)?,
                        Some(current_target.namespace.as_deref().unwrap_or("default")),
                        current_target.metadata.clone(),
                    )
                    .await?;
                    self.ack_job(id)?;
                }
                Job::Inbox(inbox) => {
                    let targets: Vec<_> = d
                        .watches
                        .values()
                        .filter(|w| normalize::relevant(w, inbox))
                        .cloned()
                        .collect();
                    let mut updates = Vec::new();
                    for w in targets {
                        let pr = self
                            .api(
                                "GET",
                                &format!("repos/{}/pulls/{}", w.spec.repo, w.spec.number),
                                None,
                            )
                            .await?;
                        updates.push((
                            w.spec.watch_id.clone(),
                            normalize::snapshot(&pr, &w.snapshot)?,
                        ));
                    }
                    self.store()?.change(|data| {
                        for (watch_id, snapshot) in updates {
                            if let Some(w) = data.watches.get_mut(&watch_id) {
                                if let Some(event) = normalize::normalize(w, inbox, snapshot) {
                                    normalize::persist_event_with_policy(
                                        data,
                                        event,
                                        &inbox.delivery,
                                        &policy,
                                    );
                                }
                            }
                        }
                        data.jobs.remove(id);
                        data.publications.remove(id);
                        Ok(())
                    })?;
                }
            }
        }
        let publication = if publishes_notifications {
            self.publish_pending().await
        } else {
            Ok(())
        };
        let cleanup = self.cleanup().await;
        publication.and(cleanup)?;
        self.operation_response()
    }
    /// Authenticated gh login for ignore_self: one GET /user, cached in memory on
    /// success; a failure (e.g. an App installation token) retries after 5 min.
    async fn discover_self(&self) -> Option<String> {
        use std::sync::atomic::Ordering::Relaxed;
        if let Some(login) = SELF_LOGIN.get() {
            return Some(login.clone());
        }
        let now = Utc::now().timestamp();
        if now - SELF_ATTEMPT.load(Relaxed) < 300 {
            return None;
        }
        SELF_ATTEMPT.store(now, Relaxed);
        let login = self.api("GET", "user", None).await.ok()?["login"]
            .as_str()?
            .to_owned();
        Some(SELF_LOGIN.get_or_init(|| login).clone())
    }
    fn ack_job(&self, id: &str) -> Result<()> {
        self.store()?.change(|data| {
            data.jobs.remove(id);
            data.publications.remove(id);
            Ok(())
        })
    }
    pub fn event_detail(&self, req: notifications::EventDetailRequest) -> Result<PrEvent> {
        self.store()?
            .read()?
            .event_history
            .get(&req.event_id)
            .map(|v| (*v.event).clone())
            .ok_or(Failure::NotFound)
    }
    fn operation_response(&self) -> Result<OperationResponse> {
        Ok(OperationResponse {
            status: "ok".into(),
            pending_jobs: self.store()?.read()?.jobs.len(),
        })
    }
    async fn publish_pending(&self) -> Result<()> {
        // Retain jobs until execution ack, not publish ack. This covers a crash
        // at every boundary, including queue redelivery/DLQ and engine restart.
        let data = self.store()?.read()?;
        let now = Utc::now().timestamp();
        let now_ms = Utc::now().timestamp_millis();
        for id in data
            .jobs
            .keys()
            .filter(|id| {
                if matches!(data.jobs.get(id), Some(Job::ReviewNotify { due_at, .. }) if *due_at > now_ms) { return false; }
                data.publications
                    .get(id)
                    .is_none_or(|(at, attempts)| *attempts < 5 && now - at >= 60)
            })
            .take(100)
        {
            let claimed = self.store()?.change(|d| {
                if !d.jobs.contains_key(id)
                    || d.publications
                        .get(id)
                        .is_some_and(|(at, attempts)| *attempts >= 5 || now - at < 60)
                {
                    return Ok(false);
                }
                let previous = d.publications.get(id).map_or(0, |(_, n)| *n);
                d.publications.insert(id.clone(), (now, previous + 1));
                Ok(true)
            })?;
            if !claimed {
                continue;
            }
            let result = self
                .invoke(
                    "iii::durable::publish",
                    json!({"topic": self.config.queue, "data": {"job_id": id}}),
                )
                .await;
            if let Err(e) = &result {
                self.store()?.change(|d| {
                    d.last_error = Some(format!("outbox publication failed: {e}"));
                    Ok(())
                })?;
            }
            result?;
        }
        Ok(())
    }
    fn accept(
        &self,
        endpoint: &str,
        headers: &HashMap<String, String>,
        raw: &[u8],
    ) -> Result<bool> {
        if raw.len() > self.config.max_body_bytes {
            return Err(Failure::Oversize);
        }
        let data = self.store()?.read()?;
        let (repo, hook) = data
            .repos
            .iter()
            .find(|(_, h)| h.endpoint_id == endpoint)
            .ok_or(Failure::Signature)?;
        verify_signature(
            &hook.secret,
            raw,
            header(headers, "x-hub-signature-256").ok_or(Failure::Signature)?,
        )?;
        let hook_id = header(headers, "x-github-hook-id")
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or(Failure::Signature)?;
        if hook.hook_id != Some(hook_id) {
            return Err(Failure::Signature);
        }
        let delivery = header(headers, "x-github-delivery")
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .ok_or_else(|| Failure::Invalid("missing delivery".into()))?;
        let event = header(headers, "x-github-event")
            .ok_or_else(|| Failure::Invalid("missing event".into()))?;
        let body: Value = serde_json::from_slice(raw)?;
        if !body
            .pointer("/repository/full_name")
            .and_then(Value::as_str)
            .is_some_and(|r| r.eq_ignore_ascii_case(repo))
        {
            return Err(Failure::Signature);
        }
        let key = format!("{hook_id}:{delivery}");
        self.store()?.change(|d| {
            if d.deliveries.contains(&key) {
                return Ok(false);
            }
            if d.jobs.len() >= self.config.max_pending {
                return Err(Failure::Capacity);
            }
            d.deliveries.insert(key.clone());
            d.jobs.insert(
                format!("inbox:{key}"),
                Job::Inbox(Inbox {
                    repo: repo.clone(),
                    event: event.into(),
                    delivery: delivery.into(),
                    body,
                }),
            );
            Ok(true)
        })
    }
    async fn receive(self: &Arc<Self>, req: ReceiveRequest) -> HttpResponse {
        let result = tokio::time::timeout(std::time::Duration::from_secs(7), async {
            self.store()?;
            if header(&req.headers, "content-length")
                .and_then(|v| v.parse::<usize>().ok())
                .is_some_and(|n| n > self.config.max_body_bytes)
            {
                return Err(Failure::Oversize);
            }
            let reader = ChannelReader::new(&self.engine_url, &req.request_body);
            let mut raw = Vec::new();
            while let Some(chunk) = reader.next_binary().await? {
                if chunk.len() > self.config.max_body_bytes.saturating_sub(raw.len()) {
                    reader.close().await?;
                    return Err(Failure::Oversize);
                }
                raw.extend_from_slice(&chunk);
            }
            let service = self.clone();
            tokio::task::spawn_blocking(move || {
                service.accept(
                    req.path_params
                        .get("endpoint_id")
                        .ok_or(Failure::Signature)?,
                    &req.headers,
                    &raw,
                )
            })
            .await??;
            // Durable acceptance is enough for 202. Queue publication runs outside
            // the HTTP budget; startup/maintenance/new ingress all drain the outbox.
            Ok::<_, Failure>(())
        })
        .await;
        match result {
            Ok(Ok(())) => HttpResponse {
                status_code: 202,
                body: "durably accepted".into(),
            },
            Ok(Err(Failure::Signature)) => HttpResponse {
                status_code: 401,
                body: "invalid signature or hook".into(),
            },
            Ok(Err(Failure::Oversize)) => HttpResponse {
                status_code: 413,
                body: "payload too large".into(),
            },
            Ok(Err(Failure::Invalid(_) | Failure::Json(_))) => HttpResponse {
                status_code: 400,
                body: "invalid delivery".into(),
            },
            _ => HttpResponse {
                status_code: 503,
                body: "not accepted; operator redelivery required".into(),
            },
        }
    }
}
#[derive(Deserialize, JsonSchema)]
pub struct ReceiveRequest {
    pub headers: HashMap<String, String>,
    pub path_params: HashMap<String, String>,
    pub request_body: StreamChannelRef,
}
#[derive(Serialize, JsonSchema)]
pub struct HttpResponse {
    pub status_code: u16,
    pub body: String,
}
pub fn header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}
pub fn verify_signature(secret: &str, raw: &[u8], signature: &str) -> Result<()> {
    let value = signature
        .strip_prefix("sha256=")
        .ok_or(Failure::Signature)?;
    let bytes = hex::decode(value).map_err(|_| Failure::Signature)?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| Failure::Signature)?;
    mac.update(raw);
    mac.verify_slice(&bytes).map_err(|_| Failure::Signature)
}
