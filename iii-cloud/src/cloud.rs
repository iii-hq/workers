//! The iii cloud HTTP client (spec §2 "Every cloud call"). The credential is
//! resolved once, kept in memory, and never reaches a log, a record, an event,
//! a function result or an error message.

use reqwest::Method as HttpVerb;
use serde_json::Value;

use crate::ctx::{Credential, Ctx, SESSION_SECRET};
use crate::error::{CloudError, Res, cut};
use crate::types::*;

/// One request to `<cloud_url>/api/v1`.
#[derive(Default, Clone)]
pub struct Req {
    pub verb: Option<HttpVerb>,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Option<Value>,
    pub if_match: Option<String>,
    pub idempotency_key: Option<String>,
}

impl Req {
    pub fn get(path: impl Into<String>) -> Self {
        Self {
            verb: Some(HttpVerb::GET),
            path: path.into(),
            ..Default::default()
        }
    }

    pub fn post(path: impl Into<String>, body: Value) -> Self {
        Self {
            verb: Some(HttpVerb::POST),
            path: path.into(),
            body: Some(body),
            ..Default::default()
        }
    }

    pub fn patch(path: impl Into<String>, body: Value) -> Self {
        Self {
            verb: Some(HttpVerb::PATCH),
            ..Self::post(path, body)
        }
    }

    pub fn query(mut self, key: &str, value: impl Into<String>) -> Self {
        self.query.push((key.to_string(), value.into()));
        self
    }
}

/// The cloud's error envelope `{code, message, fix?}` as a `CLOUD_ERROR`.
pub fn cloud_error(status: u16, body: &Value) -> CloudError {
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_owned);
    let message = text("message").unwrap_or_else(|| format!("iii cloud answered {status}"));
    let message = match text("code") {
        Some(code) => format!("{message} ({code})"),
        None => message,
    };
    let code = match text("code").as_deref() {
        Some("IDEMPOTENCY_KEY_REUSED") => "IDEMPOTENCY_KEY_REUSED",
        _ => "CLOUD_ERROR",
    };
    CloudError::new(code, message)
        .with_status(status)
        .with_fix(text("fix"))
}

/// A key's refusal of a route it can't call (`keys.rs:497-503`).
pub fn is_route_refusal(body: &Value) -> bool {
    body.get("message")
        .and_then(Value::as_str)
        .is_some_and(|m| m.contains("can't call this route"))
}

impl Ctx {
    /// The credential: in memory, else read from the vault once.
    pub async fn credential(&self) -> Res<Credential> {
        if let Some(c) = self.cred.lock().unwrap().clone() {
            return Ok(c);
        }
        let cfg = self.config().await;
        let conn = self.connection(&cfg);
        if conn.status != ConnStatus::Connected {
            return Err(CloudError::not_connected());
        }
        let (method, name) = match (conn.method, conn.api_key) {
            (Some(Method::Browser), _) => (Method::Browser, SESSION_SECRET.to_string()),
            (Some(Method::ApiKey), Some(key)) => (Method::ApiKey, key.secret),
            _ => return Err(CloudError::not_connected()),
        };
        let token = self
            .plat
            .secret_resolve(&name)
            .await?
            .filter(|t| !t.is_empty())
            .ok_or_else(|| self.credential_gone(method))?;
        let cred = Credential { method, token };
        *self.cred.lock().unwrap() = Some(cred.clone());
        Ok(cred)
    }

    fn credential_gone(&self, method: Method) -> CloudError {
        match method {
            Method::Browser => CloudError::not_connected(),
            Method::ApiKey => key_rejected(),
        }
    }

    /// Send with an explicit credential. No auth handling: 401 comes back as is.
    pub async fn send(&self, cred: &Credential, req: &Req) -> Res<(u16, Value)> {
        let cfg = self.config().await;
        let url = format!("{}{}", cfg.api_base(), req.path);
        let verb = req.verb.clone().unwrap_or(HttpVerb::GET);
        let write = verb != HttpVerb::GET;
        let mut builder = self
            .http
            .request(verb, url)
            .bearer_auth(&cred.token)
            .query(&req.query);
        if let Some(body) = &req.body {
            builder = builder.json(body);
        }
        if let Some(tag) = &req.if_match {
            builder = builder.header("If-Match", tag);
        }
        if write {
            let key = req
                .idempotency_key
                .clone()
                .unwrap_or_else(crate::ctx::new_id);
            builder = builder.header("Idempotency-Key", key);
        }
        let response = builder.send().await.map_err(|e| {
            CloudError::new(
                "CLOUD_ERROR",
                format!(
                    "iii cloud is unreachable: {}",
                    cut(&e.without_url().to_string(), 200)
                ),
            )
        })?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        let body = serde_json::from_str(&text).unwrap_or_else(|_| {
            if text.trim().is_empty() {
                Value::Null
            } else {
                serde_json::json!({ "message": cut(text.trim(), 200) })
            }
        });
        Ok((status, body))
    }

    /// Send as the connected credential. A 401 sets the connection `expired`,
    /// except a key's refusal of a route, which is `NEEDS_BROWSER_SIGN_IN`.
    /// Other statuses come back for the caller to read.
    pub async fn raw(&self, req: &Req) -> Res<(u16, Value)> {
        let cred = self.credential().await?;
        let (status, body) = self.send(&cred, req).await?;
        if status == 401 {
            if cred.method == Method::ApiKey && is_route_refusal(&body) {
                return Err(CloudError::new(
                    "NEEDS_BROWSER_SIGN_IN",
                    "This route needs a person's session; the API key can't call it.",
                )
                .with_status(401));
            }
            // A 401 is not yet "expired": cookie-only routes answer a valid token
            // with it too. Ask a route the credential can always call.
            if !self.still_valid(&cred, req).await {
                return Err(self.expire(cred.method).await);
            }
            return Err(match cred.method {
                Method::Browser => CloudError::new(
                    "BROWSER_SESSION_ONLY",
                    "This route needs a browser session; use the console.",
                ),
                Method::ApiKey => cloud_error(401, &body),
            }
            .with_status(401));
        }
        Ok((status, body))
    }

    /// After a 401: is the credential itself still good? A session is asked
    /// with `GET /me`, a key with the environment it was connected to. Only a
    /// 401 there means expired; an answer that is not a clear yes or no (the
    /// cloud is down) keeps the connection.
    async fn still_valid(&self, cred: &Credential, original: &Req) -> bool {
        let probe = match cred.method {
            Method::Browser => "/me".to_string(),
            Method::ApiKey => {
                let cfg = self.config().await;
                match self.connection(&cfg).api_key {
                    Some(k) => format!("/teams/{}/environments/{}", k.team_id, k.environment_id),
                    None => return false,
                }
            }
        };
        if original.path == probe {
            return false;
        }
        match self.send(cred, &Req::get(probe)).await {
            Ok((401, _)) => false,
            Ok(_) | Err(_) => true,
        }
    }

    /// A 2xx body, or `CLOUD_ERROR` with the cloud's status.
    pub async fn call(&self, req: &Req) -> Res<Value> {
        let (status, body) = self.raw(req).await?;
        if (200..300).contains(&status) {
            Ok(body)
        } else {
            Err(cloud_error(status, &body))
        }
    }

    /// The events of a deployment (Server-Sent Events): what the cloud kept,
    /// read until its first `ping` or `max` of waiting. Each `data:` line is
    /// one DeploymentEvent.
    pub async fn sse_events(&self, path: &str, max: std::time::Duration) -> Res<Vec<Value>> {
        let cred = self.credential().await?;
        let cfg = self.config().await;
        let mut response = self
            .http
            .get(format!("{}{path}", cfg.api_base()))
            .bearer_auth(&cred.token)
            .header("Accept", "text/event-stream")
            .timeout(max + std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| {
                CloudError::new(
                    "CLOUD_ERROR",
                    format!(
                        "iii cloud is unreachable: {}",
                        cut(&e.without_url().to_string(), 200)
                    ),
                )
            })?;
        if !response.status().is_success() {
            return Err(cloud_error(response.status().as_u16(), &Value::Null));
        }
        // Bytes up to each newline, then one `data:` line at a time: a chunk can
        // end mid-line (or mid-character), and the JSON may be spaced.
        let mut pending: Vec<u8> = Vec::new();
        let mut events = Vec::new();
        let mut total = 0usize;
        let started = std::time::Instant::now();
        'read: while started.elapsed() < max && total < 2_000_000 {
            match tokio::time::timeout(max.saturating_sub(started.elapsed()), response.chunk())
                .await
            {
                Ok(Ok(Some(chunk))) => {
                    total += chunk.len();
                    pending.extend_from_slice(&chunk);
                    while let Some(at) = pending.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=at).collect();
                        let line = String::from_utf8_lossy(&line);
                        let Some(data) = line.trim_end().strip_prefix("data:") else {
                            continue;
                        };
                        let Ok(event) = serde_json::from_str::<Value>(data.trim()) else {
                            continue;
                        };
                        if event.get("type").and_then(Value::as_str) == Some("ping") {
                            break 'read;
                        }
                        events.push(event);
                    }
                }
                _ => break,
            }
        }
        Ok(events)
    }

    pub async fn expire(&self, method: Method) -> CloudError {
        *self.cred.lock().unwrap() = None;
        let error = match method {
            Method::Browser => CloudError::not_connected(),
            Method::ApiKey => key_rejected(),
        };
        let cfg = self.config().await;
        let mut conn = self.connection(&cfg);
        if conn.status == ConnStatus::Connected {
            conn.status = ConnStatus::Expired;
            conn.error = Some(error.to_worker_error());
            self.set_connection(conn);
        }
        error
    }
}

pub fn key_rejected() -> CloudError {
    CloudError::new("KEY_REJECTED", "iii cloud rejected the API key.")
}
