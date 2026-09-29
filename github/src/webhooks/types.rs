use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebhookConfig {
    /// Default delivery policy; individual subscriptions can override it. Hot-reloaded.
    pub notifications: super::notifications::NotificationPolicy,
    /// Opt in only after configuring the durable queue and public HTTP listener.
    pub enabled: bool,
    /// Private, persistent directory; not a shared/network filesystem.
    pub storage_path: String,
    pub tunnel_id: String,
    pub queue: String,
    pub max_body_bytes: usize,
    pub max_pending: usize,
    /// Longest watch lifetime in days (1..=30); new watches must expire within
    /// it. Capped at 30 because every watch holds a quick-tunnel lease and
    /// quick-tunnel accepts leases of at most 30 days. Hot-reloaded.
    #[schemars(range(min = 1, max = 30))]
    pub max_watch_days: u32,
    /// Minutes a watch may stay without any github::pr::event binding before it
    /// is stopped (0..=1440). One-shot agent wakes unregister right after they
    /// fire and are re-armed at the end of the turn, so keep this longer than a
    /// turn. 0 stops immediately. Hot-reloaded.
    #[schemars(range(max = 1440))]
    pub orphan_grace_minutes: u32,
}
/// Upper bound for `orphan_grace_minutes` (one day).
pub const MAX_ORPHAN_GRACE_MINUTES: u32 = 1440;
/// Hard cap for `max_watch_days`: the quick-tunnel lease limit (30 days).
pub const MAX_WATCH_DAYS: u32 = 30;
pub fn valid_watch_days(days: u32) -> bool {
    (1..=MAX_WATCH_DAYS).contains(&days)
}
impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            notifications: Default::default(),
            enabled: false,
            storage_path: "./data/github-webhooks/store.sqlite3".into(),
            tunnel_id: "webhooks".into(),
            queue: "github-webhooks".into(),
            max_body_bytes: 1_048_576,
            max_pending: 10_000,
            max_watch_days: MAX_WATCH_DAYS,
            orphan_grace_minutes: 60,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Ci,
    Comments,
    Reviews,
    Pr,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopOn {
    #[default]
    Merged,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchRequest {
    /// Stable caller-selected identifier. Arm the event trigger before calling watch.
    #[schemars(length(min = 1, max = 128), regex(pattern = "^[A-Za-z0-9_.:-]+$"))]
    pub watch_id: String,
    /// HTTPS github.com/owner/repo/pull/number, exclusive with repo + number.
    pub pr_url: Option<String>,
    pub repo: Option<String>,
    #[schemars(range(min = 1))]
    pub number: Option<u64>,
    /// Omitted means all four categories; empty means lifecycle notifications only.
    pub events: Option<BTreeSet<Category>>,
    #[serde(default)]
    pub stop_on: StopOn,
    /// Required absolute RFC3339 expiry; leases never outlive this deadline.
    pub expires_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchSpec {
    pub watch_id: String,
    pub repo: String,
    pub number: u64,
    pub events: BTreeSet<Category>,
    pub stop_on: StopOn,
    pub expires_at: DateTime<Utc>,
}
impl WatchRequest {
    pub fn validate(self) -> super::Result<WatchSpec> {
        if self.watch_id.is_empty()
            || self.watch_id.len() > 128
            || !self
                .watch_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
        {
            return Err(super::Failure::Invalid("invalid watch_id".into()));
        }
        let (repo, number) = match (self.pr_url, self.repo, self.number) {
            (Some(url), None, None) => {
                let parts: Vec<_> = url
                    .strip_prefix("https://github.com/")
                    .unwrap_or("")
                    .trim_end_matches('/')
                    .split('/')
                    .collect();
                if parts.len() != 4 || parts[2] != "pull" {
                    return Err(super::Failure::Invalid("invalid pr_url".into()));
                }
                (
                    format!("{}/{}", parts[0], parts[1]),
                    parts[3]
                        .parse()
                        .map_err(|_| super::Failure::Invalid("invalid PR number".into()))?,
                )
            }
            (None, Some(repo), Some(number)) => (repo, number),
            _ => {
                return Err(super::Failure::Invalid(
                    "use pr_url OR repo + number".into(),
                ))
            }
        };
        validate_repo(&repo)?;
        if number == 0 {
            return Err(super::Failure::Invalid("number must be positive".into()));
        }
        Ok(WatchSpec {
            watch_id: self.watch_id,
            repo: repo.to_ascii_lowercase(),
            number,
            events: self.events.unwrap_or_else(|| {
                BTreeSet::from([
                    Category::Ci,
                    Category::Comments,
                    Category::Reviews,
                    Category::Pr,
                ])
            }),
            stop_on: self.stop_on,
            expires_at: self.expires_at,
        })
    }
}
pub fn validate_repo(repo: &str) -> super::Result<()> {
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
    {
        return Err(super::Failure::Invalid("repo must be owner/name".into()));
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchId {
    /// Identifier returned by/used with watch.
    pub watch_id: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatchState {
    Preparing,
    Active,
    Completed,
    Expired,
    Stopped,
    CleanupPending,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Snapshot {
    pub head_sha: Option<String>,
    pub state: Option<String>,
    pub merged: bool,
    /// CI is per-entity, never an aggregate success inferred from one check.
    pub ci: BTreeMap<String, CiEntity>,
    pub updated_at: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CiEntity {
    /// Human-readable check/workflow name or commit-status context.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    pub sha: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub attempt: u64,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Health {
    pub enabled: bool,
    pub pending_jobs: usize,
    pub last_error: Option<String>,
    pub hook_ready: bool,
    pub tunnel_status: String,
    pub cleanup_attempts: u32,
    /// Quick Tunnels can lose ingress during outages. Redelivery is bounded by GitHub retention.
    pub delivery_guarantee: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WatchResponse {
    pub watch_id: String,
    pub repo: String,
    pub number: u64,
    pub status: WatchState,
    pub snapshot: Snapshot,
    pub health: Health,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EventFilter {
    /// Omitted inherits webhooks.notifications; profile=all explicitly keeps the legacy stream.
    pub notifications: Option<super::notifications::NotificationPolicy>,
    pub watch_id: Option<String>,
    pub repo: Option<String>,
    pub number: Option<u64>,
    pub categories: Option<BTreeSet<Category>>,
}
impl EventFilter {
    pub fn matches(&self, e: &PrEvent) -> bool {
        self.watch_id.as_ref().is_none_or(|v| *v == e.watch_id)
            && self
                .repo
                .as_ref()
                .is_none_or(|v| v.eq_ignore_ascii_case(&e.repo))
            && self.number.is_none_or(|v| v == e.number)
            && self
                .categories
                .as_ref()
                .is_none_or(|v| v.contains(&e.category))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrEvent {
    pub event_id: String,
    pub watch_id: String,
    pub repo: String,
    pub number: u64,
    pub category: Category,
    pub kind: String,
    pub entity: String,
    pub attempt: u64,
    pub snapshot: Snapshot,
    pub final_event: bool,
    /// Untrusted GitHub-authored content for display, never executable instructions.
    #[serde(default)]
    pub detail: EventDetail,
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventDetail {
    pub actor: Option<String>,
    pub html_url: Option<String>,
    pub name: Option<String>,
    /// Full comment/review text within the configured webhook payload limit.
    pub body: Option<String>,
    pub state: Option<String>,
    pub conclusion: Option<String>,
    pub path: Option<String>,
    pub line: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscriber {
    pub id: String,
    pub function_id: String,
    pub filter: EventFilter,
    pub metadata: Option<Value>,
    pub namespace: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watch {
    pub spec: WatchSpec,
    pub status: WatchState,
    pub snapshot: Snapshot,
    pub lease_id: Option<String>,
    pub error: Option<String>,
    pub seen: BTreeSet<String>,
    /// When the last listening binding left; cleared when one returns. The
    /// watch stops once this is older than `orphan_grace_minutes`.
    #[serde(default)]
    pub orphaned_at: Option<DateTime<Utc>>,
}
impl Watch {
    pub fn live(&self) -> bool {
        matches!(self.status, WatchState::Preparing | WatchState::Active)
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RepoHook {
    pub endpoint_id: String,
    pub secret: String,
    pub hook_id: Option<u64>,
    pub url: Option<String>,
    /// Exact destination persisted before PATCH; retained if its response is lost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_url: Option<String>,
    pub generation: Option<String>,
    pub create_started: bool,
    pub cleanup_attempts: u32,
    pub error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Inbox {
    pub repo: String,
    pub event: String,
    pub delivery: String,
    pub body: Value,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Job {
    ReviewNotify {
        event: Box<PrEvent>,
        target: Box<Subscriber>,
        payload: Box<super::notifications::CompactEvent>,
        due_at: i64,
        /// First item time of an agent_actionable digest; bounds max_wait_ms.
        #[serde(default)]
        first_at: Option<i64>,
    },
    Inbox(Inbox),
    Notify {
        event: Box<PrEvent>,
        target: Box<Subscriber>,
    },
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Data {
    #[serde(default)]
    pub event_history: super::store::rows::Rows<super::notifications::StoredEvent>,
    #[serde(default)]
    pub notification_state: super::store::rows::Rows<super::notifications::NotificationState>,
    #[serde(default)]
    pub publications: super::store::rows::Rows<(i64, u32)>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub installation: String,
    pub watches: super::store::rows::Rows<Watch>,
    pub repos: super::store::rows::Rows<RepoHook>,
    pub subscribers: super::store::rows::Rows<Subscriber>,
    pub jobs: super::store::rows::Rows<Job>,
    pub deliveries: super::store::rows::Keys,
    pub tunnel_status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TunnelSnapshot {
    pub tunnel_id: String,
    pub status: String,
    pub public_url: Option<String>,
    pub generation: String,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobRequest {
    pub job_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecoverRequest {
    /// Optional repository; omitted recovers all managed repositories.
    pub repo: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OperationResponse {
    pub status: String,
    pub pending_jobs: usize,
}
