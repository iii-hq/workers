//! Notification policies run before a consumer invocation, not inside an agent turn.
use github::webhooks::{normalize, notifications::*, store::Store, *};
use serde_json::json;

fn policy() -> NotificationPolicy {
    NotificationPolicy {
        profile: NotificationProfile::ReviewAssistant,
        ..Default::default()
    }
}
fn subscriber() -> Subscriber {
    Subscriber {
        id: "consumer".into(),
        function_id: "test::notify".into(),
        filter: EventFilter::default(),
        metadata: Some(json!({"tenant":7})),
        namespace: Some("consumer-ns".into()),
    }
}
fn event(kind: &str, category: Category) -> PrEvent {
    PrEvent {
        event_id: "original-event".into(),
        watch_id: "watch".into(),
        repo: "owner/repo".into(),
        number: 1,
        category,
        kind: kind.into(),
        entity: "42".into(),
        attempt: 1,
        snapshot: Snapshot {
            head_sha: Some("head".into()),
            ..Default::default()
        },
        final_event: false,
        detail: EventDetail::default(),
    }
}
fn ci(kind: &str, state: &str, conclusion: Option<&str>, name: &str) -> PrEvent {
    let mut event = event(&format!("{kind}:completed"), Category::Ci);
    event.detail.name = Some(name.into());
    event.snapshot.ci.insert(
        format!("{kind}:42"),
        CiEntity {
            name: Some(name.into()),
            html_url: Some("https://ci.example/job/42".into()),
            sha: "head".into(),
            status: state.into(),
            conclusion: conclusion.map(str::to_owned),
            attempt: 1,
            updated_at: "2026-09-24T12:00:00Z".into(),
        },
    );
    event
}
fn payload(data: &Data) -> &CompactEvent {
    let Job::ReviewNotify { payload, .. } = data.jobs.values().next().unwrap() else {
        panic!("expected compact event")
    };
    payload
}

#[test]
fn progress_snapshots_edits_and_individual_success_never_wake_review_consumer() {
    for event in [
        ci("check_run", "queued", None, "lint"),
        ci("check_run", "in_progress", None, "lint"),
        ci("status", "pending", None, "preview"),
        ci("workflow_run", "completed", Some("success"), "CI"),
        event("snapshot", Category::Pr),
        event("pull_request:edited", Category::Pr),
        event("pull_request:synchronize", Category::Pr),
    ] {
        let mut data = Data::default();
        route(&mut data, &event, &subscriber(), &policy(), 1000);
        assert!(
            data.jobs.is_empty(),
            "unexpected notification for {}",
            event.kind
        );
    }
}

#[test]
fn failures_are_batched_bounded_and_keep_details_and_reruns() {
    let mut data = Data::default();
    let mut event = ci("check_run", "completed", Some("failure"), "Rust tests");
    let sub = subscriber();
    route(&mut data, &event, &sub, &policy(), 1000);
    // Same semantic event with a different delivery id must not duplicate.
    event.event_id = "redelivered".into();
    route(&mut data, &event, &sub, &policy(), 1001);
    assert_eq!(payload(&data).failures.len(), 1);
    event.snapshot.ci.get_mut("check_run:42").unwrap().attempt = 2;
    route(&mut data, &event, &sub, &policy(), 1002);
    assert_eq!(payload(&data).failures.len(), 2);
    assert_eq!(payload(&data).failures[1].attempt, 2);
    let Job::ReviewNotify { due_at, target, .. } = data.jobs.values().next().unwrap() else {
        unreachable!()
    };
    assert_eq!(
        *due_at, 11_000,
        "batch deadline must not slide on every failure"
    );
    assert_eq!(target.namespace.as_deref(), Some("consumer-ns"));
    let value = serde_json::to_value(payload(&data)).unwrap();
    assert!(value.get("snapshot").is_none());
    assert_eq!(value["kind"], "ci.failed");
    assert_eq!(value["failures"][0]["name"], "Rust tests");
    assert_eq!(
        value["failures"][0]["html_url"],
        "https://ci.example/job/42"
    );
    for attempt in 3..=25 {
        event.snapshot.ci.get_mut("check_run:42").unwrap().attempt = attempt;
        route(&mut data, &event, &sub, &policy(), 1003);
    }
    assert_eq!(payload(&data).failures.len(), 20);
    assert_eq!(payload(&data).omitted_failures, 5);
}

#[test]
fn all_selected_success_requires_complete_membership_and_no_pending_checks() {
    let mut p = policy();
    p.success_checks = ["check_run:lint".into(), "check_run:test".into()].into();
    let mut event = ci("check_run", "completed", Some("success"), "lint");
    let mut data = Data::default();
    route(&mut data, &event, &subscriber(), &p, 0);
    assert!(data.jobs.is_empty(), "missing test cannot mean success");
    let mut test = event.snapshot.ci["check_run:42"].clone();
    test.name = Some("test".into());
    test.status = "queued".into();
    test.conclusion = None;
    event.snapshot.ci.insert("check_run:43".into(), test);
    route(&mut data, &event, &subscriber(), &p, 1);
    assert!(data.jobs.is_empty(), "queued test cannot mean success");
    let test = event.snapshot.ci.get_mut("check_run:43").unwrap();
    test.status = "completed".into();
    test.conclusion = Some("success".into());
    route(&mut data, &event, &subscriber(), &p, 2);
    assert_eq!(payload(&data).kind, "ci.passed");
    assert_eq!(payload(&data).selected_checks, 2);
    data.jobs.clear();
    route(&mut data, &event, &subscriber(), &p, 3);
    assert!(
        data.jobs.is_empty(),
        "success is once per subscription/head/selection"
    );
    release_success(&mut data, &subscriber(), &event, &p);
    route(&mut data, &event, &subscriber(), &p, 4);
    assert_eq!(
        data.jobs.len(),
        1,
        "failed final confirmation must permit reconsideration"
    );
}

#[test]
fn newest_check_attempt_wins_over_old_failed_run_and_future_pending_run_blocks() {
    let mut p = policy();
    p.success_checks.insert("check_run:tests".into());
    let mut event = ci("check_run", "completed", Some("failure"), "tests");
    let mut latest = event.snapshot.ci["check_run:42"].clone();
    latest.conclusion = Some("success".into());
    latest.updated_at = "2026-09-24T12:01:00Z".into();
    event
        .snapshot
        .ci
        .insert("check_run:43".into(), latest.clone());
    assert!(selected_passed(&event.snapshot, &p));
    latest.status = "in_progress".into();
    latest.conclusion = None;
    latest.updated_at = "2026-09-24T12:02:00Z".into();
    event.snapshot.ci.insert("check_run:44".into(), latest);
    assert!(!selected_passed(&event.snapshot, &p));
}

#[test]
fn unrelated_skipped_cancelled_and_neutral_do_not_fail_but_selected_ones_block() {
    for result in ["skipped", "cancelled", "neutral"] {
        let event = ci("check_run", "completed", Some(result), "lint");
        let mut data = Data::default();
        let mut p = policy();
        route(&mut data, &event, &subscriber(), &p, 0);
        assert!(data.jobs.is_empty());
        p.success_checks.insert("check_run:lint".into());
        route(&mut data, &event, &subscriber(), &p, 0);
        assert_eq!(payload(&data).failures[0].conclusion, result);
        assert!(!selected_passed(&event.snapshot, &p));
    }
}

#[test]
fn unicode_comments_are_compact_but_full_event_is_retrievable() {
    let mut event = event("pull_request_review_comment:created", Category::Comments);
    event.detail = EventDetail {
        body: Some("ação 🌍 comentário muito longo".into()),
        actor: Some("reviewer".into()),
        path: Some("src/main.rs".into()),
        line: Some(8),
        ..Default::default()
    };
    let mut p = policy();
    p.max_comment_chars = 6;
    let mut data = Data::default();
    data.subscribers.insert("consumer".into(), subscriber());
    normalize::persist_event_with_policy(&mut data, event.clone(), "delivery", &p);
    let compact = payload(&data);
    assert_eq!(
        compact.detail.as_ref().unwrap().body.as_deref(),
        Some("ação 🌍")
    );
    assert!(compact.body_truncated);
    assert_eq!(
        data.event_history[&compact.event_id].event.detail.body,
        event.detail.body
    );
}

#[test]
fn bot_filter_is_conservative_and_does_not_discard_inline_findings() {
    let mut event = event("issue_comment:edited", Category::Comments);
    event.detail.actor = Some("coderabbitai[bot]".into());
    event.detail.body = Some(
        "<!-- This is an auto-generated comment: summarize by coderabbit.ai --> walkthrough".into(),
    );
    let mut data = Data::default();
    route(&mut data, &event, &subscriber(), &policy(), 0);
    assert!(data.jobs.is_empty());
    event.detail.path = Some("src/main.rs".into());
    route(&mut data, &event, &subscriber(), &policy(), 0);
    assert_eq!(data.jobs.len(), 1);
    data.jobs.clear();
    event.detail.path = None;
    event.detail.actor = Some("unknown[bot]".into());
    route(&mut data, &event, &subscriber(), &policy(), 0);
    assert_eq!(data.jobs.len(), 1, "unknown bot review must survive");
}

#[test]
fn explicit_actor_filter_and_meaningful_human_edits() {
    let mut event = event("issue_comment:created", Category::Comments);
    event.detail.actor = Some("Me".into());
    event.detail.body = Some("fix this".into());
    let mut p = policy();
    p.ignored_actors.insert("me".into());
    let mut data = Data::default();
    route(&mut data, &event, &subscriber(), &p, 0);
    assert!(data.jobs.is_empty());
    event.detail.actor = Some("reviewer".into());
    route(&mut data, &event, &subscriber(), &p, 1);
    data.jobs.clear();
    event.kind = "issue_comment:edited".into();
    event.snapshot.head_sha = Some("new-head".into());
    route(&mut data, &event, &subscriber(), &p, 2);
    assert!(
        data.jobs.is_empty(),
        "unchanged comment must not repeat after a push"
    );
    event.detail.body = Some("different request".into());
    route(&mut data, &event, &subscriber(), &p, 3);
    assert_eq!(data.jobs.len(), 1);
}

#[test]
fn legacy_all_and_per_subscription_overrides_remain_independent() {
    let mut data = Data::default();
    let mut full = subscriber();
    full.filter.notifications = Some(NotificationPolicy::default());
    data.subscribers.insert("consumer".into(), full);
    let mut compact = subscriber();
    compact.id = "compact".into();
    data.subscribers.insert("compact".into(), compact);
    normalize::persist_event_with_policy(
        &mut data,
        ci("check_run", "queued", None, "lint"),
        "d",
        &policy(),
    );
    assert_eq!(data.jobs.len(), 1);
    assert!(matches!(
        data.jobs.values().next(),
        Some(Job::Notify { .. })
    ));
}

#[test]
fn dedupe_and_pending_batches_survive_sqlite_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite3");
    let store = Store::open(&path).unwrap();
    let event = ci("workflow_run", "completed", Some("failure"), "CI");
    store
        .change(|data| {
            route(
                data,
                &event,
                &subscriber(),
                &policy(),
                chrono::Utc::now().timestamp_millis(),
            );
            Ok(())
        })
        .unwrap();
    // Read actual committed bytes without fighting flock inherited by unrelated test subprocesses.
    let mut persisted = Store::inspect(&path).unwrap();
    assert_eq!(persisted.notification_state.len(), 1);
    assert_eq!(persisted.jobs.len(), 1);
    persisted.jobs.clear();
    route(&mut persisted, &event, &subscriber(), &policy(), 2000);
    assert!(persisted.jobs.is_empty());
}

#[test]
fn terminal_pr_notice_survives_and_never_carries_description() {
    let mut event = event("merged", Category::Pr);
    event.final_event = true;
    event.detail.body = Some("long description".repeat(1000));
    let mut data = Data::default();
    route(&mut data, &event, &subscriber(), &policy(), 0);
    assert!(payload(&data).final_event);
    assert!(payload(&data).detail.as_ref().unwrap().body.is_none());
}

#[test]
fn policy_limits_reject_unknown_selectors_and_unbounded_payloads() {
    let mut p = policy();
    p.success_checks.insert("lint".into());
    assert!(p.validate().is_err());
    p.success_checks.clear();
    p.max_comment_chars = 0;
    assert!(p.validate().is_err());
    p.max_comment_chars = 8001;
    assert!(p.validate().is_err());
    p.max_comment_chars = 2000;
    p.batch_window_ms = 60001;
    assert!(p.validate().is_err());
}

// ---- agent_actionable: only failures, new comments/reviews, one digest per watch ----

fn actionable() -> NotificationPolicy {
    NotificationPolicy {
        profile: NotificationProfile::AgentActionable,
        ignore_self: true,
        resolved_self: Some("my-agent".into()),
        ..Default::default()
    }
}
fn digest(data: &Data) -> (&CompactEvent, i64) {
    let Job::ReviewNotify {
        payload, due_at, ..
    } = data.jobs.values().next().expect("digest job")
    else {
        panic!("expected digest")
    };
    (payload, *due_at)
}
fn comment(actor: &str, kind: &str, body: &str, entity: &str) -> PrEvent {
    let mut e = event(kind, Category::Comments);
    e.entity = entity.into();
    e.detail.actor = Some(actor.into());
    e.detail.body = Some(body.into());
    e
}

#[test]
fn actionable_drops_success_progress_cancellation_pushes_edits_and_self() {
    for event in [
        ci("check_run", "queued", None, "clippy"),
        ci("check_run", "in_progress", None, "clippy"),
        ci("status", "pending", None, "preview"),
        ci("check_run", "completed", Some("success"), "clippy"),
        ci("check_run", "completed", Some("cancelled"), "clippy"),
        ci("check_run", "completed", Some("skipped"), "clippy"),
        event("pull_request:synchronize", Category::Pr),
        event("pull_request:labeled", Category::Pr),
        comment("someone", "issue_comment:edited", "fixed typo", "7"),
        comment("My-Agent", "issue_comment:created", "pushed a fix", "8"),
    ] {
        let mut data = Data::default();
        route(&mut data, &event, &subscriber(), &actionable(), 1000);
        assert!(
            data.jobs.is_empty(),
            "unexpected notification for {}",
            event.kind
        );
    }
}

#[test]
fn actionable_coalesces_failures_comments_and_reviews_into_one_debounced_digest() {
    let mut data = Data::default();
    let (sub, p) = (subscriber(), actionable());
    route(
        &mut data,
        &ci("check_run", "completed", Some("failure"), "clippy"),
        &sub,
        &p,
        1_000,
    );
    assert_eq!(digest(&data).1, 16_000);
    route(
        &mut data,
        &comment("reviewer", "issue_comment:created", "please rename", "7"),
        &sub,
        &p,
        5_000,
    );
    let mut review = event("pull_request_review:submitted", Category::Reviews);
    review.entity = "9".into();
    review.detail.actor = Some("reviewer".into());
    review.detail.state = Some("changes_requested".into());
    route(&mut data, &review, &sub, &p, 10_000);
    assert_eq!(data.jobs.len(), 1, "one wake per watch");
    let (payload, due) = digest(&data);
    assert_eq!(due, 25_000, "quiet window restarts on each item");
    assert_eq!(payload.kind, "digest");
    let kinds: Vec<_> = payload.items.iter().map(|i| i.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "ci.failed",
            "issue_comment:created",
            "pull_request_review:submitted"
        ]
    );
    assert_eq!(payload.items[0].failure.as_ref().unwrap().name, "clippy");
    assert!(serde_json::to_value(payload)
        .unwrap()
        .get("snapshot")
        .is_none());
    route(
        &mut data,
        &comment("reviewer", "issue_comment:created", "please rename", "7"),
        &sub,
        &p,
        11_000,
    );
    assert_eq!(digest(&data).0.items.len(), 3, "redelivery is idempotent");
}

#[test]
fn actionable_max_wait_caps_debounce_and_final_event_flushes_now() {
    let mut data = Data::default();
    let (sub, p) = (subscriber(), actionable());
    for i in 0..9i64 {
        let c = comment(
            "human",
            "issue_comment:created",
            &format!("note {i}"),
            &i.to_string(),
        );
        route(&mut data, &c, &sub, &p, i * 14_000);
    }
    assert_eq!(digest(&data).1, 120_000, "max_wait_ms bounds a busy thread");
    let mut merged = event("merged", Category::Pr);
    merged.final_event = true;
    route(&mut data, &merged, &sub, &p, 113_000);
    let (payload, due) = digest(&data);
    assert_eq!(due, 113_000);
    assert!(payload.final_event);
}

#[test]
fn actionable_keeps_the_specific_job_over_its_failed_workflow() {
    let (sub, p) = (subscriber(), actionable());
    let job = ci("check_run", "completed", Some("failure"), "clippy");
    let mut workflow = ci("workflow_run", "completed", Some("failure"), "CI");
    let entry = workflow.snapshot.ci.remove("workflow_run:42").unwrap();
    workflow.snapshot.ci.insert("workflow_run:77".into(), entry);
    workflow.entity = "77".into();
    let names = |data: &Data| -> Vec<String> {
        digest(data)
            .0
            .items
            .iter()
            .map(|i| i.failure.clone().unwrap().name)
            .collect()
    };
    // Job first, then its workflow: the workflow is a duplicate.
    let mut data = Data::default();
    route(&mut data, &job, &sub, &p, 1_000);
    let mut later = workflow.clone();
    later.snapshot.ci.extend(job.snapshot.ci.clone());
    route(&mut data, &later, &sub, &p, 2_000);
    assert_eq!(names(&data), ["clippy"]);
    // Workflow first, then the job: the job replaces it.
    let mut data = Data::default();
    route(&mut data, &workflow, &sub, &p, 1_000);
    let mut specific = job.clone();
    specific.snapshot.ci.extend(workflow.snapshot.ci.clone());
    route(&mut data, &specific, &sub, &p, 2_000);
    assert_eq!(names(&data), ["clippy"]);
}

#[test]
fn actionable_delivery_drops_failures_of_a_superseded_head_but_keeps_comments() {
    let mut data = Data::default();
    let (sub, p) = (subscriber(), actionable());
    route(
        &mut data,
        &ci("check_run", "completed", Some("failure"), "clippy"),
        &sub,
        &p,
        1_000,
    );
    route(
        &mut data,
        &comment("reviewer", "issue_comment:created", "why?", "7"),
        &sub,
        &p,
        2_000,
    );
    let mut payload = digest(&data).0.clone();
    assert!(prune_digest(&mut payload, Some("new-head")));
    assert_eq!(payload.items.len(), 1);
    assert_eq!(payload.items[0].kind, "issue_comment:created");
    assert_eq!(payload.head_sha.as_deref(), Some("new-head"));
    let mut only_ci = digest(&data).0.clone();
    only_ci.items.truncate(1);
    assert!(!prune_digest(&mut only_ci, Some("new-head")));
}

#[test]
fn actionable_policy_validates_windows_and_rejects_runtime_fields() {
    assert!(actionable().validate().is_ok());
    assert!(NotificationPolicy {
        quiet_ms: 200_000,
        max_wait_ms: 100_000,
        ..actionable()
    }
    .validate()
    .is_err());
    assert!(NotificationPolicy {
        max_items: 0,
        ..actionable()
    }
    .validate()
    .is_err());
    let parsed: NotificationPolicy =
        serde_json::from_value(json!({"profile":"agent_actionable","ignore_self":true})).unwrap();
    assert_eq!(parsed.profile, NotificationProfile::AgentActionable);
    assert!(serde_json::from_value::<NotificationPolicy>(json!({"resolved_self":"x"})).is_err());
}
