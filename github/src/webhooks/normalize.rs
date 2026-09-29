mod baseline;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{types::*, Failure, Result};

/// A point-in-time REST read is authoritative; never replace current HEAD with
/// the head embedded in a delayed webhook. Called only at setup/recovery/events.
pub fn snapshot(pr: &Value, previous: &Snapshot) -> Result<Snapshot> {
    let head = pr
        .pointer("/head/sha")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::Invalid("GitHub PR response missing head.sha".into()))?;
    let state = pr
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::Invalid("GitHub PR response missing state".into()))?;
    let updated = pr.get("updated_at").and_then(Value::as_str);
    if previous.merged
        || updated
            .zip(previous.updated_at.as_deref())
            .is_some_and(|(new, old)| new < old)
    {
        return Ok(previous.clone());
    }
    let mut next = previous.clone();
    if next.head_sha.as_deref() != Some(head) {
        next.ci.clear();
    }
    next.head_sha = Some(head.into());
    next.state = Some(state.into());
    next.merged = pr.get("merged").and_then(Value::as_bool).unwrap_or(false);
    next.updated_at = pr
        .get("updated_at")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(next)
}

pub fn category(event: &str) -> Option<Category> {
    match event {
        "pull_request" => Some(Category::Pr),
        "issue_comment" | "pull_request_review_comment" => Some(Category::Comments),
        "pull_request_review" => Some(Category::Reviews),
        "check_run" | "status" | "workflow_run" => Some(Category::Ci),
        _ => None,
    }
}
pub fn event_sha<'a>(event: &str, body: &'a Value) -> Option<&'a str> {
    let pointer = match event {
        "check_run" => "/check_run/head_sha",
        "workflow_run" => "/workflow_run/head_sha",
        "status" => "/sha",
        _ => "/pull_request/head/sha",
    };
    body.pointer(pointer).and_then(Value::as_str)
}
pub fn number(body: &Value) -> Option<u64> {
    body.pointer("/pull_request/number")
        .or_else(|| body.get("number"))
        .or_else(|| body.pointer("/issue/number"))
        .and_then(Value::as_u64)
}
pub fn relevant(watch: &Watch, inbox: &Inbox) -> bool {
    if !watch.live() || watch.spec.repo != inbox.repo {
        return false;
    }
    match inbox.event.as_str() {
        "issue_comment" => {
            inbox.body.pointer("/issue/pull_request").is_some()
                && number(&inbox.body) == Some(watch.spec.number)
        }
        "pull_request" | "pull_request_review" | "pull_request_review_comment" => {
            number(&inbox.body) == Some(watch.spec.number)
        }
        "check_run" | "status" | "workflow_run" => true, // Includes forks and empty pull_requests: correlate by authoritative SHA after GET.
        _ => false,
    }
}

pub fn finish_if_needed(w: &mut Watch) -> bool {
    let done = w.snapshot.merged
        || (w.spec.stop_on == StopOn::Closed && w.snapshot.state.as_deref() == Some("closed"));
    if done {
        w.status = WatchState::Completed;
    }
    done
}

pub fn normalize(w: &mut Watch, inbox: &Inbox, current: Snapshot) -> Option<PrEvent> {
    let cat = category(&inbox.event)?;
    w.snapshot = current;
    w.status = WatchState::Active;
    // A merge discovered while reconciling any event must become a final event,
    // even when that event's own SHA is stale or its category was not selected.
    if finish_if_needed(w) {
        return Some(make_event(
            w,
            Category::Pr,
            if w.snapshot.merged {
                "merged"
            } else {
                "closed"
            },
            "lifecycle",
            0,
            true,
        ));
    }
    if !w.spec.events.contains(&cat) {
        return None;
    }
    let body = &inbox.body;
    if inbox.event == "pull_request" {
        if event_sha(&inbox.event, body)
            .is_some_and(|sha| w.snapshot.head_sha.as_deref() != Some(sha))
        {
            return None;
        }
        if body["action"] == "closed" && w.snapshot.state.as_deref() != Some("closed") {
            return None;
        }
    }
    let mut entity = number(body).unwrap_or(w.spec.number).to_string();
    let mut attempt = 0;
    if cat == Category::Ci {
        let sha = event_sha(&inbox.event, body)?;
        if w.snapshot.head_sha.as_deref() != Some(sha) {
            return None;
        }
        let item = match inbox.event.as_str() {
            "check_run" => &body["check_run"],
            "workflow_run" => &body["workflow_run"],
            _ => body,
        };
        entity = if inbox.event == "status" {
            item["context"].as_str()?.into()
        } else {
            item["id"].as_u64()?.to_string()
        };
        attempt = item["run_attempt"].as_u64().unwrap_or(1);
        let status = item["status"]
            .as_str()
            .or_else(|| item["state"].as_str())
            .unwrap_or("unknown")
            .to_owned();
        let updated_at = item["updated_at"]
            .as_str()
            .or_else(|| item["completed_at"].as_str())
            .or_else(|| item["started_at"].as_str())
            .or_else(|| item["created_at"].as_str())
            .unwrap_or("")
            .to_owned();
        let key = format!("{}:{entity}", inbox.event);
        let ci = CiEntity {
            name: text(item, "name").or_else(|| text(item, "context")),
            html_url: text(item, "html_url")
                .or_else(|| text(item, "details_url"))
                .or_else(|| text(item, "target_url")),
            sha: sha.into(),
            status,
            conclusion: item["conclusion"].as_str().map(str::to_owned),
            attempt,
            updated_at,
        };
        if let Some(old) = w.snapshot.ci.get(&key) {
            if old.attempt > ci.attempt
                || (old.attempt == ci.attempt
                    && (old.updated_at > ci.updated_at
                        || (old.status == "completed" && ci.status != "completed")))
                || old == &ci
            {
                return None;
            }
        }
        w.snapshot.ci.insert(key, ci);
    } else if cat == Category::Comments || cat == Category::Reviews {
        let item = if cat == Category::Reviews {
            &body["review"]
        } else {
            &body["comment"]
        };
        entity = item["id"].as_u64()?.to_string();
    }
    let action = body["action"].as_str().unwrap_or("updated");
    let kind = if inbox.event == "pull_request" && action == "closed" {
        "closed"
    } else {
        action
    };
    // Entity content/state forms the effect key, not only the delivery UUID.
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&(inbox.event.as_str(), &entity, attempt, body)).ok()?,
    ));
    if !w.seen.insert(fingerprint) {
        return None;
    }
    let mut event = make_event(
        w,
        cat,
        &format!("{}:{kind}", inbox.event),
        &entity,
        attempt,
        false,
    );
    event.detail = event_detail(inbox);
    Some(event)
}
pub fn make_event(
    w: &Watch,
    category: Category,
    kind: &str,
    entity: &str,
    attempt: u64,
    final_event: bool,
) -> PrEvent {
    // Distinct snapshots/attempts must not share a consumer deduplication ID.
    // Replaying exactly the same semantic state keeps a stable ID.
    let key = serde_json::to_vec(&(
        &w.spec.watch_id,
        &w.spec.repo,
        w.spec.number,
        &w.snapshot,
        category,
        kind,
        entity,
        attempt,
        final_event,
    ))
    .expect("typed PR event identity serializes");
    PrEvent {
        event_id: hex::encode(Sha256::digest(&key)),
        watch_id: w.spec.watch_id.clone(),
        repo: w.spec.repo.clone(),
        number: w.spec.number,
        category,
        kind: kind.into(),
        entity: entity.into(),
        attempt,
        snapshot: w.snapshot.clone(),
        final_event,
        detail: EventDetail::default(),
    }
}
fn text(item: &Value, field: &str) -> Option<String> {
    item.get(field).and_then(Value::as_str).map(str::to_owned)
}
fn event_detail(inbox: &Inbox) -> EventDetail {
    let body = &inbox.body;
    let item = match inbox.event.as_str() {
        "issue_comment" | "pull_request_review_comment" => &body["comment"],
        "pull_request_review" => &body["review"],
        "pull_request" => &body["pull_request"],
        "check_run" => &body["check_run"],
        "workflow_run" => &body["workflow_run"],
        _ => body,
    };
    EventDetail {
        actor: item
            .pointer("/user/login")
            .or_else(|| body.pointer("/sender/login"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        html_url: text(item, "html_url")
            .or_else(|| text(item, "details_url"))
            .or_else(|| text(item, "target_url")),
        name: text(item, "name")
            .or_else(|| text(item, "context"))
            .or_else(|| text(item, "title")),
        body: text(item, "body"),
        state: text(item, "state").or_else(|| text(item, "status")),
        conclusion: text(item, "conclusion"),
        path: text(item, "path"),
        line: item
            .get("line")
            .or_else(|| item.get("original_line"))
            .and_then(Value::as_u64),
    }
}
pub fn persist_event(data: &mut Data, event: PrEvent, source: &str) {
    persist_event_with_policy(
        data,
        event,
        source,
        &super::notifications::NotificationPolicy::default(),
    );
}

pub fn persist_event_with_policy(
    data: &mut Data,
    mut event: PrEvent,
    source: &str,
    policy: &super::notifications::NotificationPolicy,
) {
    event.event_id = hex::encode(Sha256::digest(format!("{}:{source}", event.event_id)));
    let now = chrono::Utc::now().timestamp_millis();
    data.event_history
        .entry(event.event_id.clone())
        .or_insert_with(|| super::notifications::StoredEvent {
            event: Box::new(event.clone()),
            created_at: now,
        });
    let subscribers: Vec<_> = data.subscribers.values().cloned().collect();
    for sub in subscribers {
        super::notifications::route(
            data,
            &event,
            &sub,
            &super::notifications::effective(&sub, policy),
            now,
        );
    }
}
