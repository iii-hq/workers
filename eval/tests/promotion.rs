//! Promotion helpers against local fixtures: a tiny HTTP server standing in
//! for GitHub, Release Control and the Registry, the shared fake SDK engine
//! standing in for the target Compose daemon, and real local processes whose
//! `/proc/<pid>/exe` is hashed. None of this is production qualification:
//! it proves the helpers' checks and wire shapes, not the live services.

#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eval::promotion::*;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const BASE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CANDIDATE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TREE: &str = "cccccccccccccccccccccccccccccccccccccccc";
const MERGED: &str = "dddddddddddddddddddddddddddddddddddddddd";
const PLAN_HASH: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const TARGET: &str = "x86_64-unknown-linux-gnu";

// ── HTTP fixture ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct Seen {
    headers: HashMap<String, String>,
    body: String,
}

type Routes = Arc<Mutex<HashMap<String, VecDeque<(u16, Vec<u8>)>>>>;
type Log = Arc<Mutex<Vec<(String, Seen)>>>;

struct Http {
    url: String,
    routes: Routes,
    log: Log,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Http {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Http {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let routes: Routes = Arc::default();
        let log: Log = Arc::default();
        let (served, logged) = (routes.clone(), log.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let (routes, log) = (served.clone(), logged.clone());
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let header_end = loop {
                        let read = socket.read(&mut chunk).await.unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
                    let mut lines = head.split("\r\n");
                    let request_line = lines.next().unwrap_or_default().to_string();
                    let headers: HashMap<String, String> = lines
                        .filter_map(|line| line.split_once(':'))
                        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                        .collect();
                    let length: usize = headers
                        .get("content-length")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0);
                    while buffer.len() < header_end + length {
                        let read = socket.read(&mut chunk).await.unwrap_or(0);
                        if read == 0 {
                            break;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                    }
                    let body = String::from_utf8_lossy(&buffer[header_end..]).to_string();
                    let mut parts = request_line.split(' ');
                    let key = format!(
                        "{} {}",
                        parts.next().unwrap_or_default(),
                        parts.next().unwrap_or_default()
                    );
                    log.lock()
                        .unwrap()
                        .push((key.clone(), Seen { headers, body }));
                    let reply = {
                        let mut routes = routes.lock().unwrap();
                        match routes.get_mut(&key) {
                            Some(queue) if queue.len() > 1 => queue.pop_front(),
                            Some(queue) => queue.front().cloned(),
                            None => None,
                        }
                    };
                    let (status, payload) =
                        reply.unwrap_or((404, br#"{"error":"no route"}"#.to_vec()));
                    // Status 0 drops the connection unanswered: a lost reply.
                    if status == 0 {
                        return;
                    }
                    let response = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        payload.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.write_all(&payload).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self {
            url,
            routes,
            log,
            task,
        }
    }

    fn route(&self, key: &str, status: u16, body: impl Into<Vec<u8>>) {
        self.routes
            .lock()
            .unwrap()
            .insert(key.to_string(), VecDeque::from([(status, body.into())]));
    }

    fn json(&self, key: &str, status: u16, body: Value) {
        self.route(key, status, body.to_string());
    }

    /// Answers in order; the last answer repeats.
    fn sequence(&self, key: &str, answers: Vec<(u16, Value)>) {
        self.routes.lock().unwrap().insert(
            key.to_string(),
            answers
                .into_iter()
                .map(|(status, body)| (status, body.to_string().into_bytes()))
                .collect(),
        );
    }

    fn seen(&self, key: &str) -> Vec<Seen> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(seen, _)| seen == key)
            .map(|(_, seen)| seen.clone())
            .collect()
    }
}

// ── setup ───────────────────────────────────────────────────────────────────────

/// The fixed credential variables, set to the same values by every test so
/// parallel tests never race on them.
fn set_credentials() {
    std::env::set_var(GITHUB_TOKEN_ENV, "token-secret");
    std::env::set_var(RELEASE_CONTROL_SESSION_ENV, "session-secret");
}

/// Fixture origins, injected the way the worker's bootstrap injects the
/// production ones.
fn origins(base_url: &str) -> PromotionOrigins {
    PromotionOrigins {
        release_control_url: base_url.to_string(),
        github_api_url: base_url.to_string(),
        registry_url: base_url.to_string(),
    }
}

fn config(target: ActivationTarget) -> PromotionConfig {
    PromotionConfig {
        artifact_target: TARGET.to_string(),
        target,
    }
}

fn local_target(compose_file: &str) -> ActivationTarget {
    ActivationTarget {
        engine_url: "ws://127.0.0.1:49999".into(),
        namespace: "promotion-target".into(),
        compose_file: compose_file.into(),
    }
}

/// A client for tests that never reach Compose.
fn idle_engine() -> Arc<IIIClient> {
    Arc::new(register_worker("ws://127.0.0.1:9", InitOptions::default()))
}

async fn http_promotion() -> (Http, Promotion) {
    set_credentials();
    let http = Http::start().await;
    let promotion = Promotion::new(
        config(local_target("/tmp/worker-compose.yaml")),
        origins(&http.url),
        idle_engine(),
    )
    .unwrap();
    (http, promotion)
}

fn approval() -> SourceApproval {
    SourceApproval {
        candidate_id: "c42".into(),
        outcome: ApprovalOutcome::Accepted,
        policy_revision: "policy-7".into(),
        base_sha: BASE.into(),
        candidate_sha: CANDIDATE.into(),
        candidate_tree: TREE.into(),
        evidence_urls: vec!["https://evidence.example/run/1".into()],
    }
}

fn merge() -> MergeEvidence {
    MergeEvidence {
        pull_request: 7,
        base_sha: BASE.into(),
        candidate_sha: CANDIDATE.into(),
        candidate_tree: TREE.into(),
        merged_sha: MERGED.into(),
        merged_tree: TREE.into(),
        main_sha: MERGED.into(),
    }
}

fn rc_preview(source: &str, blocking: Value) -> Value {
    json!({
        "planId": "6f1c2a52-6a3e-4f43-9a55-1f0f7c1d2e3f",
        "planHash": PLAN_HASH,
        "expiresAt": "2026-10-23T12:15:00.000Z",
        "preview": {
            "pipeline": "worker-deployment",
            "sourceSha": source,
            "blocked": !blocking.is_null(),
            "plan": [{
                "worker": "harness", "channel": "next", "sourceSha": source,
                "targetVersion": "0.9.1-rc.1", "blocking": blocking,
                "snapshots": {
                    "next": { "version": "0.9.0-rc.3", "availability": "found" },
                    "latest": { "version": "0.9.0", "availability": "found" }
                }
            }]
        }
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A gzip tar like `normalized_tar` writes: each entry `(name, type,
/// contents or link target)`.
fn tar_gz(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (name, kind, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(*kind);
        header.set_mode(0o755);
        if kind.is_symlink() || kind.is_hard_link() {
            header.set_size(0);
            header
                .set_link_name(std::str::from_utf8(data).unwrap())
                .unwrap();
            builder
                .append_data(&mut header, name, std::io::empty())
                .unwrap();
        } else {
            header.set_size(data.len() as u64);
            builder.append_data(&mut header, name, *data).unwrap();
        }
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn rc_artifact(archive: &str) -> ReleaseArtifact {
    ReleaseArtifact {
        version: "0.9.1-rc.1".into(),
        tags: vec!["next".into()],
        target: TARGET.into(),
        archive_url: "https://example/harness.tar.gz".into(),
        archive_sha256: archive.into(),
        binary_sha256: sha256(b"executable"),
        source_sha: MERGED.into(),
        operation_id: "op-1".into(),
        deployment_id: "dep-1".into(),
        build_run_id: Some(11),
    }
}

// ── approvals ────────────────────────────────────────────────────────────────────

#[test]
fn only_an_accepted_exact_source_approval_authorizes_a_merge() {
    assert!(check_source_approval(&approval()).is_ok());

    let mut rejected = approval();
    rejected.outcome = ApprovalOutcome::Rejected;
    assert_eq!(
        check_source_approval(&rejected).unwrap_err().kind,
        StopKind::Rejected
    );

    let mut unsure = approval();
    unsure.outcome = ApprovalOutcome::Inconclusive;
    assert_eq!(
        check_source_approval(&unsure).unwrap_err().kind,
        StopKind::Inconclusive
    );

    let mut short = approval();
    short.candidate_tree = "cccc".into();
    assert_eq!(
        check_source_approval(&short).unwrap_err().kind,
        StopKind::Rejected
    );

    let mut hostile = approval();
    hostile.candidate_id = "x;rm -rf".into();
    assert!(check_source_approval(&hostile).is_err());
    assert_eq!(candidate_branch("c42"), "feat/loop-candidate-c42");
}

#[test]
fn artifact_approval_binds_version_source_archive_and_the_running_binary() {
    let archive = sha256(b"archive");
    let binary = sha256(b"executable");
    let rc = rc_artifact(&archive);
    let approved = ArtifactApproval {
        outcome: ApprovalOutcome::Accepted,
        policy_revision: "policy-7".into(),
        version: rc.version.clone(),
        source_sha: MERGED.into(),
        archive_sha256: archive.clone(),
        binary_sha256: Some(binary.clone()),
    };
    let validated = check_artifact_approval(&approved, &rc).unwrap();
    assert_eq!(validated.archive_sha256, archive);
    assert_eq!(
        validated.binary_sha256, binary,
        "archive and executable digests are kept apart"
    );

    let cached_build = ArtifactApproval {
        binary_sha256: Some(sha256(b"a cached build of the same version")),
        ..approved.clone()
    };
    assert_eq!(
        check_artifact_approval(&cached_build, &rc)
            .unwrap_err()
            .kind,
        StopKind::Rejected,
        "the E2E must have run the executable the archive installs"
    );

    let unknown = ArtifactApproval {
        binary_sha256: None,
        ..approved.clone()
    };
    assert_eq!(
        check_artifact_approval(&unknown, &rc).unwrap_err().kind,
        StopKind::Inconclusive,
        "an unmeasured running binary never authorizes finalization"
    );
    let other_archive = ArtifactApproval {
        archive_sha256: sha256(b"other"),
        ..approved.clone()
    };
    assert_eq!(
        check_artifact_approval(&other_archive, &rc)
            .unwrap_err()
            .kind,
        StopKind::Rejected
    );
    let source_only = ArtifactApproval {
        source_sha: CANDIDATE.into(),
        ..approved
    };
    assert!(check_artifact_approval(&source_only, &rc).is_err());

    let mut stable = rc_artifact(&archive);
    stable.version = "0.9.1".into();
    let promoted = check_republished(&validated, &stable).unwrap();
    assert_eq!(promoted.version, "0.9.1");
    assert_eq!(promoted.binary_sha256, binary);
    let mut other_binary = stable.clone();
    other_binary.binary_sha256 = sha256(b"another executable");
    assert_eq!(
        check_republished(&validated, &other_binary)
            .unwrap_err()
            .kind,
        StopKind::Rejected
    );
    stable.archive_sha256 = sha256(b"rebuilt");
    assert_eq!(
        check_republished(&validated, &stable).unwrap_err().kind,
        StopKind::Rejected,
        "a rebuilt stable is not the validated rc"
    );
    assert_eq!(stable_for_candidate("0.9.1-rc.1").as_deref(), Some("0.9.1"));
    assert_eq!(stable_for_candidate("0.9.1"), None);
    assert_eq!(stable_for_candidate("0.9.1-rc.0"), None);
}

// ── GitHub ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_pull_request_is_reused_only_for_the_exact_candidate_head() {
    let (http, promotion) = http_promotion().await;
    let branch = "/repos/iii-hq/workers/git/ref/heads/feat/loop-candidate-c42";
    let search =
        "/repos/iii-hq/workers/pulls?head=iii-hq:feat/loop-candidate-c42&state=all&per_page=100";
    http.json(
        &format!("GET {branch}"),
        200,
        json!({ "object": { "sha": BASE } }),
    );
    let diverged = promotion
        .ensure_pull_request(&approval())
        .await
        .unwrap_err();
    assert_eq!(diverged.kind, StopKind::Rejected, "{diverged}");
    assert!(http.seen(&format!("GET {search}")).is_empty());

    http.json(
        &format!("GET {branch}"),
        200,
        json!({ "object": { "sha": CANDIDATE } }),
    );
    http.json(
        &format!("GET {search}"),
        200,
        json!([{ "number": 7, "state": "open", "merged_at": null, "html_url": "https://github.com/iii-hq/workers/pull/7",
                 "head": { "sha": CANDIDATE, "ref": "feat/loop-candidate-c42" } }]),
    );
    let found = promotion.ensure_pull_request(&approval()).await.unwrap();
    assert_eq!((found.number, found.created), (7, false));
    assert!(http.seen("POST /repos/iii-hq/workers/pulls").is_empty());
    let seen = &http.seen(&format!("GET {search}"))[0];
    assert_eq!(
        seen.headers.get("authorization").map(String::as_str),
        Some("Bearer token-secret"),
        "the controller token comes from the named variable"
    );
}

const PULL: &str = "GET /repos/iii-hq/workers/pulls/7";
const MAIN: &str = "GET /repos/iii-hq/workers/git/ref/heads/main";
const UPDATE: &str = "PATCH /repos/iii-hq/workers/git/refs/heads/main";
const SQUASH: &str = "PUT /repos/iii-hq/workers/pulls/7/merge";

fn candidate_commit_key() -> String {
    format!("GET /repos/iii-hq/workers/git/commits/{CANDIDATE}")
}

fn candidate_commit(parents: &[&str], tree: &str) -> Value {
    json!({ "sha": CANDIDATE, "tree": { "sha": tree },
            "parents": parents.iter().map(|sha| json!({ "sha": sha })).collect::<Vec<_>>() })
}

fn main_at(sha: &str) -> (u16, Value) {
    (200, json!({ "object": { "sha": sha } }))
}

/// A clean open pull request for the candidate, whose commit is one commit
/// on the approved base with the approved tree.
fn fast_forward_rig(http: &Http) {
    http.json(
        &candidate_commit_key(),
        200,
        candidate_commit(&[BASE], TREE),
    );
    http.json(
        PULL,
        200,
        json!({ "number": 7, "state": "open", "merged": false, "mergeable_state": "clean",
                "head": { "sha": CANDIDATE }, "base": { "ref": "main" } }),
    );
}

#[tokio::test]
async fn main_is_fast_forwarded_only_from_the_approved_base_and_proved_on_read() {
    let (http, promotion) = http_promotion().await;
    fast_forward_rig(&http);
    http.sequence(MAIN, vec![main_at(BASE)]);
    http.json(UPDATE, 200, json!({ "object": { "sha": CANDIDATE } }));
    let requested = promotion.fast_forward_main(&approval(), 7).await.unwrap();
    assert!(requested.updated);
    let body: Value = serde_json::from_str(&http.seen(UPDATE)[0].body).unwrap();
    assert_eq!(
        body,
        json!({ "sha": CANDIDATE, "force": false }),
        "never a forced update"
    );
    assert!(http.seen(SQUASH).is_empty(), "no merge or squash commit");

    http.sequence(MAIN, vec![main_at(CANDIDATE)]);
    let merged = promotion.read_merge(&approval(), 7).await.unwrap();
    assert_eq!(merged.merged_sha, CANDIDATE);
    assert_eq!(merged.main_sha, CANDIDATE);
    assert_eq!(merged.merged_tree, TREE);

    // main moved on later: the candidate still counts if main contains it.
    http.sequence(MAIN, vec![main_at(MERGED)]);
    let compare = format!("GET /repos/iii-hq/workers/compare/{CANDIDATE}...{MERGED}");
    http.json(&compare, 200, json!({ "status": "ahead" }));
    assert_eq!(
        promotion.read_merge(&approval(), 7).await.unwrap().main_sha,
        MERGED
    );
    http.json(&compare, 200, json!({ "status": "diverged" }));
    assert_eq!(
        promotion.read_merge(&approval(), 7).await.unwrap_err().kind,
        StopKind::Rejected
    );
}

#[tokio::test]
async fn a_main_that_moved_before_the_check_is_never_updated() {
    let (http, promotion) = http_promotion().await;
    fast_forward_rig(&http);
    // Even an empty commit with the base's tree is a different base.
    http.sequence(MAIN, vec![main_at(MERGED)]);
    let advanced = promotion
        .fast_forward_main(&approval(), 7)
        .await
        .unwrap_err();
    assert_eq!(advanced.kind, StopKind::Rejected);
    assert!(advanced.reason.contains("advanced"), "{advanced}");
    assert!(http.seen(UPDATE).is_empty());
}

#[tokio::test]
async fn a_race_that_moves_main_after_the_check_is_refused_by_the_server() {
    let (http, promotion) = http_promotion().await;
    fast_forward_rig(&http);
    http.sequence(MAIN, vec![main_at(BASE), main_at(MERGED)]);
    http.json(
        UPDATE,
        422,
        json!({ "message": "Update is not a fast forward leaked-secret-value" }),
    );
    let raced = promotion
        .fast_forward_main(&approval(), 7)
        .await
        .unwrap_err();
    assert_eq!(raced.kind, StopKind::Rejected);
    assert!(raced.reason.contains("advanced"), "{raced}");
    assert!(!raced.reason.contains("leaked-secret"), "{raced}");
    assert_eq!(http.seen(UPDATE).len(), 1);
    assert!(http.seen(SQUASH).is_empty(), "no substitute merge");
}

#[tokio::test]
async fn repository_rules_refusing_the_update_stop_without_another_merge() {
    let (http, promotion) = http_promotion().await;
    fast_forward_rig(&http);
    http.sequence(MAIN, vec![main_at(BASE)]);
    http.json(
        UPDATE,
        403,
        json!({ "message": "Protected branch update failed leaked-secret-value" }),
    );
    let refused = promotion
        .fast_forward_main(&approval(), 7)
        .await
        .unwrap_err();
    assert_eq!(refused.kind, StopKind::Rejected);
    assert!(refused.reason.contains("repository rules"), "{refused}");
    assert!(!refused.reason.contains("leaked-secret"), "{refused}");
    assert!(http.seen(SQUASH).is_empty(), "no substitute merge");
}

#[tokio::test]
async fn a_lost_update_reply_is_reconciled_without_a_second_update() {
    let (http, promotion) = http_promotion().await;
    fast_forward_rig(&http);
    http.sequence(MAIN, vec![main_at(BASE), main_at(CANDIDATE)]);
    http.route(UPDATE, 0, Vec::new());
    let lost = promotion
        .fast_forward_main(&approval(), 7)
        .await
        .unwrap_err();
    assert_eq!(lost.kind, StopKind::Inconclusive, "{lost}");
    let merged = promotion.read_merge(&approval(), 7).await.unwrap();
    assert_eq!(merged.main_sha, CANDIDATE);
    let again = promotion.fast_forward_main(&approval(), 7).await.unwrap();
    assert!(!again.updated, "main already is the candidate");
    assert_eq!(http.seen(UPDATE).len(), 1, "one update, ever");
}

#[tokio::test]
async fn only_one_commit_on_the_approved_base_with_its_tree_is_fast_forwarded() {
    let (http, promotion) = http_promotion().await;
    fast_forward_rig(&http);
    http.sequence(MAIN, vec![main_at(BASE)]);
    for commit in [
        candidate_commit(&[BASE, MERGED], TREE),
        candidate_commit(&[MERGED], TREE),
        candidate_commit(&[BASE], BASE),
    ] {
        http.json(&candidate_commit_key(), 200, commit);
        assert_eq!(
            promotion
                .fast_forward_main(&approval(), 7)
                .await
                .unwrap_err()
                .kind,
            StopKind::Rejected
        );
    }
    assert!(http.seen(UPDATE).is_empty());
}

#[tokio::test]
async fn remote_error_fields_never_reach_a_reason() {
    let (http, promotion) = http_promotion().await;
    for field in ["code", "error", "message"] {
        for status in [400u16, 403, 404, 409, 422, 500] {
            let body = json!({ (field): "leaked-secret-value" });
            http.json("POST /operations/preview", status, body.clone());
            let preview = promotion.preview_rc(&merge()).await.unwrap_err();
            assert!(
                !preview.to_string().contains("leaked-secret"),
                "{field} {status}: {preview}"
            );
            http.json(&candidate_commit_key(), status, body);
            let github = promotion
                .fast_forward_main(&approval(), 7)
                .await
                .unwrap_err();
            assert!(
                !github.to_string().contains("leaked-secret"),
                "{field} {status}: {github}"
            );
        }
    }
    http.json(
        "POST /operations/preview",
        200,
        rc_preview(MERGED, json!("leaked-secret-value")),
    );
    let blocked = promotion.preview_rc(&merge()).await.unwrap_err();
    assert!(!blocked.to_string().contains("leaked-secret"), "{blocked}");
}

// ── Release Control and Registry ─────────────────────────────────────────────────────────

#[tokio::test]
async fn the_rc_plan_is_frozen_and_a_repeated_confirmation_returns_the_original_operation() {
    let (http, promotion) = http_promotion().await;
    http.json(
        "POST /operations/preview",
        200,
        rc_preview(MERGED, Value::Null),
    );
    http.json(
        "POST /operations",
        201,
        json!({ "operation": { "id": "op-1", "kind": "deploy" } }),
    );

    let plan = promotion.preview_rc(&merge()).await.unwrap();
    assert_eq!(plan.kind, ReleaseKind::Deploy);
    assert_eq!(plan.source_sha, MERGED);
    assert_eq!(plan.target_version, "0.9.1-rc.1");
    assert_eq!(plan.prior_latest.as_deref(), Some("0.9.0"));
    assert_eq!(plan.plan_hash, PLAN_HASH);
    let preview = &http.seen("POST /operations/preview")[0];
    assert_eq!(
        serde_json::from_str::<Value>(&preview.body).unwrap(),
        json!({ "type": "deploy", "sourceSha": MERGED, "workers": [{ "name": "harness",
                "version": { "mode": "bump", "bump": "patch", "suffix": "rc" }, "channel": "next" }] })
    );
    assert_eq!(
        preview.headers.get("cookie").map(String::as_str),
        Some("rc_session=session-secret")
    );

    let first = promotion.confirm_plan(&plan).await.unwrap();
    let lost_and_retried = promotion.confirm_plan(&plan).await.unwrap();
    assert_eq!(first, lost_and_retried);
    let confirms = http.seen("POST /operations");
    assert_eq!(confirms.len(), 2);
    assert_eq!(
        confirms[0].body, confirms[1].body,
        "the same envelope, never a new preview"
    );
    assert_eq!(http.seen("POST /operations/preview").len(), 1);

    let mut journal = PromotionJournal::new("p1");
    journal.push(PromotionPhase::RcPlanned(plan));
    journal.push(PromotionPhase::RcConfirmed(first));
    let persisted = serde_json::to_string(&journal).unwrap();
    assert!(!persisted.contains("session-secret"), "{persisted}");
    let restored: PromotionJournal = serde_json::from_str(&persisted).unwrap();
    assert_eq!(restored, journal);
}

#[tokio::test]
async fn a_plan_for_another_source_or_a_blocked_plan_stops() {
    let (http, promotion) = http_promotion().await;
    http.json(
        "POST /operations/preview",
        200,
        rc_preview(BASE, Value::Null),
    );
    assert_eq!(
        promotion.preview_rc(&merge()).await.unwrap_err().kind,
        StopKind::Rejected
    );
    http.json(
        "POST /operations/preview",
        200,
        rc_preview(
            MERGED,
            json!("harness: Registry channel snapshot is unavailable"),
        ),
    );
    let blocked = promotion.preview_rc(&merge()).await.unwrap_err();
    assert_eq!(blocked.kind, StopKind::Rejected);
    assert!(blocked.reason.contains("1 blocking"), "{blocked}");
    assert!(
        !blocked.reason.contains("snapshot"),
        "Release Control's own words stay there: {blocked}"
    );
}

/// Runs only inside the child process the next test starts with the
/// credential variables removed; a no-op in the ordinary run.
#[tokio::test]
async fn missing_session_child() {
    if std::env::var("PROMOTION_CHILD").as_deref() != Ok("1") {
        return;
    }
    let http = Http::start().await;
    let promotion = Promotion::new(
        config(local_target("/tmp/worker-compose.yaml")),
        origins(&http.url),
        idle_engine(),
    )
    .unwrap();
    let missing = promotion.preview_rc(&merge()).await.unwrap_err();
    assert_eq!(missing.kind, StopKind::Unavailable, "{missing}");
    assert!(missing.reason.contains(RELEASE_CONTROL_SESSION_ENV));
    assert!(
        http.seen("POST /operations/preview").is_empty(),
        "nothing is sent"
    );
    let no_token = promotion
        .ensure_pull_request(&approval())
        .await
        .unwrap_err();
    assert_eq!(no_token.kind, StopKind::Unavailable, "{no_token}");
}

#[test]
fn a_missing_session_is_unavailable_and_sends_nothing() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["missing_session_child", "--exact", "--test-threads=1"])
        .env("PROMOTION_CHILD", "1")
        .env_remove(RELEASE_CONTROL_SESSION_ENV)
        .env_remove(GITHUB_TOKEN_ENV)
        .status()
        .expect("run the child test");
    assert!(status.success());
}

#[tokio::test]
async fn expired_sessions_and_stale_plans_never_echo_bodies() {
    let (http, promotion) = http_promotion().await;
    http.json(
        "POST /operations/preview",
        401,
        json!({ "error": "unauthorized", "echo": "session-secret-expired" }),
    );
    let expired = promotion.preview_rc(&merge()).await.unwrap_err();
    assert_eq!(expired.kind, StopKind::Unavailable);
    assert!(!expired.to_string().contains("session-secret"), "{expired}");

    http.json(
        "POST /operations",
        409,
        json!({ "error": "stale plan", "code": "stale_plan", "echo": "session-secret-expired" }),
    );
    let plan = ReleasePlan {
        kind: ReleaseKind::Deploy,
        plan_id: "p".into(),
        plan_hash: PLAN_HASH.into(),
        expires_at: String::new(),
        source_sha: MERGED.into(),
        target_version: "0.9.1-rc.1".into(),
        candidate_version: None,
        channel: "next".into(),
        prior_next: None,
        prior_latest: Some("0.9.0".into()),
        preview_sha256: String::new(),
    };
    let stale = promotion.confirm_plan(&plan).await.unwrap_err();
    assert_eq!(stale.kind, StopKind::Rejected);
    assert!(stale.reason.contains("stale"), "{stale}");
    assert!(!stale.reason.contains("session-secret"));
}

#[tokio::test]
async fn the_operation_and_the_registry_archive_must_be_the_planned_build() {
    let (http, promotion) = http_promotion().await;
    let plan = ReleasePlan {
        kind: ReleaseKind::Deploy,
        plan_id: "p".into(),
        plan_hash: PLAN_HASH.into(),
        expires_at: String::new(),
        source_sha: MERGED.into(),
        target_version: "0.9.1-rc.1".into(),
        candidate_version: None,
        channel: "next".into(),
        prior_next: Some("0.9.0-rc.3".into()),
        prior_latest: Some("0.9.0".into()),
        preview_sha256: String::new(),
    };
    let confirmed = ConfirmedPlan {
        plan,
        operation_id: "op-1".into(),
    };
    let deployment = |version: &str, status: &str| {
        json!({ "operation": { "id": "op-1" }, "deployments": [{
            "id": "dep-1", "worker": "harness", "version": version, "channel": "next",
            "fromVersion": null, "sourceSha": MERGED, "status": status, "statusDetail": null,
            "descriptorSha256": PLAN_HASH,
            "executions": [{ "runId": 11, "conclusion": "success", "hasManifest": true }]
        }] })
    };
    http.json(
        "GET /operations/op-1",
        200,
        deployment("0.9.1-rc.1", "converging"),
    );
    assert!(matches!(
        promotion.read_operation(&confirmed).await.unwrap(),
        OperationState::Converging { .. }
    ));
    http.json(
        "GET /operations/op-1",
        200,
        deployment("0.9.1-rc.2", "converged"),
    );
    assert_eq!(
        promotion.read_operation(&confirmed).await.unwrap_err().kind,
        StopKind::Rejected,
        "a different version is a divergence, not progress"
    );
    http.json(
        "GET /operations/op-1",
        200,
        deployment("0.9.1-rc.1", "converged"),
    );
    let OperationState::Converged(evidence) = promotion.read_operation(&confirmed).await.unwrap()
    else {
        panic!("converged");
    };
    assert_eq!(evidence.build_run_id, Some(11));

    let executable = b"\x7fELF the harness executable".to_vec();
    let archive = tar_gz(&[
        ("harness", tar::EntryType::Regular, &executable),
        ("libcompanion.so", tar::EntryType::Regular, b"companion"),
    ]);
    let versions = |claimed: &str| {
        json!({ "versions": [{ "version": "0.9.1-rc.1", "tags": ["next"],
            "binaries": { (TARGET): { "url": format!("{}/harness.tar.gz", http.url), "sha256": claimed } } }] })
    };
    http.route("GET /harness.tar.gz", 200, archive.clone());
    http.json(
        "GET /w/harness/versions",
        200,
        versions(&sha256(b"something else")),
    );
    assert_eq!(
        promotion.read_artifact(&evidence).await.unwrap_err().kind,
        StopKind::Rejected,
        "the Registry's claim must match the bytes"
    );
    http.json("GET /w/harness/versions", 200, versions(&sha256(&archive)));
    let artifact = promotion.read_artifact(&evidence).await.unwrap();
    assert_eq!(artifact.archive_sha256, sha256(&archive));
    assert_eq!(
        artifact.binary_sha256,
        sha256(&executable),
        "the executable inside the archive, hashed from the stream"
    );
    assert_ne!(artifact.binary_sha256, artifact.archive_sha256);
    assert_eq!(artifact.source_sha, MERGED);
    assert_eq!(artifact.build_run_id, Some(11));

    // Produced by deployment_train.py::normalized_tar, independently of
    // the Rust tar writer used by the hostile archive fixtures below.
    let pipeline = include_bytes!("fixtures/release/harness-pipeline.tar.gz");
    http.route("GET /harness.tar.gz", 200, pipeline.to_vec());
    http.json("GET /w/harness/versions", 200, versions(&sha256(pipeline)));
    let packaged = promotion.read_artifact(&evidence).await.unwrap();
    assert_eq!(
        packaged.binary_sha256,
        sha256(b"\x7fELF official-normalized-tar-fixture\n")
    );

    // Only one regular harness entry identifies the executable.
    let refused: [(&str, Vec<u8>); 5] = [
        (
            "duplicate",
            tar_gz(&[
                ("harness", tar::EntryType::Regular, &executable),
                ("./harness", tar::EntryType::Regular, b"other bytes"),
            ]),
        ),
        (
            "symlink",
            tar_gz(&[("harness", tar::EntryType::Symlink, b"/usr/bin/sh")]),
        ),
        (
            "hardlink",
            tar_gz(&[
                ("real", tar::EntryType::Regular, &executable),
                ("harness", tar::EntryType::Link, b"real"),
            ]),
        ),
        (
            "missing",
            tar_gz(&[("not-harness", tar::EntryType::Regular, &executable)]),
        ),
        ("not a tar.gz", b"plain bytes".to_vec()),
    ];
    for (case, bad) in refused {
        http.route("GET /harness.tar.gz", 200, bad.clone());
        http.json("GET /w/harness/versions", 200, versions(&sha256(&bad)));
        let error = promotion.read_artifact(&evidence).await.unwrap_err();
        assert_eq!(error.kind, StopKind::Rejected, "{case}: {error}");
    }
}

#[tokio::test]
async fn finalization_requires_next_on_the_approved_rc_and_latest_where_it_was_frozen() {
    let (http, promotion) = http_promotion().await;
    let archive = sha256(b"archive");
    let rc = rc_artifact(&archive);
    let approved = ValidatedArtifact {
        version: rc.version.clone(),
        archive_sha256: archive.clone(),
        binary_sha256: sha256(b"executable"),
    };
    let rc_plan = ReleasePlan {
        kind: ReleaseKind::Deploy,
        plan_id: "p".into(),
        plan_hash: PLAN_HASH.into(),
        expires_at: String::new(),
        source_sha: MERGED.into(),
        target_version: rc.version.clone(),
        candidate_version: None,
        channel: "next".into(),
        prior_next: Some("0.9.0-rc.3".into()),
        prior_latest: Some("0.9.0".into()),
        preview_sha256: String::new(),
    };
    let preview = |next: &str, latest: &str| {
        json!({ "planId": "f", "planHash": PLAN_HASH, "expiresAt": "later", "preview": {
            "pipeline": "release-finalization", "blocked": false, "plan": [{
                "worker": "harness", "candidateVersion": "0.9.1-rc.1", "stableVersion": "0.9.1",
                "channel": "latest", "sourceSha": MERGED, "expectedNextVersion": next,
                "expectedLatestVersion": latest, "blocking": null }] } })
    };
    http.json(
        "POST /operations/preview",
        200,
        preview("0.9.1-rc.2", "0.9.0"),
    );
    assert_eq!(
        promotion
            .preview_finalize(&rc, &approved, &rc_plan)
            .await
            .unwrap_err()
            .kind,
        StopKind::Rejected,
        "a newer rc on next must be tested, not promoted"
    );
    http.json(
        "POST /operations/preview",
        200,
        preview("0.9.1-rc.1", "0.9.0-hotfix"),
    );
    assert!(promotion
        .preview_finalize(&rc, &approved, &rc_plan)
        .await
        .is_err());
    http.json(
        "POST /operations/preview",
        200,
        preview("0.9.1-rc.1", "0.9.0"),
    );
    let plan = promotion
        .preview_finalize(&rc, &approved, &rc_plan)
        .await
        .unwrap();
    assert_eq!(plan.kind, ReleaseKind::FinalizeRelease);
    assert_eq!(plan.target_version, "0.9.1");
    assert_eq!(plan.candidate_version.as_deref(), Some("0.9.1-rc.1"));
    let sent: Value =
        serde_json::from_str(&http.seen("POST /operations/preview").last().unwrap().body).unwrap();
    assert_eq!(
        sent,
        json!({ "type": "finalize_release", "workers": [{ "name": "harness", "candidateVersion": "0.9.1-rc.1" }] })
    );
}

// ── Compose activation over the SDK wire ─────────────────────────────────────────────────

#[derive(Default)]
struct Daemon {
    namespace: String,
    file: String,
    harness: Value,
    operations: HashMap<String, Value>,
    calls: Vec<(String, Value)>,
}

impl Daemon {
    fn respond(&mut self, function: &str, data: &Value) -> Result<Value, String> {
        self.calls.push((function.to_string(), data.clone()));
        match function {
            "compose::status" => Ok(json!({
                "namespace": self.namespace, "file": self.file, "daemon_pid": 1,
                "state_dir": "/tmp/state", "containers": [self.harness.clone()],
            })),
            "compose::operation" => {
                let id = data["progress_operation_id"].as_str().unwrap_or_default();
                self.operations
                    .get(id)
                    .cloned()
                    .ok_or_else(|| format!("unknown compose operation '{id}'"))
            }
            "compose::add" => {
                let id = data["operation_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                self.operations.insert(
                    id.clone(),
                    json!({ "operation_id": id, "status": "running" }),
                );
                Ok(json!({ "operation_id": id, "status": "accepted", "requested": 1 }))
            }
            other => Err(format!("unexpected {other}")),
        }
    }

    fn adds(&self) -> Vec<Value> {
        self.calls
            .iter()
            .filter(|(function, _)| function == "compose::add")
            .map(|(_, data)| data.clone())
            .collect()
    }
}

/// A local process group running a copy of a system tool renamed
/// `harness`, killed with its whole group and its copy removed on drop.
struct Proc {
    child: std::process::Child,
    dir: std::path::PathBuf,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &format!("-{}", self.child.id())])
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// This test executable, copied to a fresh directory as `harness` with a
/// `variant` trailer appended: the program runs the same (an ELF ignores
/// trailing bytes) but each variant has its own digest. System tools do not
/// work here: uutils coreutils is one multi-call binary that dispatches on
/// its file name and refuses to run as `harness`.
fn harness_copy(variant: &str) -> (std::path::PathBuf, std::path::PathBuf, String) {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "promotion-harness-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("harness");
    let mut bytes = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    bytes.extend_from_slice(format!("\npromotion-variant:{variant}\n").as_bytes());
    std::fs::write(&path, &bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, path, sha256(&bytes))
}

/// What a `harness` copy runs: this one test, which sleeps when asked.
const SLEEP_CHILD: [&str; 4] = ["sleep_child", "--exact", "--test-threads=1", "--quiet"];

/// Run inside a `harness` copy: just stay alive to be observed.
#[test]
fn sleep_child() {
    if std::env::var("PROMOTION_SLEEP").as_deref() == Ok("1") {
        std::thread::sleep(Duration::from_secs(120));
    }
}

fn harness_children(pid: u32) -> Vec<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|child| child.parse().ok())
        .filter(|child| {
            std::fs::read_link(format!("/proc/{child}/exe"))
                .is_ok_and(|exe| exe.file_name().is_some_and(|name| name == "harness"))
        })
        .collect()
}

/// A `harness` copy run directly: Compose's pid is the Harness pid.
fn spawn(variant: &str) -> (Proc, u32, String) {
    use std::os::unix::process::CommandExt;
    let (dir, path, digest) = harness_copy(variant);
    let child = std::process::Command::new(&path)
        .args(SLEEP_CHILD)
        .env("PROMOTION_SLEEP", "1")
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn a harness copy");
    let pid = child.id();
    (Proc { child, dir }, pid, digest)
}

/// `sh -c '<copy> & ... wait'`: Compose's pid is the shell, and the Harness
/// processes are its children. Returns the shell pid and theirs.
fn spawn_wrapped(variant: &str, copies: usize) -> (Proc, u32, Vec<u32>, String) {
    use std::os::unix::process::CommandExt;
    let (dir, path, digest) = harness_copy(variant);
    let script = format!(
        "{}wait",
        format!("'{}' {} & ", path.display(), SLEEP_CHILD.join(" ")).repeat(copies)
    );
    let child = std::process::Command::new("sh")
        .args(["-c", &script])
        .env("PROMOTION_SLEEP", "1")
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn the wrapper");
    let pid = child.id();
    let proc = Proc { child, dir };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut harness = harness_children(pid);
    while harness.len() < copies && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        harness = harness_children(pid);
    }
    assert_eq!(
        harness.len(),
        copies,
        "the wrapper started its harness copies"
    );
    (proc, pid, harness, digest)
}

async fn compose_rig(
    harness: Value,
) -> (
    Arc<Mutex<Daemon>>,
    fake_engine::FakeEngine,
    Promotion,
    String,
) {
    let compose_file = std::env::temp_dir().join(format!(
        "promotion-rig-{}-{}.yaml",
        std::process::id(),
        sha256(harness.to_string().as_bytes())
    ));
    std::fs::write(&compose_file, "containers: {}\n").unwrap();
    let compose_file = compose_file.to_string_lossy().to_string();
    let daemon = Arc::new(Mutex::new(Daemon {
        namespace: "promotion-target".into(),
        file: compose_file.clone(),
        harness,
        ..Daemon::default()
    }));
    let shared = daemon.clone();
    let fake = fake_engine::start(move |frame: Value| {
        if frame["type"] != "invokefunction" || frame["invocation_id"].is_null() {
            return vec![];
        }
        let function = frame["function_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let mut reply = json!({ "type": "invocationresult", "invocation_id": frame["invocation_id"],
                                "function_id": function });
        match shared.lock().unwrap().respond(&function, &frame["data"]) {
            Ok(result) => reply["result"] = result,
            Err(message) => {
                reply["error"] = json!({ "code": "invocation_failed", "message": message })
            }
        }
        vec![reply]
    })
    .await;
    let iii = Arc::new(register_worker(&fake.url, InitOptions::default()));
    tokio::time::timeout(Duration::from_secs(10), async {
        while iii.get_connection_state() != iii_sdk::runtime::IIIConnectionState::Connected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("connects to the fake engine");
    let promotion = Promotion::new(
        config(local_target(&compose_file)),
        origins("http://127.0.0.1:9"),
        iii,
    )
    .unwrap();
    (daemon, fake, promotion, compose_file)
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn the_harness_under_a_shell_wrapper_is_hashed_never_the_wrapper() {
    let (_wrapper, shell_pid, harness, digest) = spawn_wrapped("wrapped", 1);
    let shell_digest = sha256(&std::fs::read(format!("/proc/{shell_pid}/exe")).unwrap());
    assert_ne!(
        shell_digest, digest,
        "the shell and the harness copy differ"
    );
    let (daemon, _fake, promotion, compose_file) = compose_rig(row(shell_pid, "0.9.1")).await;

    let observed = promotion.observe_target().await.unwrap();
    assert_eq!(observed.supervisor_pid, Some(shell_pid));
    assert_eq!(observed.pid, Some(harness[0]), "the actual Harness pid");
    assert_eq!(
        observed.binary_sha256.as_deref(),
        Some(digest.as_str()),
        "the Harness bytes, not the shell's"
    );

    let candidate = ValidatedArtifact {
        version: "0.9.1".into(),
        archive_sha256: sha256(b"archive"),
        binary_sha256: digest.clone(),
    };
    let operation = compose_operation_id("p1", "activate", "0.9.1", 1);
    daemon
        .lock()
        .unwrap()
        .operations
        .insert(operation.clone(), json!({ "status": "succeeded" }));
    let Confirmation::Confirmed(running) = promotion
        .confirm_running(&candidate, &operation)
        .await
        .unwrap()
    else {
        panic!("the wrapped harness is proved");
    };
    assert_eq!(
        (running.supervisor_pid, running.pid),
        (shell_pid, harness[0])
    );

    // Two Harness processes under one wrapper: which one serves is unknown.
    let (_twice, twice_pid, _, _) = spawn_wrapped("twice", 2);
    daemon.lock().unwrap().harness = row(twice_pid, "0.9.1");
    let ambiguous = promotion.observe_target().await.unwrap();
    assert_eq!(ambiguous.binary_sha256, None);
    assert!(
        ambiguous
            .binary_unknown_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("2 harness processes")),
        "{ambiguous:?}"
    );
    assert!(matches!(
        promotion
            .confirm_running(&candidate, &operation)
            .await
            .unwrap(),
        Confirmation::Pending { .. }
    ));

    // A supervisor with no Harness under it is never hashed as Harness.
    let mut bare = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    daemon.lock().unwrap().harness = row(bare.id(), "0.9.1");
    let missing = promotion.observe_target().await.unwrap();
    let _ = bare.kill();
    let _ = bare.wait();
    assert_eq!(missing.binary_sha256, None);
    assert_eq!(missing.pid, None);

    // A live process named harness outside the supervisor's tree is never
    // taken for it, however plausible its name.
    let (_stranger, stranger_pid, _) = spawn("stranger");
    let mut shell = std::process::Command::new("sh")
        .args(["-c", "sleep 60; true"])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    daemon.lock().unwrap().harness = row(shell.id(), "0.9.1");
    let unrelated = promotion.observe_target().await.unwrap();
    let _ = shell.kill();
    let _ = shell.wait();
    assert_eq!(unrelated.binary_sha256, None, "{unrelated:?}");
    assert_ne!(unrelated.pid, Some(stranger_pid));
    let _ = std::fs::remove_file(&compose_file);
}

fn row(pid: u32, version: &str) -> Value {
    json!({ "container": "harness", "state": "ready", "version": version, "pid": pid,
            "owned": true, "log_path": "/tmp/harness.log" })
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn activation_and_rollback_are_proved_by_the_running_executable() {
    let (_prior_proc, prior_pid, prior_digest) = spawn("prior");
    let (_candidate_proc, candidate_pid, candidate_digest) = spawn("candidate");
    let (_other_proc, other_pid, other_digest) = spawn("other");
    assert!(prior_digest != candidate_digest && other_digest != prior_digest);
    let compose_file = std::env::temp_dir().join(format!("promotion-{}.yaml", std::process::id()));
    std::fs::write(&compose_file, "containers: {}\n").unwrap();
    let compose_file = compose_file.to_string_lossy().to_string();

    let daemon = Arc::new(Mutex::new(Daemon {
        namespace: "promotion-target".into(),
        file: compose_file.clone(),
        harness: row(prior_pid, "0.9.0"),
        ..Daemon::default()
    }));
    let shared = daemon.clone();
    let fake = fake_engine::start(move |frame: Value| {
        if frame["type"] != "invokefunction" || frame["invocation_id"].is_null() {
            return vec![];
        }
        let function = frame["function_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let mut reply = json!({ "type": "invocationresult", "invocation_id": frame["invocation_id"],
                                "function_id": function });
        match shared.lock().unwrap().respond(&function, &frame["data"]) {
            Ok(result) => reply["result"] = result,
            Err(message) => {
                reply["error"] = json!({ "code": "invocation_failed", "message": message })
            }
        }
        vec![reply]
    })
    .await;
    let iii = Arc::new(register_worker(&fake.url, InitOptions::default()));
    tokio::time::timeout(Duration::from_secs(10), async {
        while iii.get_connection_state() != iii_sdk::runtime::IIIConnectionState::Connected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("connects to the fake engine");
    let promotion = Promotion::new(
        config(local_target(&compose_file)),
        origins("http://127.0.0.1:9"),
        iii,
    )
    .unwrap();

    let prior = ValidatedArtifact {
        version: "0.9.0".into(),
        archive_sha256: sha256(b"prior archive"),
        binary_sha256: prior_digest.clone(),
    };
    let candidate = ValidatedArtifact {
        version: "0.9.1".into(),
        archive_sha256: sha256(b"candidate archive"),
        binary_sha256: candidate_digest.clone(),
    };

    // The target runs the validated prior: that is the rollback destination.
    let observed = promotion.observe_target().await.unwrap();
    assert_eq!(
        classify_target(&observed, &prior, &candidate),
        TargetIs::Prior
    );

    let activate = compose_operation_id("p1", "activate", "0.9.1", 1);
    // Recovery reads the operation without dispatching it.
    assert_eq!(
        promotion.read_compose_operation(&activate).await.unwrap(),
        ComposeOperationState::Unknown
    );
    assert!(
        daemon.lock().unwrap().adds().is_empty(),
        "a read never pins"
    );
    let requested = promotion
        .request_activation(&prior, &candidate, &activate)
        .await
        .unwrap();
    assert!(requested.accepted);
    let adds = daemon.lock().unwrap().adds();
    assert_eq!(adds.len(), 1);
    assert_eq!(
        adds[0]["workers"],
        json!([{ "worker": "package://harness", "version": "0.9.1" }])
    );
    assert_eq!(adds[0]["operation_id"], activate);
    assert_eq!(adds[0]["file"], compose_file);

    assert_eq!(
        promotion.read_compose_operation(&activate).await.unwrap(),
        ComposeOperationState::Running
    );
    // A restart before confirmation reconciles the running operation
    // instead of sending a second pin.
    promotion
        .request_activation(&prior, &candidate, &activate)
        .await
        .unwrap();
    assert_eq!(daemon.lock().unwrap().adds().len(), 1);

    // Requested is not activated.
    assert!(matches!(
        promotion
            .confirm_running(&candidate, &activate)
            .await
            .unwrap(),
        Confirmation::Pending { .. }
    ));
    daemon
        .lock()
        .unwrap()
        .operations
        .insert(activate.clone(), json!({ "status": "succeeded" }));
    assert!(
        matches!(
            promotion
                .confirm_running(&candidate, &activate)
                .await
                .unwrap(),
            Confirmation::Pending { .. }
        ),
        "the old process still runs the prior version"
    );
    daemon.lock().unwrap().harness = row(prior_pid, "0.9.1");
    assert_eq!(
        promotion
            .confirm_running(&candidate, &activate)
            .await
            .unwrap_err()
            .kind,
        StopKind::Rejected,
        "the right version label over the wrong executable is not activation"
    );
    daemon.lock().unwrap().harness = row(candidate_pid, "0.9.1");
    let Confirmation::Confirmed(running) = promotion
        .confirm_running(&candidate, &activate)
        .await
        .unwrap()
    else {
        panic!("activation is proved");
    };
    assert_eq!(running.binary_sha256, candidate_digest);
    assert_eq!(running.pid, candidate_pid);

    // Rollback pins the exact prior and is confirmed only by its executable.
    let rollback = compose_operation_id("p1", "rollback", "0.9.0", 1);
    assert!(
        promotion
            .request_rollback(&prior, &candidate, &rollback)
            .await
            .unwrap()
            .accepted
    );
    assert_eq!(
        daemon.lock().unwrap().adds()[1]["workers"],
        json!([{ "worker": "package://harness", "version": "0.9.0" }])
    );
    daemon
        .lock()
        .unwrap()
        .operations
        .insert(rollback.clone(), json!({ "status": "succeeded" }));
    assert!(
        matches!(
            promotion.confirm_running(&prior, &rollback).await.unwrap(),
            Confirmation::Pending { .. }
        ),
        "restoration is not assumed"
    );
    daemon.lock().unwrap().harness = row(prior_pid, "0.9.0");
    assert!(matches!(
        promotion.confirm_running(&prior, &rollback).await.unwrap(),
        Confirmation::Confirmed(_)
    ));

    // An operation cancelled before its add is read as cancelled and never
    // dispatched by the read.
    let cancelled = compose_operation_id("p1", "activate", "0.9.1", 9);
    daemon
        .lock()
        .unwrap()
        .operations
        .insert(cancelled.clone(), json!({ "status": "cancelled" }));
    let adds_before = daemon.lock().unwrap().adds().len();
    assert_eq!(
        promotion.read_compose_operation(&cancelled).await.unwrap(),
        ComposeOperationState::Cancelled
    );
    assert_eq!(daemon.lock().unwrap().adds().len(), adds_before);

    // An operator's own change is never overwritten.
    daemon.lock().unwrap().harness = row(other_pid, "0.9.0-local");
    let fresh = compose_operation_id("p1", "activate", "0.9.1", 2);
    assert_eq!(
        promotion
            .request_activation(&prior, &candidate, &fresh)
            .await
            .unwrap_err()
            .kind,
        StopKind::Rejected
    );
    assert_eq!(daemon.lock().unwrap().adds().len(), 2);

    // A daemon for another namespace is not the target.
    daemon.lock().unwrap().namespace = "my-project".into();
    assert_eq!(
        promotion.observe_target().await.unwrap_err().kind,
        StopKind::Rejected
    );
    let _ = std::fs::remove_file(&compose_file);
}

#[test]
fn a_remote_target_is_never_treated_as_observable() {
    let mut target = local_target("/tmp/worker-compose.yaml");
    assert_eq!(target.is_local_linux(), cfg!(target_os = "linux"));
    target.engine_url = "ws://10.0.0.5:49134".into();
    assert!(!target.is_local_linux());
    target.engine_url = "wss://127.0.0.1:49134".into();
    assert!(!target.is_local_linux());
}
