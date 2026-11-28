//! The iii cloud client (spec §2 "Every cloud call"). The credential is
//! resolved once, kept in memory, and never reaches a log, a record, an event,
//! a function result or an error message.
//!
//! A call goes to iii cloud's function for its route (`cloud::<area>::<verb>`, one
//! per operation of `/api/v1`), over a socket to `<cloud_url>/_iii/ws` with the
//! credential as a Bearer on its handshake, and a deployment's events are read
//! from the channel its function answers. It goes over HTTP instead while that
//! socket is not up and listed for the credential, for a route that is no
//! function, a request the function can't carry whole, a credential being
//! tried (not yet the connected one), and when no answer came (nothing ran, or
//! the socket went): the same request, idempotency key included, so a write that
//! did run is not made twice. A call that times out fails as HTTP's own timeout
//! does, and the socket rests (calls over HTTP) for `REST`: a control plane gone
//! leaves its functions unanswered until the engine drops it, about 15 minutes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use iii_sdk::protocol::StreamChannelRef;
use iii_sdk::runtime::{IIIConnectionState, WorkerMetadata};
use iii_sdk::{
    CallRequest, Error as SdkError, IIIClient, InitOptions, InputStream, WorkerIdentityMode,
    register_worker,
};
use reqwest::Method as HttpVerb;
use serde_json::{Map, Value, json};

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
        let verb = req.verb.clone().unwrap_or(HttpVerb::GET);
        let write = verb != HttpVerb::GET;
        // One key for both ways, so a write sent again over HTTP is the same request.
        let key = write.then(|| {
            req.idempotency_key
                .clone()
                .unwrap_or_else(crate::ctx::new_id)
        });
        if let Some(answer) = self
            .by_function(&cfg, cred, &verb, req, key.as_deref())
            .await
        {
            return answer;
        }
        let url = format!("{}{}", cfg.api_base(), req.path);
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
        if let Some(key) = key {
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
        if let Some(events) = self.events_by_function(&cfg, &cred, path, max).await {
            return events;
        }
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
        self.unlink();
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

/// Where the cloud's functions are called.
#[derive(Clone, Debug, PartialEq)]
pub enum FunctionsAt {
    /// `<cloud_url>/_iii/ws`.
    Cloud,
    /// Nowhere: every call over HTTP (the tests that fake iii cloud over HTTP).
    Off,
    /// An engine's socket (a test's).
    Engine(String),
}

/// How long a socket that stopped serving rests (calls over HTTP) before a new one.
pub const REST: Duration = Duration::from_secs(300);
/// How long a new socket has to list the cloud's functions before it rests.
pub const LIST_WITHIN: Duration = Duration::from_secs(30);

/// The socket to the cloud's functions, for one credential.
pub struct Link {
    token: String,
    address: String,
    client: IIIClient,
    /// Filled once the socket has listed the functions.
    routes: Arc<OnceLock<Arc<Vec<Route>>>>,
    /// Since when it doesn't serve (a call timed out, or nothing listed in time).
    down: Arc<Mutex<Option<Instant>>>,
}

impl Link {
    fn close(self) {
        tokio::spawn(async move { self.client.shutdown_async().await });
    }
}

/// A link that serves: its socket, its functions, and how to rest it.
struct Linked {
    client: IIIClient,
    routes: Arc<Vec<Route>>,
    down: Arc<Mutex<Option<Instant>>>,
}

impl Linked {
    fn rest(&self) {
        *lock(&self.down) = Some(Instant::now());
        let client = self.client.clone();
        tokio::spawn(async move { client.shutdown_async().await });
    }
}

/// A lock that a panic elsewhere doesn't poison: the link is only ever swapped whole.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The control plane's own answer about the function rather than the
/// operation: the listing is out of date (a function it no longer has, a field
/// it no longer takes).
pub fn stale((status, body): &(u16, Value)) -> bool {
    let message = body
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    (*status == 404 && message == "No such function.")
        || (*status == 422 && message.ends_with(": not a field of this function."))
}

fn too_slow() -> CloudError {
    CloudError::new(
        "CLOUD_ERROR",
        "iii cloud is unreachable: it took too long to answer.",
    )
}

/// A function of the cloud and the operation it is (its metadata `api`).
#[derive(Debug, PartialEq)]
pub struct Route {
    pub id: String,
    pub method: String,
    /// The path under `/api/v1`, by segment; `{name}` is a parameter.
    pub path: Vec<String>,
    /// The input's field names: the parameters in snake case, and `body`.
    pub fields: Vec<String>,
    /// It answers an event stream as a channel.
    pub events: bool,
}

/// `wss://<host>/_iii/ws` of an `https://<host>` cloud (`ws://` of `http://`).
pub fn engine_address(cloud_url: &str) -> Option<String> {
    let base = cloud_url.trim_end_matches('/');
    let (scheme, rest) = match base.strip_prefix("https://") {
        Some(rest) => ("wss", rest),
        None => ("ws", base.strip_prefix("http://")?),
    };
    Some(format!("{scheme}://{rest}/_iii/ws"))
}

/// The functions an `engine::functions::list` answer names with their operation.
pub fn routes_of(listed: &Value) -> Vec<Route> {
    let functions = listed.get("functions").and_then(Value::as_array);
    functions
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let api = f.pointer("/metadata/api")?;
            let path = api.get("path")?.as_str()?.strip_prefix("/api/v1/")?;
            Some(Route {
                id: f.get("function_id")?.as_str()?.to_owned(),
                method: api.get("method")?.as_str()?.to_uppercase(),
                path: path.split('/').map(str::to_owned).collect(),
                fields: api
                    .get("fields")?
                    .as_array()?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                events: api.get("events") == Some(&Value::Bool(true)),
            })
        })
        .collect()
}

/// The route of `method path` (under `/api/v1`) and its path parameters,
/// decoded. A literal segment beats a parameter, as the API's router does.
pub fn route_of<'a>(
    routes: &'a [Route],
    method: &str,
    path: &str,
) -> Option<(&'a Route, Vec<(String, String)>)> {
    let given: Vec<&str> = path.strip_prefix('/')?.split('/').collect();
    routes
        .iter()
        .filter(|r| r.method == method && r.path.len() == given.len())
        .filter_map(|r| {
            let mut params = Vec::new();
            for (template, segment) in r.path.iter().zip(&given) {
                match template.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
                    Some(name) if !segment.is_empty() => {
                        params.push((name.to_owned(), decoded(segment)?));
                    }
                    Some(_) => return None,
                    None if template != segment => return None,
                    None => {}
                }
            }
            Some((r, params))
        })
        .max_by_key(|(r, _)| {
            r.path
                .iter()
                .map(|s| !s.starts_with('{'))
                .collect::<Vec<_>>()
        })
}

/// A path segment with its `%XX` decoded; `None` when it isn't valid UTF-8 or
/// valid escapes.
fn decoded(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The function's input for `req`: its path parameters, query and headers by
/// field name, and `body`. `None` when the function can't carry it whole (a
/// value it has no field for, which the API would answer 422 to). The
/// idempotency key every write is sent with goes only where the operation takes
/// one: one that declares none (a sign-in step) ignores the header.
pub fn input_of(
    route: &Route,
    params: Vec<(String, String)>,
    req: &Req,
    key: Option<&str>,
) -> Option<Value> {
    let takes = |field: &str| route.fields.iter().any(|f| f == field);
    let mut input = Map::new();
    for (name, value) in params {
        input.insert(name, Value::String(value));
    }
    for (name, value) in &req.query {
        if !takes(name) {
            return None;
        }
        match input.get_mut(name) {
            Some(Value::Array(values)) => values.push(json!(value)),
            Some(first) => *first = json!([first.take(), value]),
            None => {
                input.insert(name.clone(), json!(value));
            }
        }
    }
    if let Some(tag) = &req.if_match {
        if !takes("if_match") {
            return None;
        }
        input.insert("if_match".to_owned(), json!(tag));
    }
    if let Some(key) = key.filter(|_| takes("idempotency_key")) {
        input.insert("idempotency_key".to_owned(), json!(key));
    }
    if let Some(body) = &req.body {
        if !takes("body") {
            return None;
        }
        input.insert("body".to_owned(), body.clone());
    }
    Some(Value::Object(input))
}

/// The API's answer a function's error carries (`{status, body}` in its
/// `stacktrace`).
pub fn answer_of(error: &SdkError) -> Option<(u16, Value)> {
    let SdkError::Remote {
        stacktrace: Some(stacktrace),
        ..
    } = error
    else {
        return None;
    };
    let answer: Value = serde_json::from_str(stacktrace).ok()?;
    let status = u16::try_from(answer.get("status")?.as_u64()?).ok()?;
    (400..=599)
        .contains(&status)
        .then(|| (status, answer.get("body").cloned().unwrap_or(Value::Null)))
}

/// Lists the socket's functions once it is up, again every 2 s until the cloud
/// names some (the control plane registers them after the socket opens), while
/// its link stands. None within `within` (a cloud without `/_iii/ws`, an engine
/// without its functions, a socket refused): it rests, rather than a reconnect
/// and a warning every 2 s.
async fn list(
    client: IIIClient,
    routes: Weak<OnceLock<Arc<Vec<Route>>>>,
    down: Arc<Mutex<Option<Instant>>>,
    within: Duration,
) {
    let started = Instant::now();
    loop {
        let Some(slot) = routes.upgrade() else {
            return;
        };
        if client.get_connection_state() == IIIConnectionState::Connected {
            let listed = client
                .call(CallRequest {
                    function_id: "engine::functions::list".into(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(10_000),
                })
                .await;
            let found = listed.map(|l| routes_of(&l)).unwrap_or_default();
            if !found.is_empty() {
                let _ = slot.set(Arc::new(found));
                return;
            }
        }
        drop(slot);
        if started.elapsed() >= within {
            *lock(&down) = Some(Instant::now());
            client.shutdown_async().await;
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

impl Ctx {
    /// The socket of `cred`'s calls and its functions, once it is up and has
    /// listed them; `None` (HTTP) until then, while it rests, and for a
    /// credential being tried (a key or session `connect` checks before it is
    /// the connected one, whose socket stays as it is). The first call of a
    /// credential opens its socket, closing the last one's.
    fn linked(&self, cfg: &crate::config::Config, cred: &Credential) -> Option<Linked> {
        let connected = lock(&self.cred).as_ref().map(|c| c.token.clone());
        if connected.as_deref() != Some(cred.token.as_str()) {
            return None;
        }
        let address = match &*lock(&self.functions_at) {
            FunctionsAt::Off => return None,
            FunctionsAt::Cloud => engine_address(&cfg.cloud_url)?,
            FunctionsAt::Engine(address) => address.clone(),
        };
        let mut link = lock(&self.link);
        if let Some(l) = link
            .as_ref()
            .filter(|l| l.token == cred.token && l.address == address)
        {
            let down = *lock(&l.down);
            match down {
                None => {
                    let routes = l.routes.get()?.clone();
                    let up = l.client.get_connection_state() == IIIConnectionState::Connected;
                    return up.then(|| Linked {
                        client: l.client.clone(),
                        routes,
                        down: l.down.clone(),
                    });
                }
                Some(at) if at.elapsed() < REST => return None,
                Some(_) => {}
            }
        }
        if let Some(old) = link.take() {
            old.close();
        }
        let client = register_worker(
            &address,
            InitOptions {
                metadata: Some(WorkerMetadata {
                    name: "iii-cloud-worker".into(),
                    ..Default::default()
                }),
                // A second connection of this worker: never the harness's name or
                // namespace (`III_WORKER_NAME`, `III_NAMESPACE`), since the cloud's
                // functions are in its `default`.
                identity: WorkerIdentityMode::Explicit,
                namespace: Some("default".into()),
                headers: Some(HashMap::from([(
                    "authorization".to_owned(),
                    format!("Bearer {}", cred.token),
                )])),
                ..Default::default()
            },
        );
        let routes = Arc::new(OnceLock::new());
        let down = Arc::new(Mutex::new(None));
        tokio::spawn(list(
            client.clone(),
            Arc::downgrade(&routes),
            down.clone(),
            LIST_WITHIN,
        ));
        *link = Some(Link {
            token: cred.token.clone(),
            address,
            client,
            routes,
            down,
        });
        None
    }

    /// Closes the socket of the credential that went.
    pub fn unlink(&self) {
        if let Some(old) = lock(&self.link).take() {
            old.close();
        }
    }

    /// `req` through its function: `Some` with the API's answer (a success as
    /// 200, or 204 when empty) or the timeout's error, `None` for HTTP.
    async fn by_function(
        &self,
        cfg: &crate::config::Config,
        cred: &Credential,
        verb: &HttpVerb,
        req: &Req,
        key: Option<&str>,
    ) -> Option<Res<(u16, Value)>> {
        let linked = self.linked(cfg, cred)?;
        let (route, params) = route_of(&linked.routes, verb.as_str(), &req.path)?;
        if route.events {
            return None;
        }
        let input = input_of(route, params, req, key)?;
        let call = linked
            .client
            .call(CallRequest {
                function_id: route.id.clone(),
                payload: input,
                action: None,
                timeout_ms: Some(self.secs(60).as_millis() as u64),
            })
            .await;
        match call {
            Ok(Value::Null) => Some(Ok((204, Value::Null))),
            Ok(answer) => Some(Ok((200, answer))),
            // Not asked again over HTTP: a read may still run (a plan holds its
            // repository read), as it did past HTTP's own timeout.
            Err(SdkError::Timeout) => {
                linked.rest();
                Some(Err(too_slow()))
            }
            Err(error) => match answer_of(&error) {
                Some(answer) if stale(&answer) => {
                    self.unlink();
                    None
                }
                Some(answer) => Some(Ok(answer)),
                None => {
                    tracing::debug!(function = %route.id, "iii cloud: no answer from the function, over HTTP");
                    None
                }
            },
        }
    }

    /// A deployment's events through its function's channel, as `sse_events`
    /// reads them, within `max` from the call; `None` for HTTP.
    async fn events_by_function(
        &self,
        cfg: &crate::config::Config,
        cred: &Credential,
        path: &str,
        max: Duration,
    ) -> Option<Res<Vec<Value>>> {
        let linked = self.linked(cfg, cred)?;
        let (route, params) = route_of(&linked.routes, "GET", path)?;
        if !route.events {
            return None;
        }
        let input = input_of(route, params, &Req::get(path), None)?;
        let started = Instant::now();
        let answer = linked
            .client
            .call(CallRequest {
                function_id: route.id.clone(),
                payload: input,
                action: None,
                timeout_ms: Some(max.as_millis() as u64),
            })
            .await;
        let answer = match answer {
            Ok(answer) => answer,
            Err(SdkError::Timeout) => {
                linked.rest();
                return Some(Err(too_slow()));
            }
            Err(error) => {
                return match answer_of(&error) {
                    Some(answer) if stale(&answer) => {
                        self.unlink();
                        None
                    }
                    Some((status, body)) => Some(Err(cloud_error(status, &body))),
                    None => None,
                };
            }
        };
        let end: StreamChannelRef = serde_json::from_value(answer.get("events")?.clone()).ok()?;
        let channel = InputStream::from_ref(end, &linked.client).ok()?;
        let mut messages = channel.messages::<Value>();
        let mut events = Vec::new();
        let mut total = 0usize;
        while started.elapsed() < max && total < 2_000_000 {
            let next = tokio::time::timeout(max.saturating_sub(started.elapsed()), messages.next());
            let message = match next.await {
                Ok(Some(Ok(message))) => message,
                // A channel that would not open (its socket refused) is a failed read, as HTTP's.
                Ok(Some(Err(_))) if events.is_empty() => {
                    channel.cancel().await;
                    return Some(Err(CloudError::new(
                        "CLOUD_ERROR",
                        "iii cloud is unreachable: the deployment's events could not be read.",
                    )));
                }
                _ => break,
            };
            let data = message.get("data").cloned().unwrap_or(Value::Null);
            let ping = message.get("event").and_then(Value::as_str) == Some("ping")
                || data.get("type").and_then(Value::as_str) == Some("ping");
            if ping {
                break;
            }
            total += data.to_string().len();
            if !data.is_null() {
                events.push(data);
            }
        }
        // The reader leaving ends the server's stream.
        channel.cancel().await;
        Some(Ok(events))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(id: &str, method: &str, path: &str, fields: &[&str]) -> Route {
        Route {
            id: id.into(),
            method: method.into(),
            path: path
                .trim_start_matches('/')
                .split('/')
                .map(str::to_owned)
                .collect(),
            fields: fields.iter().map(|f| (*f).to_owned()).collect(),
            events: false,
        }
    }

    #[test]
    fn a_cloud_url_has_its_engine_under_iii_ws() {
        assert_eq!(
            engine_address("https://cloud.iii.dev/").as_deref(),
            Some("wss://cloud.iii.dev/_iii/ws")
        );
        assert_eq!(
            engine_address("http://127.0.0.1:8080").as_deref(),
            Some("ws://127.0.0.1:8080/_iii/ws")
        );
        assert_eq!(engine_address("ftp://x"), None);
    }

    #[test]
    fn the_listing_names_each_function_with_its_operation() {
        let listed = json!({ "functions": [
            { "function_id": "cloud::deployments::events", "metadata": { "api": {
                "method": "GET",
                "path": "/api/v1/teams/{team_id}/environments/{environment_id}/deployments/{deployment_id}/events",
                "fields": ["team_id", "environment_id", "deployment_id", "last_event_id"], "events": true } } },
            { "function_id": "engine::functions::list", "metadata": null },
            { "function_id": "cloud::odd", "metadata": { "api": { "method": "GET", "path": "/elsewhere" } } },
        ] });
        let routes = routes_of(&listed);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].id, "cloud::deployments::events");
        assert!(routes[0].events);
        assert_eq!(routes[0].path[0], "teams");
    }

    #[test]
    fn a_path_finds_its_route_a_literal_first_with_its_parameters_decoded() {
        let routes = [
            route(
                "by_id",
                "GET",
                "/teams/{team_id}/projects/{project_id}",
                &[],
            ),
            route("list", "GET", "/teams/{team_id}/projects", &[]),
            route("literal", "GET", "/teams/{team_id}/projects/archived", &[]),
            route("write", "POST", "/teams/{team_id}/projects", &[]),
        ];
        let (r, params) = route_of(&routes, "GET", "/teams/t%201/projects/p1").unwrap();
        assert_eq!(r.id, "by_id");
        assert_eq!(
            params,
            [
                ("team_id".into(), "t 1".into()),
                ("project_id".into(), "p1".into())
            ]
        );
        assert_eq!(
            route_of(&routes, "GET", "/teams/t/projects/archived")
                .unwrap()
                .0
                .id,
            "literal"
        );
        assert_eq!(
            route_of(&routes, "POST", "/teams/t/projects").unwrap().0.id,
            "write"
        );
        assert!(route_of(&routes, "DELETE", "/teams/t/projects").is_none());
        assert!(
            route_of(&routes, "GET", "/teams//projects").is_none(),
            "an empty parameter"
        );
        assert!(
            route_of(&routes, "GET", "/teams/%zz/projects").is_none(),
            "a bad escape"
        );
        assert!(route_of(&routes, "GET", "teams/t/projects").is_none());
    }

    #[test]
    fn the_input_carries_the_request_whole_or_it_goes_over_http() {
        let create = route(
            "create",
            "POST",
            "/teams/{team_id}/projects",
            &["team_id", "idempotency_key", "body", "tag"],
        );
        let req = Req::post("/teams/t/projects", json!({ "name": "a" }))
            .query("tag", "x")
            .query("tag", "y");
        let input = input_of(
            &create,
            vec![("team_id".into(), "t".into())],
            &req,
            Some("k1"),
        )
        .unwrap();
        assert_eq!(
            input,
            json!({ "team_id": "t", "tag": ["x", "y"], "idempotency_key": "k1", "body": { "name": "a" } })
        );
        // What the function has no field for would be a 422 there, and HTTP takes it.
        assert!(
            input_of(
                &create,
                vec![],
                &req.clone().query("other", "1"),
                Some("k1")
            )
            .is_none()
        );
        let read = route("read", "GET", "/teams/{team_id}", &["team_id"]);
        let tagged = Req {
            if_match: Some("\"3\"".into()),
            ..Req::get("/teams/t")
        };
        assert!(input_of(&read, vec![], &tagged, None).is_none());
        assert!(input_of(&read, vec![], &Req::post("/teams/t", json!({})), None).is_none());
        // An operation that takes no idempotency key ignores the header: the key is left out.
        let delete = route("delete", "DELETE", "/teams/{team_id}", &["team_id"]);
        let req = Req {
            verb: Some(HttpVerb::DELETE),
            ..Req::get("/teams/t")
        };
        let params = vec![("team_id".to_owned(), "t".to_owned())];
        assert_eq!(
            input_of(&delete, params, &req, Some("k2")),
            Some(json!({ "team_id": "t" }))
        );
    }

    #[test]
    fn a_function_s_error_carries_the_api_s_answer() {
        let refused = SdkError::Remote {
            code: "CONFLICT".into(),
            message: "Taken.".into(),
            stacktrace: Some(
                json!({ "status": 409, "body": { "code": "CONFLICT", "message": "Taken." } })
                    .to_string(),
            ),
        };
        assert_eq!(
            answer_of(&refused),
            Some((409, json!({ "code": "CONFLICT", "message": "Taken." })))
        );
        let engine = SdkError::Remote {
            code: "FUNCTION_NOT_FOUND".into(),
            message: "Function not found".into(),
            stacktrace: None,
        };
        assert_eq!(answer_of(&engine), None, "nothing ran: HTTP");
        let odd = SdkError::Remote {
            code: "X".into(),
            message: "x".into(),
            stacktrace: Some(json!({ "status": 200 }).to_string()),
        };
        assert_eq!(answer_of(&odd), None);
        assert_eq!(answer_of(&SdkError::Timeout), None);
    }

    #[test]
    fn the_control_plane_s_own_answer_about_a_function_means_the_listing_is_stale() {
        let gone = json!({ "code": "NOT_FOUND", "message": "No such function." });
        assert!(stale(&(404, gone)));
        let field =
            json!({ "code": "VALIDATION_FAILED", "message": "tag: not a field of this function." });
        assert!(stale(&(422, field)));
        // The operation's own 404 and 422 are answers.
        assert!(!stale(&(
            404,
            json!({ "code": "NOT_FOUND", "message": "No such project." })
        )));
        assert!(!stale(&(
            422,
            json!({ "code": "VALIDATION_FAILED", "message": "name: too long." })
        )));
    }
}
