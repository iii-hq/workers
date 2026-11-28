//! `iii-cloud::connect` (spec §2): the browser device flow and the API key.

use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};

use crate::cloud::{Req, cloud_error, is_route_refusal, key_rejected};
use crate::ctx::{Credential, Ctx, SESSION_SECRET};
use crate::error::{CloudError, Res, cut};
use crate::types::*;

fn uuid_ok(label: &str, value: &str) -> Res<()> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| CloudError::invalid(format!("{label} must be a UUID")))
}

fn secret_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 128
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

pub async fn connect(ctx: &Arc<Ctx>, request: ConnectRequest) -> Res<Connection> {
    match request {
        ConnectRequest::Browser => start_browser(ctx).await,
        ConnectRequest::ApiKey {
            secret,
            team_id,
            environment_id,
        } => connect_key(ctx, secret, team_id, environment_id).await,
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/proc/sys/kernel/hostname").ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "this machine".into())
}

/// Unicode format characters (category Cf): zero-width and direction marks, BOM, tags.
fn is_format(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
        | 0xFEFF | 0xFFF9..=0xFFFB | 0xE0001 | 0xE0020..=0xE007F)
}

/// The device name: at most 64 characters, control and format characters removed.
pub fn device_name(host: &str) -> String {
    format!("iii-cloud worker on {host}")
        .chars()
        .filter(|c| !c.is_control() && !is_format(*c))
        .take(64)
        .collect()
}

async fn start_browser(ctx: &Arc<Ctx>) -> Res<Connection> {
    let cfg = ctx.config().await;
    if let Some(handle) = ctx.poller.lock().unwrap().take() {
        handle.abort();
    }
    let url = format!("{}/auth/device", cfg.api_base());
    let response = ctx
        .http
        .post(url)
        .json(&json!({ "name": device_name(&hostname()), "platform": "worker" }))
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
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    // Only the cloud saying the platform is not accepted means "no worker sign-in
    // yet"; any other 422 (a bad device name) keeps the cloud's own code.
    let says_platform = body
        .get("message")
        .and_then(Value::as_str)
        .is_some_and(|m| m.to_lowercase().contains("platform"));
    if status == 422 && says_platform {
        return Err(CloudError::new(
            "BROWSER_SIGN_IN_UNAVAILABLE",
            "This iii cloud does not accept worker sign-in yet; use an API key.",
        )
        .with_status(422));
    }
    if status != 200 {
        return Err(cloud_error(status, &body));
    }
    let text = |k: &str| {
        body.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let (device_code, user_code, uri) = (
        text("device_code"),
        text("user_code"),
        text("verification_uri"),
    );
    if device_code.is_empty() || user_code.is_empty() {
        return Err(CloudError::new(
            "CLOUD_ERROR",
            "iii cloud sent no device code.",
        ));
    }
    let expires_in = body
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(600);
    let interval = body.get("interval").and_then(Value::as_u64).unwrap_or(5);
    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(expires_in as i64))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // The worker never builds a link with the code in it.
    let mut conn = Connection::signed_out(&cfg.cloud_url);
    conn.status = ConnStatus::Waiting;
    conn.method = Some(Method::Browser);
    conn.device = Some(Device {
        user_code,
        verification_uri: uri,
        expires_at,
    });
    ctx.set_connection(conn.clone());
    ctx.store
        .update(|s| s.pending_device = Some((device_code.clone(), interval)));
    let task = tokio::spawn(poll(ctx.clone(), device_code, expires_in, interval));
    *ctx.poller.lock().unwrap() = Some(task);
    Ok(conn)
}

/// After a restart, a sign-in that was waiting either goes on polling until its
/// `expires_at` or, when that has passed (or its code was not kept), expires.
pub fn resume(ctx: &Arc<Ctx>) {
    let (conn, pending) = ctx
        .store
        .read(|s| (s.connection.clone(), s.pending_device.clone()));
    let Some(conn) = conn.filter(|c| c.status == ConnStatus::Waiting) else {
        return;
    };
    let remaining = conn
        .device
        .as_ref()
        .and_then(|d| chrono::DateTime::parse_from_rfc3339(&d.expires_at).ok())
        .map(|t| (t.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds())
        .unwrap_or(0);
    match pending {
        Some((device_code, interval)) if remaining > 0 => {
            let task = tokio::spawn(poll(ctx.clone(), device_code, remaining as u64, interval));
            *ctx.poller.lock().unwrap() = Some(task);
        }
        _ => {
            let mut expired = Connection::signed_out(&conn.cloud_url);
            expired.status = ConnStatus::Error;
            expired.error = Some(
                CloudError::new(
                    "DEVICE_CODE_EXPIRED",
                    "The sign-in code expired. Start again.",
                )
                .to_worker_error(),
            );
            ctx.set_connection(expired);
        }
    }
}

/// Poll `/auth/device/token` until a token, an end, or `expires_in`.
async fn poll(ctx: Arc<Ctx>, device_code: String, expires_in: u64, mut interval: u64) {
    let started = Instant::now();
    let deadline = ctx.secs(expires_in);
    let cfg = ctx.config().await;
    let fail = |ctx: &Ctx, error: CloudError| {
        let mut conn = Connection::signed_out(&cfg.cloud_url);
        conn.status = ConnStatus::Error;
        conn.error = Some(error.to_worker_error());
        ctx.set_connection(conn);
    };
    loop {
        tokio::time::sleep(ctx.secs(interval)).await;
        if started.elapsed() > deadline {
            return fail(
                &ctx,
                CloudError::new(
                    "DEVICE_CODE_EXPIRED",
                    "The sign-in code expired. Start again.",
                ),
            );
        }
        let url = format!("{}/auth/device/token", cfg.api_base());
        let answer = ctx
            .http
            .post(url)
            .json(&json!({ "device_code": device_code }))
            .send()
            .await;
        let Ok(response) = answer else { continue };
        let status = response.status().as_u16();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let code = body.get("code").and_then(Value::as_str).unwrap_or_default();
        match (status, code) {
            (200, _) => return finish(&ctx, &body).await,
            (429, _) => interval += 5,
            (_, "AUTHORIZATION_PENDING") => {}
            (_, "DEVICE_CODE_EXPIRED" | "ACCOUNT_BLOCKED") => {
                let message = body
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("The sign-in ended.");
                return fail(&ctx, CloudError::new(code, message).with_status(status));
            }
            (s, _) if s >= 500 => {}
            _ => return fail(&ctx, cloud_error(status, &body)),
        }
    }
}

async fn finish(ctx: &Arc<Ctx>, body: &Value) {
    let cfg = ctx.config().await;
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let expires_at = body
        .get("expires_at")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let result: Res<Connection> = async {
        let current = ctx.plat.secret_resolve(SESSION_SECRET).await?;
        if !ctx
            .plat
            .secret_cas(SESSION_SECRET, current.as_deref(), &token)
            .await?
        {
            return Err(CloudError::new(
                "INVALID_REQUEST",
                "The vault changed while signing in. Sign in again.",
            ));
        }
        let cred = Credential {
            method: Method::Browser,
            token,
        };
        let (status, me) = ctx.send(&cred, &Req::get("/me")).await?;
        if status != 200 {
            return Err(cloud_error(status, &me));
        }
        *ctx.cred.lock().unwrap() = Some(cred);
        let mut conn = Connection::signed_out(&cfg.cloud_url);
        conn.status = ConnStatus::Connected;
        conn.method = Some(Method::Browser);
        conn.user = parse_user(&me);
        conn.teams = parse_teams(&me);
        conn.expires_at = expires_at;
        Ok(conn)
    }
    .await;
    match result {
        Ok(conn) => ctx.set_connection(conn),
        Err(error) => {
            let mut conn = Connection::signed_out(&cfg.cloud_url);
            conn.status = ConnStatus::Error;
            conn.error = Some(error.to_worker_error());
            ctx.set_connection(conn);
        }
    }
}

fn parse_user(me: &Value) -> Option<User> {
    let u = me.get("user")?;
    Some(User {
        id: u.get("id")?.as_str()?.to_string(),
        email: u.get("email")?.as_str()?.to_string(),
        name: u.get("name").and_then(Value::as_str).map(str::to_owned),
    })
}

fn parse_teams(me: &Value) -> Vec<TeamRole> {
    me.get("teams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let s = |k: &str| t.get(k).and_then(Value::as_str).map(str::to_owned);
            Some(TeamRole {
                id: s("id")?,
                name: s("name")?,
                slug: s("slug")?,
                role: s("role").unwrap_or_default(),
            })
        })
        .collect()
}

async fn connect_key(
    ctx: &Arc<Ctx>,
    secret: String,
    team_id: Option<String>,
    environment_id: Option<String>,
) -> Res<Connection> {
    if !secret_name_ok(&secret) {
        return Err(CloudError::invalid("secret must be a vault secret name"));
    }
    let cfg = ctx.config().await;
    let token = ctx
        .plat
        .secret_resolve(&secret)
        .await?
        .filter(|t| !t.is_empty())
        .ok_or_else(|| CloudError::invalid(format!("the vault has no secret named {secret}")))?;
    let cred = Credential {
        method: Method::ApiKey,
        token,
    };

    // The key tells its own team and environment; ids sent by the caller are
    // the fallback for a cloud that answers 404 (or its route refusal).
    let (status, body) = ctx.send(&cred, &Req::get("/keys/self")).await?;
    let (team, environment) = match status {
        200 => (
            body.get("team_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            body.get("environment_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
        404 => (team_id, environment_id),
        401 if is_route_refusal(&body) => (team_id, environment_id),
        401 => return Err(key_rejected()),
        _ => return Err(cloud_error(status, &body)),
    };
    let (Some(team), Some(environment)) = (team, environment) else {
        return Err(CloudError::invalid(
            "this iii cloud can't tell the key's environment; send team_id and environment_id",
        ));
    };
    uuid_ok("team_id", &team)?;
    uuid_ok("environment_id", &environment)?;

    let (status, body) = ctx
        .send(
            &cred,
            &Req::get(format!("/teams/{team}/environments/{environment}")),
        )
        .await?;
    match status {
        200 => {}
        401 => return Err(key_rejected()),
        404 => {
            return Err(CloudError::new(
                "KEY_WRONG_ENVIRONMENT",
                "The API key can't read that team and environment.",
            )
            .with_status(404));
        }
        _ => return Err(cloud_error(status, &body)),
    }
    if let Some(handle) = ctx.poller.lock().unwrap().take() {
        handle.abort();
    }
    *ctx.cred.lock().unwrap() = Some(cred);
    let mut conn = Connection::signed_out(&cfg.cloud_url);
    conn.status = ConnStatus::Connected;
    conn.method = Some(Method::ApiKey);
    conn.api_key = Some(ApiKeyRef {
        secret,
        team_id: team,
        environment_id: environment,
    });
    ctx.set_connection(conn.clone());
    Ok(conn)
}
