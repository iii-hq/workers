//! `artifact::share`, `shares` and `revoke`: expiring, token-guarded links to
//! one frozen version, plus the quick-tunnel lease that publishes them.
//!
//! Two clocks bound a public link. Each share has its own `expires_at`, which
//! the share server enforces per request. The tunnel lease is stretched to the
//! latest live public share and released as soon as none is left (on revoke,
//! delete, or the sweeper), so the tunnel never stays up for nothing.

use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Ctx;
use super::artifacts::{load, resolve_version};
use crate::config::WorkerConfig;
use crate::store::{LeaseRecord, ShareRecord};
use crate::tunnel::TunnelSnapshot;
use crate::util::{random_token, rfc3339, unix_now};

pub const SHARE_ID: &str = "artifact::share";
pub const SHARE_DESC: &str = "Share one frozen version of an artifact through a secret link that \
     expires after `hours`. `local_url` works on this machine at once; with `public: true` \
     (default) the link is also published through the operator's quick-tunnel and \
     `public_url` is set once the tunnel is ready. Anyone holding the link can view it.";

pub const SHARES_ID: &str = "artifact::shares";
pub const SHARES_DESC: &str = "List the live shares (optionally of one artifact) with their current \
     local and public URLs and expiry, after dropping expired ones. A public URL can change \
     when the tunnel restarts; this always shows the current one.";

pub const REVOKE_ID: &str = "artifact::revoke";
pub const REVOKE_DESC: &str = "Revoke a share by its token so the link stops working at once. \
     Releases the public tunnel when no public share remains.";

/// How often `share` re-reads a starting tunnel's status.
const READY_POLL_MS: u64 = 500;

/// How often `share` asks public DNS about a fresh tunnel hostname.
const DNS_POLL_MS: u64 = 2_000;

/// Tunnel status reported while its new hostname is not in public DNS yet.
pub const WARMING: &str = "warming";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShareRequest {
    /// Artifact id.
    pub id: String,
    /// Version to freeze; omit for the latest. Later updates do not change
    /// what this link shows.
    #[serde(default)]
    pub version: Option<u32>,
    /// Hours until the link expires; defaults to the configured
    /// `default_share_hours` and may not exceed `max_share_hours`.
    #[serde(default)]
    pub hours: Option<u32>,
    /// Publish through the quick-tunnel (default `true`). `false` gives a
    /// local-only link, e.g. to preview in the console browser.
    #[serde(default)]
    pub public: Option<bool>,
}

/// A share as callers see it, with URLs resolved against the current tunnel.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ShareView {
    /// The secret part of the link; pass it to `artifact::revoke`.
    pub token: String,
    pub artifact_id: String,
    pub version: u32,
    pub public: bool,
    /// Creation time, unix seconds.
    pub created_at: i64,
    /// Expiry, unix seconds.
    pub expires_at: i64,
    /// Expiry as RFC 3339 UTC.
    pub expires_at_utc: String,
    /// The link on this machine's share listener.
    pub local_url: String,
    /// The public link, once the tunnel is ready; `null` for a local share or
    /// while the tunnel starts.
    pub public_url: Option<String>,
    /// Tunnel state for a public share: `ready`, `starting`, `failed`, …
    pub tunnel_status: Option<String>,
    /// Why the share is not public yet, when something went wrong.
    pub tunnel_error: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SharesRequest {
    /// Only shares of this artifact.
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SharesResponse {
    pub shares: Vec<ShareView>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RevokeRequest {
    /// The share token (the last path segment of the link).
    pub token: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RevokeResponse {
    /// `false` when no live share had this token.
    pub revoked: bool,
    /// Whether the public tunnel lease was released because no public share
    /// remains.
    pub tunnel_released: bool,
}

pub async fn share(
    ctx: Arc<Ctx>,
    cfg: Arc<WorkerConfig>,
    req: ShareRequest,
) -> Result<ShareView, String> {
    let hours = req.hours.unwrap_or(cfg.default_share_hours);
    if hours == 0 || hours > cfg.max_share_hours {
        return Err(format!(
            "hours must be between 1 and {} (the configured max_share_hours)",
            cfg.max_share_hours
        ));
    }
    let public = req.public.unwrap_or(true);

    let started = tokio::time::Instant::now();
    let (record, snapshot) = {
        let _guard = ctx.store.lock().await;
        let artifact = load(&ctx, &req.id).await?;
        let version = resolve_version(&artifact, req.version)?;
        let now = unix_now();
        let record = ShareRecord {
            token: random_token()?,
            artifact_id: artifact.id,
            version,
            public,
            created_at: now,
            expires_at: now + i64::from(hours) * 3600,
        };
        ctx.store.save_share(&record).await?;
        let snapshot = if public {
            Some(ensure_lease(&ctx, &cfg, record.expires_at).await)
        } else {
            None
        };
        (record, snapshot)
    };

    // One budget covers both waits: the tunnel coming up, then its hostname
    // reaching public DNS.
    let deadline = started + Duration::from_millis(cfg.tunnel_ready_timeout_ms);
    let snapshot = match snapshot {
        Some(Ok(snap)) => {
            let snap = wait_ready(&ctx, &cfg, snap, deadline).await;
            Some(Ok(publishable(&ctx, snap, Some(deadline)).await))
        }
        other => other,
    };
    Ok(view(&ctx, record, snapshot.as_ref()))
}

pub async fn shares(
    ctx: Arc<Ctx>,
    cfg: Arc<WorkerConfig>,
    req: SharesRequest,
) -> Result<SharesResponse, String> {
    sweep(&ctx, &cfg).await?;
    let mut shares: Vec<ShareRecord> = ctx
        .store
        .shares()
        .await?
        .into_iter()
        .filter(|s| req.id.as_deref().is_none_or(|id| s.artifact_id == id))
        .collect();
    shares.sort_by_key(|s| std::cmp::Reverse(s.created_at));
    let snapshot = if shares.iter().any(|s| s.public) {
        Some(match ctx.tunnel.status(&cfg.tunnel_id).await {
            Ok(snap) => Ok(publishable(&ctx, snap, None).await),
            Err(e) => Err(tunnel_hint(&cfg.tunnel_id, &e)),
        })
    } else {
        None
    };
    Ok(SharesResponse {
        shares: shares
            .into_iter()
            .map(|s| view(&ctx, s, snapshot.as_ref()))
            .collect(),
    })
}

pub async fn revoke(
    ctx: Arc<Ctx>,
    cfg: Arc<WorkerConfig>,
    req: RevokeRequest,
) -> Result<RevokeResponse, String> {
    let token = req
        .token
        .trim()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();
    let _guard = ctx.store.lock().await;
    let removed = ctx.store.remove_shares(|s| s.token == token).await?;
    let tunnel_released = release_if_idle(&ctx, &cfg, unix_now()).await?;
    Ok(RevokeResponse {
        revoked: !removed.is_empty(),
        tunnel_released,
    })
}

/// Drop expired shares and release an idle tunnel whose grace has passed.
/// Run by `artifact::shares` and by the background sweeper.
pub async fn sweep(ctx: &Ctx, cfg: &WorkerConfig) -> Result<usize, String> {
    let _guard = ctx.store.lock().await;
    let now = unix_now();
    let expired = ctx.store.remove_shares(|s| !s.is_live(now)).await?;
    release_if_idle(ctx, cfg, now).await?;
    Ok(expired.len())
}

/// Once no live public share remains, keep the tunnel for
/// `tunnel_grace_minutes` (so a share soon after reuses the same,
/// already-resolving address), then release it. A public share in the
/// meantime cancels the countdown. Caller holds the store lock. Returns
/// whether a lease was released.
pub(crate) async fn release_if_idle(
    ctx: &Ctx,
    cfg: &WorkerConfig,
    now: i64,
) -> Result<bool, String> {
    let Some(mut lease) = ctx.store.lease().await? else {
        return Ok(false);
    };
    let busy = ctx
        .store
        .shares()
        .await?
        .iter()
        .any(|s| s.public && s.is_live(now));
    if busy {
        if lease.idle_since.take().is_some() {
            ctx.store.save_lease(&lease).await?;
        }
        return Ok(false);
    }
    let grace = i64::from(cfg.tunnel_grace_minutes) * 60;
    let lapsed = lease.expires_at <= now;
    let idle_since = match lease.idle_since {
        Some(since) => since,
        None if grace > 0 && !lapsed => {
            lease.idle_since = Some(now);
            ctx.store.save_lease(&lease).await?;
            tracing::info!(
                tunnel_id = %lease.tunnel_id,
                grace_minutes = cfg.tunnel_grace_minutes,
                "no public share left; keeping the tunnel for the grace period"
            );
            return Ok(false);
        }
        None => now,
    };
    if !lapsed && now < idle_since + grace {
        return Ok(false);
    }
    ctx.tunnel.release(&lease.lease_id).await?;
    ctx.store.clear_lease().await?;
    tracing::info!(tunnel_id = %lease.tunnel_id, "tunnel lease released");
    Ok(true)
}

/// Acquire or stretch the lease to cover `until` plus the grace period, and
/// cancel any idle countdown. Caller holds the store lock.
async fn ensure_lease(ctx: &Ctx, cfg: &WorkerConfig, until: i64) -> Result<TunnelSnapshot, String> {
    let current = ctx.store.lease().await?;
    let until = until + i64::from(cfg.tunnel_grace_minutes) * 60;
    let until = match &current {
        Some(lease) if lease.tunnel_id == cfg.tunnel_id => until.max(lease.expires_at),
        _ => until,
    };
    let (lease_id, snapshot) = ctx
        .tunnel
        .acquire(&cfg.tunnel_id, until)
        .await
        .map_err(|e| tunnel_hint(&cfg.tunnel_id, &e))?;
    if let Some(old) = current.filter(|l| l.tunnel_id != cfg.tunnel_id) {
        // The operator moved shares to another target; drop the old lease.
        if let Err(e) = ctx.tunnel.release(&old.lease_id).await {
            tracing::warn!(error = %e, "could not release the previous tunnel lease");
        }
    }
    ctx.store
        .save_lease(&LeaseRecord {
            lease_id,
            tunnel_id: cfg.tunnel_id.clone(),
            expires_at: until,
            idle_since: None,
        })
        .await?;
    Ok(snapshot)
}

/// Re-read a starting tunnel until it is ready, failed, or `deadline`; the
/// last snapshot wins either way.
async fn wait_ready(
    ctx: &Ctx,
    cfg: &WorkerConfig,
    mut snap: TunnelSnapshot,
    deadline: tokio::time::Instant,
) -> TunnelSnapshot {
    while !snap.is_ready() && !snap.is_failed() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(READY_POLL_MS)).await;
        match ctx.tunnel.status(&cfg.tunnel_id).await {
            Ok(next) => snap = next,
            Err(e) => {
                tracing::warn!(error = %e, "tunnel status read failed while waiting");
                break;
            }
        }
    }
    snap
}

/// Hand out a ready tunnel's URL only once public DNS knows its hostname;
/// until then report `warming`. With a deadline, keep asking until it.
async fn publishable(
    ctx: &Ctx,
    snap: TunnelSnapshot,
    deadline: Option<tokio::time::Instant>,
) -> TunnelSnapshot {
    let Some(url) = snap.public_url.clone() else {
        return snap;
    };
    loop {
        if ctx.dns.resolves(&url).await {
            return snap;
        }
        let poll = Duration::from_millis(DNS_POLL_MS);
        match deadline {
            Some(deadline) if tokio::time::Instant::now() + poll < deadline => {
                tokio::time::sleep(poll).await;
            }
            _ => break,
        }
    }
    TunnelSnapshot {
        status: WARMING.to_string(),
        public_url: None,
        error: None,
    }
}

fn tunnel_hint(tunnel_id: &str, error: &str) -> String {
    if error.contains("unauthorized tunnel_id") || error.contains("unknown tunnel_id") {
        format!(
            "quick-tunnel has no `{tunnel_id}` target; the operator must add `{tunnel_id}: \
             http://<listen_addr>` to the quick-tunnel targets ({error})"
        )
    } else if error.contains("function_not_found") || error.contains("not found") {
        format!("the quick-tunnel worker is not available ({error})")
    } else {
        error.to_string()
    }
}

fn view(
    ctx: &Ctx,
    share: ShareRecord,
    snapshot: Option<&Result<TunnelSnapshot, String>>,
) -> ShareView {
    let path = format!("/s/{}", share.token);
    let (public_url, tunnel_status, tunnel_error) = match (share.public, snapshot) {
        (false, _) | (true, None) => (None, None, None),
        (true, Some(Ok(snap))) => (
            snap.public_url
                .as_deref()
                .map(|base| format!("{}{path}", base.trim_end_matches('/'))),
            Some(snap.status.clone()),
            snap.error.clone(),
        ),
        (true, Some(Err(e))) => (None, None, Some(e.clone())),
    };
    ShareView {
        local_url: format!("{}{path}", ctx.local_base),
        expires_at_utc: rfc3339(share.expires_at),
        token: share.token,
        artifact_id: share.artifact_id,
        version: share.version,
        public: share.public,
        created_at: share.created_at,
        expires_at: share.expires_at,
        public_url,
        tunnel_status,
        tunnel_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::artifacts::{self, CreateRequest, DeleteRequest};

    fn cfg() -> Arc<WorkerConfig> {
        Arc::new(WorkerConfig {
            tunnel_ready_timeout_ms: 0,
            tunnel_grace_minutes: 0,
            ..WorkerConfig::default()
        })
    }

    async fn artifact(ctx: &Arc<Ctx>) -> String {
        artifacts::create(
            ctx.clone(),
            cfg(),
            CreateRequest {
                title: "proto".into(),
                kind: None,
                format: None,
                content: "<h1>v1</h1>".into(),
                note: None,
            },
        )
        .await
        .expect("creates")
        .id
    }

    fn req(id: &str, public: bool) -> ShareRequest {
        ShareRequest {
            id: id.to_string(),
            version: None,
            hours: Some(2),
            public: Some(public),
        }
    }

    #[tokio::test]
    async fn a_public_share_leases_the_tunnel_and_builds_both_urls() {
        let ctx = Ctx::for_tests(Some("https://x.trycloudflare.com/"));
        let id = artifact(&ctx).await;
        let view = share(ctx.clone(), cfg(), req(&id, true))
            .await
            .expect("shares");
        assert_eq!(view.version, 1);
        assert_eq!(
            view.local_url,
            format!("http://127.0.0.1:3120/s/{}", view.token)
        );
        assert_eq!(
            view.public_url.as_deref(),
            Some(format!("https://x.trycloudflare.com/s/{}", view.token).as_str())
        );
        assert_eq!(view.expires_at - view.created_at, 2 * 3600);
        let acquired = ctx.tunnel.with_fake(|f| f.acquired.clone());
        assert_eq!(acquired, vec![("artifacts".to_string(), view.expires_at)]);
    }

    #[tokio::test]
    async fn a_local_share_never_touches_the_tunnel() {
        let ctx = Ctx::for_tests(Some("https://x.trycloudflare.com"));
        let id = artifact(&ctx).await;
        let view = share(ctx.clone(), cfg(), req(&id, false))
            .await
            .expect("shares");
        assert_eq!(view.public_url, None);
        assert!(ctx.tunnel.with_fake(|f| f.acquired.is_empty()));
    }

    #[tokio::test]
    async fn the_lease_never_shrinks_below_the_longest_share() {
        let ctx = Ctx::for_tests(Some("https://x.trycloudflare.com"));
        let id = artifact(&ctx).await;
        let long = share(
            ctx.clone(),
            cfg(),
            ShareRequest {
                hours: Some(10),
                ..req(&id, true)
            },
        )
        .await
        .expect("long");
        share(ctx.clone(), cfg(), req(&id, true))
            .await
            .expect("short");
        let acquired = ctx.tunnel.with_fake(|f| f.acquired.clone());
        assert_eq!(acquired[1].1, long.expires_at);
    }

    #[tokio::test]
    async fn revoking_the_last_public_share_releases_the_tunnel() {
        let ctx = Ctx::for_tests(Some("https://x.trycloudflare.com"));
        let id = artifact(&ctx).await;
        let a = share(ctx.clone(), cfg(), req(&id, true)).await.expect("a");
        let b = share(ctx.clone(), cfg(), req(&id, true)).await.expect("b");

        let first = revoke(ctx.clone(), cfg(), RevokeRequest { token: a.token })
            .await
            .expect("revokes");
        assert!(first.revoked && !first.tunnel_released);

        let last = revoke(ctx.clone(), cfg(), RevokeRequest { token: b.local_url })
            .await
            .expect("revokes by full link too");
        assert!(last.revoked && last.tunnel_released);
        assert_eq!(ctx.tunnel.with_fake(|f| f.released.len()), 1);
        assert!(ctx.store.lease().await.expect("lease").is_none());
    }

    #[tokio::test]
    async fn a_missing_tunnel_target_still_yields_a_local_link_with_a_hint() {
        let ctx = Ctx::for_tests(None);
        ctx.tunnel.with_fake(|f| {
            f.fail = Some("invalid request: invalid consumer_id or unauthorized tunnel_id".into())
        });
        let id = artifact(&ctx).await;
        let view = share(ctx, cfg(), req(&id, true))
            .await
            .expect("still shares");
        assert_eq!(view.public_url, None);
        assert!(view.local_url.contains(&view.token));
        assert!(
            view.tunnel_error
                .as_deref()
                .is_some_and(|e| e.contains("targets"))
        );
    }

    #[tokio::test]
    async fn share_rejects_bad_hours_and_unknown_versions() {
        let ctx = Ctx::for_tests(None);
        let id = artifact(&ctx).await;
        let too_long = ShareRequest {
            hours: Some(73),
            ..req(&id, false)
        };
        assert!(share(ctx.clone(), cfg(), too_long).await.is_err());
        let zero = ShareRequest {
            hours: Some(0),
            ..req(&id, false)
        };
        assert!(share(ctx.clone(), cfg(), zero).await.is_err());
        let bad_version = ShareRequest {
            version: Some(5),
            ..req(&id, false)
        };
        assert!(share(ctx, cfg(), bad_version).await.is_err());
    }

    #[tokio::test]
    async fn deleting_an_artifact_revokes_its_shares_and_frees_the_tunnel() {
        let ctx = Ctx::for_tests(Some("https://x.trycloudflare.com"));
        let id = artifact(&ctx).await;
        share(ctx.clone(), cfg(), req(&id, true))
            .await
            .expect("shares");
        let out = artifacts::delete(ctx.clone(), cfg(), DeleteRequest { id: id.clone() })
            .await
            .expect("deletes");
        assert_eq!(out.revoked_shares, 1);
        assert!(out.tunnel_released);
        let listed = shares(ctx, cfg(), SharesRequest { id: Some(id) })
            .await
            .expect("lists");
        assert!(listed.shares.is_empty());
    }

    fn grace_cfg() -> Arc<WorkerConfig> {
        Arc::new(WorkerConfig {
            tunnel_ready_timeout_ms: 0,
            tunnel_grace_minutes: 30,
            ..WorkerConfig::default()
        })
    }

    #[tokio::test]
    async fn the_lease_covers_the_share_plus_the_grace_period() {
        let ctx = Ctx::for_tests(Some("https://x.trycloudflare.com"));
        let id = artifact(&ctx).await;
        let view = share(ctx.clone(), grace_cfg(), req(&id, true))
            .await
            .expect("shares");
        let acquired = ctx.tunnel.with_fake(|f| f.acquired.clone());
        assert_eq!(acquired[0].1, view.expires_at + 30 * 60);
    }

    #[tokio::test]
    async fn the_tunnel_outlives_the_last_share_by_the_grace_period() {
        let ctx = Ctx::for_tests(Some("https://x.trycloudflare.com"));
        let id = artifact(&ctx).await;
        let link = share(ctx.clone(), grace_cfg(), req(&id, true))
            .await
            .expect("shares");

        let out = revoke(
            ctx.clone(),
            grace_cfg(),
            RevokeRequest { token: link.token },
        )
        .await
        .expect("revokes");
        assert!(
            out.revoked && !out.tunnel_released,
            "kept for the grace period"
        );
        let lease = ctx.store.lease().await.expect("lease").expect("still held");
        let idle_since = lease.idle_since.expect("countdown started");

        let early = release_if_idle(&ctx, &grace_cfg(), idle_since + 29 * 60)
            .await
            .expect("early");
        assert!(!early);
        let late = release_if_idle(&ctx, &grace_cfg(), idle_since + 30 * 60)
            .await
            .expect("late");
        assert!(late);
        assert!(ctx.store.lease().await.expect("lease").is_none());
    }

    #[tokio::test]
    async fn a_new_public_share_cancels_the_idle_countdown() {
        let ctx = Ctx::for_tests(Some("https://x.trycloudflare.com"));
        let id = artifact(&ctx).await;
        let first = share(ctx.clone(), grace_cfg(), req(&id, true))
            .await
            .expect("first");
        revoke(
            ctx.clone(),
            grace_cfg(),
            RevokeRequest { token: first.token },
        )
        .await
        .expect("revokes");
        let second = share(ctx.clone(), grace_cfg(), req(&id, true))
            .await
            .expect("second");
        assert_eq!(
            second
                .public_url
                .as_deref()
                .map(|u| u.starts_with("https://x.")),
            Some(true)
        );
        let lease = ctx.store.lease().await.expect("lease").expect("held");
        assert_eq!(lease.idle_since, None);
        assert!(ctx.tunnel.with_fake(|f| f.released.is_empty()));
    }

    #[tokio::test]
    async fn a_hostname_not_in_public_dns_yet_is_reported_as_warming() {
        let base = Ctx::for_tests(Some("https://fresh.trycloudflare.com"));
        let ctx = Arc::new(Ctx {
            store: base.store.clone(),
            tunnel: base.tunnel.clone(),
            dns: Arc::new(crate::dns::Dns::fake(false)),
            local_base: base.local_base.clone(),
        });
        let id = artifact(&ctx).await;
        let view = share(ctx.clone(), cfg(), req(&id, true))
            .await
            .expect("shares");
        assert_eq!(view.public_url, None);
        assert_eq!(view.tunnel_status.as_deref(), Some(WARMING));
        assert!(view.local_url.contains(&view.token));
    }

    #[tokio::test]
    async fn shares_drops_expired_links_and_lists_live_ones() {
        let ctx = Ctx::for_tests(None);
        let id = artifact(&ctx).await;
        let live = share(ctx.clone(), cfg(), req(&id, false))
            .await
            .expect("live");
        ctx.store
            .save_share(&ShareRecord {
                token: "e".repeat(64),
                artifact_id: id.clone(),
                version: 1,
                public: false,
                created_at: 0,
                expires_at: 1,
            })
            .await
            .expect("expired");
        let listed = shares(ctx.clone(), cfg(), SharesRequest::default())
            .await
            .expect("lists");
        assert_eq!(listed.shares.len(), 1);
        assert_eq!(listed.shares[0].token, live.token);
        assert!(
            ctx.store
                .load_share(&"e".repeat(64))
                .await
                .expect("load")
                .is_none()
        );
    }
}
