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

    let snapshot = match snapshot {
        Some(Ok(snap)) if !snap.is_ready() && !snap.is_failed() => {
            Some(Ok(wait_ready(&ctx, &cfg, snap).await))
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
    sweep(&ctx).await?;
    let mut shares: Vec<ShareRecord> = ctx
        .store
        .shares()
        .await?
        .into_iter()
        .filter(|s| req.id.as_deref().is_none_or(|id| s.artifact_id == id))
        .collect();
    shares.sort_by_key(|s| std::cmp::Reverse(s.created_at));
    let snapshot = if shares.iter().any(|s| s.public) {
        Some(ctx.tunnel.status(&cfg.tunnel_id).await)
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
    _cfg: Arc<WorkerConfig>,
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
    let tunnel_released = release_if_idle(&ctx).await?;
    Ok(RevokeResponse {
        revoked: !removed.is_empty(),
        tunnel_released,
    })
}

/// Drop expired shares and release the tunnel if nothing public is left.
/// Run by `artifact::shares` and by the background sweeper.
pub async fn sweep(ctx: &Ctx) -> Result<usize, String> {
    let _guard = ctx.store.lock().await;
    let now = unix_now();
    let expired = ctx.store.remove_shares(|s| !s.is_live(now)).await?;
    release_if_idle(ctx).await?;
    Ok(expired.len())
}

/// Release the lease when no live public share remains. Caller holds the
/// store lock. Returns whether a lease was released.
pub(crate) async fn release_if_idle(ctx: &Ctx) -> Result<bool, String> {
    let now = unix_now();
    let busy = ctx
        .store
        .shares()
        .await?
        .iter()
        .any(|s| s.public && s.is_live(now));
    if busy {
        return Ok(false);
    }
    let Some(lease) = ctx.store.lease().await? else {
        return Ok(false);
    };
    ctx.tunnel.release(&lease.lease_id).await?;
    ctx.store.clear_lease().await?;
    tracing::info!(tunnel_id = %lease.tunnel_id, "no public share left; tunnel lease released");
    Ok(true)
}

/// Acquire or stretch the lease to cover `until`. Caller holds the store lock.
async fn ensure_lease(ctx: &Ctx, cfg: &WorkerConfig, until: i64) -> Result<TunnelSnapshot, String> {
    let current = ctx.store.lease().await?;
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
        })
        .await?;
    Ok(snapshot)
}

/// Re-read a starting tunnel until it is ready, failed, or the configured
/// wait runs out; the last snapshot wins either way.
async fn wait_ready(ctx: &Ctx, cfg: &WorkerConfig, mut snap: TunnelSnapshot) -> TunnelSnapshot {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(cfg.tunnel_ready_timeout_ms);
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
