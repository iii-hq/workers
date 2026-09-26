//! Subscriber-side notification policy. Ingress and authoritative state stay lossless.
use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::types::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationProfile {
    /// Backward-compatible full per-entity event stream.
    #[default]
    All,
    /// Compact review actions, batched CI failures and explicitly scoped success.
    ReviewAssistant,
    /// Only what needs an agent action: CI failures on the current head, new
    /// comments/reviews from others and terminal lifecycle, coalesced into one
    /// digest per watch. CI success, progress and pushes are never delivered.
    AgentActionable,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationPolicy {
    pub profile: NotificationProfile,
    /// Exact selectors: check_run:<name>, workflow_run:<name>, status:<context>.
    /// Every selector must exist and pass. Empty disables aggregate success.
    pub success_checks: BTreeSet<String>,
    /// Suppress comments/reviews by these logins (case-insensitive).
    /// Explicit identity avoids dropping another human using the same account unknowingly.
    pub ignored_actors: BTreeSet<String>,
    /// Only known informational bot templates are suppressed; unknown reviews survive.
    pub suppress_bot_noise: bool,
    /// Maximum comment characters in the compact payload; original stays in event-detail.
    pub max_comment_chars: usize,
    /// Durable local coalescing window. Maintenance releases due batches every 10s.
    pub batch_window_ms: u64,
    pub notify_ci_failures: bool,
    pub notify_ci_success: bool,
    pub notify_resolved_threads: bool,
    /// Drop comments/reviews authored by the authenticated gh identity, discovered
    /// automatically (GET /user) and kept in memory only.
    pub ignore_self: bool,
    /// agent_actionable: deliver after this much silence; each new item restarts it.
    pub quiet_ms: u64,
    /// agent_actionable: never hold a digest longer than this after its first item.
    pub max_wait_ms: u64,
    /// agent_actionable: items per digest; extra items are counted, not sent.
    pub max_items: usize,
    /// Runtime-only login used by ignore_self. Never configured or persisted.
    #[serde(skip)]
    #[schemars(skip)]
    pub resolved_self: Option<String>,
}
impl Default for NotificationPolicy {
    fn default() -> Self {
        Self {
            profile: NotificationProfile::All,
            success_checks: BTreeSet::new(),
            ignored_actors: BTreeSet::new(),
            suppress_bot_noise: true,
            max_comment_chars: 2000,
            batch_window_ms: 10_000,
            notify_ci_failures: true,
            notify_ci_success: true,
            notify_resolved_threads: false,
            ignore_self: false,
            quiet_ms: 15_000,
            max_wait_ms: 120_000,
            max_items: 30,
            resolved_self: None,
        }
    }
}
impl NotificationPolicy {
    pub fn validate(&self) -> super::Result<()> {
        if !(1..=8000).contains(&self.max_comment_chars)
            || self.batch_window_ms > 60_000
            || self.quiet_ms > self.max_wait_ms
            || self.max_wait_ms > 900_000
            || !(1..=100).contains(&self.max_items)
            || self.success_checks.len() > 100
            || self.ignored_actors.len() > 100
            || self
                .ignored_actors
                .iter()
                .any(|v| v.is_empty() || v.len() > 100)
            || self.success_checks.iter().any(|v| {
                !v.split_once(':').is_some_and(|(kind, name)| {
                    matches!(kind, "check_run" | "workflow_run" | "status")
                        && !name.trim().is_empty()
                        && name.len() <= 256
                })
            })
        {
            return Err(super::Failure::Invalid(
                "invalid notification limits or CI selectors".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CiFailure {
    pub entity: String,
    pub name: String,
    pub conclusion: String,
    pub attempt: u64,
    pub html_url: Option<String>,
    /// Retrieve the original normalized event with github::pr::event-detail.
    pub event_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompactEvent {
    pub event_id: String,
    pub watch_id: String,
    pub repo: String,
    pub number: u64,
    pub head_sha: Option<String>,
    pub category: Category,
    pub kind: String,
    pub final_event: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<EventDetail>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<CiFailure>,
    /// Additional failures omitted from this bounded payload; consult watch-status.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_failures: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub selected_checks: usize,
    pub body_truncated: bool,
    /// agent_actionable digest (kind = "digest"): ordered actionable items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DigestItem>,
    /// Items dropped because the digest reached max_items; consult event-detail/watch-status.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_items: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DigestItem {
    /// Retrieve the original normalized event with github::pr::event-detail.
    pub event_id: String,
    pub category: Category,
    /// ci.failed, or the original kind (issue_comment:created, pull_request_review:submitted, merged...).
    pub kind: String,
    pub head_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<EventDetail>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure: Option<CiFailure>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub body_truncated: bool,
}
fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The trigger keeps the legacy schema as one branch and adds compact delivery.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NotificationEvent {
    Full(PrEvent),
    Compact(CompactEvent),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct StoredEvent {
    pub event: Box<PrEvent>,
    pub created_at: i64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct NotificationState {
    pub created_at: i64,
    /// Semantic identities committed with outbox insertion, surviving restarts.
    pub seen: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventDetailRequest {
    /// Full normalized event id received in a notification, retained for seven days (bounded).
    pub event_id: String,
}

/// The subscriber override (or the default) plus runtime-only discovered state.
pub fn effective(sub: &Subscriber, default: &NotificationPolicy) -> NotificationPolicy {
    let mut policy = sub
        .filter
        .notifications
        .clone()
        .unwrap_or_else(|| default.clone());
    policy.resolved_self = default.resolved_self.clone();
    policy
}
/// ignore_self needs the authenticated login only when some policy asks for it.
pub fn wants_self(default: &NotificationPolicy, data: &Data) -> bool {
    default.ignore_self
        || data.subscribers.values().any(|s| {
            s.filter
                .notifications
                .as_ref()
                .is_some_and(|p| p.ignore_self)
        })
}
fn hash(value: impl Serialize) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(&value).expect("notification identity serializes"),
    ))
}
fn outcome(ci: &CiEntity) -> &str {
    if ci.status == "completed" {
        ci.conclusion.as_deref().unwrap_or("unknown")
    } else {
        &ci.status
    }
}
fn selector(key: &str, ci: &CiEntity) -> String {
    let (kind, entity) = key.split_once(':').unwrap_or((key, key));
    format!(
        "{kind}:{}",
        if kind == "status" {
            entity
        } else {
            ci.name.as_deref().unwrap_or(entity)
        }
    )
}
/// Explicit membership prevents a single early green check from completing discovery.
pub fn selected_passed(snapshot: &Snapshot, policy: &NotificationPolicy) -> bool {
    if policy.success_checks.is_empty() {
        return false;
    }
    let mut latest = BTreeMap::<String, (&str, &CiEntity)>::new();
    for (key, ci) in &snapshot.ci {
        if snapshot.head_sha.as_deref() != Some(ci.sha.as_str()) {
            continue;
        }
        let name = selector(key, ci);
        if !policy.success_checks.contains(&name) {
            continue;
        }
        let replace = latest.get(&name).is_none_or(|(old_key, old)| {
            (&ci.updated_at, ci.attempt, key.as_str()) > (&old.updated_at, old.attempt, *old_key)
        });
        if replace {
            latest.insert(name, (key, ci));
        }
    }
    latest.len() == policy.success_checks.len()
        && latest.values().all(|(_, ci)| outcome(ci) == "success")
}
fn truncate(value: &mut Option<String>, limit: usize) -> bool {
    let Some(text) = value else {
        return false;
    };
    let Some((end, _)) = text.char_indices().nth(limit) else {
        return false;
    };
    text.truncate(end);
    true
}
fn compact(event: &PrEvent, policy: &NotificationPolicy) -> CompactEvent {
    let mut detail = event.detail.clone();
    let body_truncated = truncate(&mut detail.body, policy.max_comment_chars);
    truncate(&mut detail.name, 200);
    truncate(&mut detail.actor, 100);
    truncate(&mut detail.html_url, 1024);
    truncate(&mut detail.path, 512);
    if event.category == Category::Pr || event.category == Category::Ci {
        detail.body = None;
    }
    CompactEvent {
        event_id: event.event_id.clone(),
        watch_id: event.watch_id.clone(),
        repo: event.repo.clone(),
        number: event.number,
        head_sha: event.snapshot.head_sha.clone(),
        category: event.category,
        kind: event.kind.clone(),
        final_event: event.final_event,
        detail: Some(detail),
        failures: vec![],
        omitted_failures: 0,
        selected_checks: 0,
        body_truncated,
        items: vec![],
        omitted_items: 0,
    }
}
fn bot_noise(event: &PrEvent) -> bool {
    let actor = event
        .detail
        .actor
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if !actor.ends_with("[bot]") {
        return false;
    }
    let body = event.detail.body.as_deref().unwrap_or("");
    // Never discard an inline finding, changes-requested review, or an unknown bot.
    if event.detail.path.is_some() || event.detail.state.as_deref() == Some("changes_requested") {
        return false;
    }
    match actor.as_str() {
        "vercel[bot]" => {
            body.starts_with("[vc]:") && body.contains("The latest updates on your projects.")
        }
        "coderabbitai[bot]" => {
            body.contains("<!-- This is an auto-generated comment: summarize by coderabbit.ai -->")
                || (body.contains("<!-- CodeRabbit review command invocation:")
                    && (body.contains("Review triggered.") || body.contains("Review finished.")))
                || (body.contains("<!-- This is an auto-generated reply by CodeRabbit -->")
                    && body.contains("✅ Review thread resolved."))
        }
        "github-actions[bot]" => {
            body.starts_with("<!-- skill-check-status-comment:") && body.contains("Four for four.")
        }
        _ => false,
    }
}
fn review_notice(event: &PrEvent, policy: &NotificationPolicy) -> bool {
    if event.final_event {
        return true;
    }
    if matches!(event.category, Category::Comments | Category::Reviews) {
        if event.detail.actor.as_ref().is_some_and(|actor| {
            policy
                .ignored_actors
                .iter()
                .any(|v| v.eq_ignore_ascii_case(actor))
        }) {
            return false;
        }
        if policy.suppress_bot_noise && bot_noise(event) {
            return false;
        }
        if event.kind.ends_with(":deleted") {
            return false;
        }
        if event.kind == "pull_request_review_thread:resolved" {
            return policy.notify_resolved_threads;
        }
        return event
            .detail
            .body
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            || matches!(
                event.detail.state.as_deref(),
                Some("changes_requested" | "approved" | "dismissed")
            )
            || event.kind == "pull_request_review_thread:unresolved";
    }
    matches!(
        event.kind.as_str(),
        "pull_request:opened"
            | "pull_request:reopened"
            | "pull_request:ready_for_review"
            | "pull_request:closed"
            | "merged"
            | "closed"
            | "expired"
    )
}

/// Called inside the same transaction as normalization and inbox acknowledgement.
pub fn route(
    data: &mut Data,
    event: &PrEvent,
    sub: &Subscriber,
    policy: &NotificationPolicy,
    now: i64,
) {
    if !sub.filter.matches(event) {
        return;
    }
    if policy.profile == NotificationProfile::All {
        let id = format!("notify:{}:{}", event.event_id, sub.id);
        data.jobs.entry(id).or_insert_with(|| Job::Notify {
            event: Box::new(event.clone()),
            target: Box::new(sub.clone()),
        });
        return;
    }
    if policy.profile == NotificationProfile::AgentActionable {
        route_actionable(data, event, sub, policy, now);
        return;
    }
    let mut payload = compact(event, policy);
    let semantic;
    if event.category == Category::Ci {
        let Some(ci) = event.snapshot.ci.get(&format!(
            "{}:{}",
            event.kind.split(':').next().unwrap_or(""),
            event.entity
        )) else {
            return;
        };
        let result = outcome(ci);
        if matches!(
            result,
            "queued" | "in_progress" | "pending" | "waiting" | "requested" | "unknown"
        ) {
            return;
        }
        let selected = policy.success_checks.contains(&selector(
            &format!(
                "{}:{}",
                event.kind.split(':').next().unwrap_or(""),
                event.entity
            ),
            ci,
        ));
        if policy.notify_ci_failures
            && (matches!(
                result,
                "failure" | "error" | "timed_out" | "action_required" | "startup_failure"
            ) || (selected && matches!(result, "cancelled" | "skipped" | "neutral" | "stale")))
        {
            payload.kind = "ci.failed".into();
            payload.detail = None;
            payload.failures.push(CiFailure {
                entity: event.entity.chars().take(256).collect(),
                name: ci
                    .name
                    .as_deref()
                    .unwrap_or(&event.entity)
                    .chars()
                    .take(200)
                    .collect(),
                conclusion: result.into(),
                attempt: ci.attempt,
                html_url: ci.html_url.as_ref().map(|s| s.chars().take(1024).collect()),
                event_id: event.event_id.clone(),
            });
            semantic = hash((
                &event.kind.split(':').next(),
                &event.entity,
                ci.attempt,
                &ci.updated_at,
                result,
            ));
        } else if policy.notify_ci_success && selected_passed(&event.snapshot, policy) {
            payload.kind = "ci.passed".into();
            payload.detail = None;
            payload.selected_checks = policy.success_checks.len();
            semantic = hash(("ci.passed", &policy.success_checks));
        } else {
            return;
        }
    } else {
        if !review_notice(event, policy) {
            return;
        }
        semantic = hash((
            &event.category,
            event.kind.split(':').next(),
            &event.entity,
            &event.detail,
        ));
    }
    let state_key = hash((
        &sub.id,
        &event.watch_id,
        (event.category == Category::Ci).then_some(&event.snapshot.head_sha),
    ));
    let state = data
        .notification_state
        .entry(state_key.clone())
        .or_insert_with(|| NotificationState {
            created_at: now,
            ..Default::default()
        });
    if !state.seen.insert(semantic.clone()) {
        return;
    }
    state.created_at = now;
    // Bounded per consumer/head; the seven-day SQL retention also bounds inactive heads.
    if state.seen.len() > 4096 {
        state.seen.pop_first();
    }
    let batch = payload.kind == "ci.failed";
    let id = format!(
        "review:{state_key}:{}",
        if batch { "failures" } else { &semantic }
    );
    if let Some(Job::ReviewNotify {
        payload: existing, ..
    }) = data.jobs.get_mut(&id)
    {
        if existing.failures.len() < 20 {
            existing.failures.extend(payload.failures);
        } else {
            existing.omitted_failures += 1;
        }
        return;
    }
    let due_at = now.saturating_add(if event.category == Category::Ci {
        policy.batch_window_ms as i64
    } else {
        0
    });
    data.jobs.insert(
        id,
        Job::ReviewNotify {
            event: Box::new(event.clone()),
            target: Box::new(sub.clone()),
            payload: Box::new(payload),
            due_at,
            first_at: None,
        },
    );
}

/// A suppressed queued success may be reconsidered by a later terminal event.
pub fn release_success(
    data: &mut Data,
    sub: &Subscriber,
    event: &PrEvent,
    policy: &NotificationPolicy,
) {
    let key = hash((&sub.id, &event.watch_id, Some(&event.snapshot.head_sha)));
    if let Some(state) = data.notification_state.get_mut(&key) {
        state
            .seen
            .remove(&hash(("ci.passed", &policy.success_checks)));
    }
}

/// Payload kind of an `agent_actionable` digest.
pub const DIGEST_KIND: &str = "digest";
/// Only these CI outcomes need an agent action. Success, progress, skipped and
/// cancelled (usually superseded by a newer push) are never delivered.
const ACTIONABLE_CI: &[&str] = &["failure", "timed_out", "startup_failure", "action_required"];

fn is_self(event: &PrEvent, policy: &NotificationPolicy) -> bool {
    policy.ignore_self
        && policy
            .resolved_self
            .as_deref()
            .zip(event.detail.actor.as_deref())
            .is_some_and(|(me, actor)| me.eq_ignore_ascii_case(actor))
}
fn push_item(payload: &mut CompactEvent, item: DigestItem, max_items: usize, force: bool) {
    if force || payload.items.len() < max_items {
        payload.items.push(item);
    } else {
        payload.omitted_items += 1;
    }
}

/// Actionable-only routing. Dropped events stay retrievable via event-detail.
fn route_actionable(
    data: &mut Data,
    event: &PrEvent,
    sub: &Subscriber,
    policy: &NotificationPolicy,
    now: i64,
) {
    let mut item = DigestItem {
        event_id: event.event_id.clone(),
        category: event.category,
        kind: event.kind.clone(),
        head_sha: event.snapshot.head_sha.clone(),
        detail: None,
        failure: None,
        body_truncated: false,
    };
    let mut supersedes_workflow = false;
    let semantic = if event.final_event {
        hash(("final", &event.kind))
    } else if event.category == Category::Ci {
        let source = event.kind.split(':').next().unwrap_or("");
        let Some(ci) = event.snapshot.ci.get(&format!("{source}:{}", event.entity)) else {
            return;
        };
        let result = outcome(ci);
        if !ACTIONABLE_CI.contains(&result) {
            return;
        }
        // A failed workflow repeats the failure of its job; keep the specific job.
        if source == "workflow_run"
            && event.snapshot.ci.iter().any(|(key, c)| {
                key.starts_with("check_run:")
                    && c.sha == ci.sha
                    && ACTIONABLE_CI.contains(&outcome(c))
            })
        {
            return;
        }
        supersedes_workflow = source == "check_run";
        item.kind = "ci.failed".into();
        item.failure = Some(CiFailure {
            entity: event.entity.chars().take(256).collect(),
            name: ci
                .name
                .as_deref()
                .unwrap_or(&event.entity)
                .chars()
                .take(200)
                .collect(),
            conclusion: result.into(),
            attempt: ci.attempt,
            html_url: ci.html_url.as_ref().map(|s| s.chars().take(1024).collect()),
            event_id: event.event_id.clone(),
        });
        hash((source, &event.entity, ci.attempt, &ci.updated_at, result))
    } else if matches!(event.category, Category::Comments | Category::Reviews) {
        if event.kind.ends_with(":edited")
            || is_self(event, policy)
            || !review_notice(event, policy)
        {
            return;
        }
        let compacted = compact(event, policy);
        item.detail = compacted.detail;
        item.body_truncated = compacted.body_truncated;
        hash((
            &event.category,
            event.kind.split(':').next(),
            &event.entity,
            &event.detail,
        ))
    } else if event.kind == "pull_request:closed" {
        hash((&event.kind, &event.snapshot.updated_at))
    } else {
        // Pushes, edits, labels, assignments and snapshots need no action.
        return;
    };
    let state_key = hash((&sub.id, &event.watch_id, DIGEST_KIND));
    let state = data
        .notification_state
        .entry(state_key.clone())
        .or_insert_with(|| NotificationState {
            created_at: now,
            ..Default::default()
        });
    if !state.seen.insert(semantic) {
        return;
    }
    state.created_at = now;
    if state.seen.len() > 4096 {
        state.seen.pop_first();
    }
    let quiet_due = now.saturating_add(policy.quiet_ms as i64);
    let id = format!("digest:{state_key}");
    if let Some(Job::ReviewNotify {
        payload,
        due_at,
        first_at,
        ..
    }) = data.jobs.get_mut(&id)
    {
        if supersedes_workflow {
            let ci = &event.snapshot.ci;
            payload.items.retain(|i| {
                !i.failure
                    .as_ref()
                    .is_some_and(|f| ci.contains_key(&format!("workflow_run:{}", f.entity)))
            });
        }
        push_item(payload, item, policy.max_items, event.final_event);
        payload.final_event |= event.final_event;
        payload.head_sha = event.snapshot.head_sha.clone();
        let deadline = first_at
            .unwrap_or(now)
            .saturating_add(policy.max_wait_ms as i64);
        *due_at = if payload.final_event {
            now
        } else {
            quiet_due.min(deadline)
        };
        return;
    }
    let mut payload = CompactEvent {
        event_id: event.event_id.clone(),
        watch_id: event.watch_id.clone(),
        repo: event.repo.clone(),
        number: event.number,
        head_sha: event.snapshot.head_sha.clone(),
        category: event.category,
        kind: DIGEST_KIND.into(),
        final_event: event.final_event,
        detail: None,
        failures: vec![],
        omitted_failures: 0,
        selected_checks: 0,
        body_truncated: false,
        items: vec![],
        omitted_items: 0,
    };
    push_item(&mut payload, item, policy.max_items, true);
    data.jobs.insert(
        id,
        Job::ReviewNotify {
            event: Box::new(event.clone()),
            target: Box::new(sub.clone()),
            payload: Box::new(payload),
            due_at: if event.final_event { now } else { quiet_due },
            first_at: Some(now),
        },
    );
}

/// At delivery: CI failures of a superseded head are dropped, while comments and
/// reviews stay relevant after a push. Returns false when nothing is left.
pub fn prune_digest(payload: &mut CompactEvent, head: Option<&str>) -> bool {
    payload
        .items
        .retain(|i| i.failure.is_none() || i.head_sha.as_deref() == head);
    if head.is_some() {
        payload.head_sha = head.map(str::to_owned);
    }
    !payload.items.is_empty()
}
