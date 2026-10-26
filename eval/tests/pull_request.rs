//! The pull request helper against a tiny local HTTP server standing in
//! for GitHub. It proves the checks and wire shapes, not the live service.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use eval::pull_request::*;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const BASE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CANDIDATE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BRANCH: &str = "feat/loop-candidate-c42";
const REF: &str = "GET /repos/iii-hq/workers/git/ref/heads/feat/loop-candidate-c42";
const SEARCH: &str =
    "GET /repos/iii-hq/workers/pulls?head=iii-hq:feat/loop-candidate-c42&state=all&per_page=100";
const CREATE: &str = "POST /repos/iii-hq/workers/pulls";

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

/// The fixed credential variable, set to the same value by every test so
/// parallel tests never race on it.
fn set_credentials() {
    std::env::set_var(GITHUB_TOKEN_ENV, "token-secret");
}

async fn github() -> Http {
    set_credentials();
    Http::start().await
}

async fn open(http: &Http) -> Result<PullRequestV1, Refusal> {
    ensure(
        &http.url,
        BRANCH,
        CANDIDATE,
        "eval: summarise documents",
        "the evidence",
    )
    .await
}

fn pull(number: u64, state: &str, merged: bool) -> Value {
    json!({ "number": number, "state": state, "merged_at": if merged { json!("2026-10-25T00:00:00Z") } else { Value::Null },
            "html_url": format!("https://github.com/iii-hq/workers/pull/{number}"),
            "head": { "sha": CANDIDATE, "ref": BRANCH } })
}

// ── tests ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_pull_request_is_reused_only_for_the_exact_candidate_head() {
    let http = github().await;
    http.json(REF, 200, json!({ "object": { "sha": BASE } }));
    let diverged = open(&http).await.unwrap_err();
    assert!(!diverged.retry, "{diverged:?}");
    assert!(http.seen(SEARCH).is_empty());

    http.json(REF, 200, json!({ "object": { "sha": CANDIDATE } }));
    let mut other = pull(6, "open", false);
    other["head"]["sha"] = json!(BASE);
    http.json(SEARCH, 200, json!([other, pull(7, "open", false)]));
    let found = open(&http).await.unwrap();
    assert_eq!((found.number, found.created), (7, false));
    assert!(http.seen(CREATE).is_empty());
    assert_eq!(
        http.seen(SEARCH)[0]
            .headers
            .get("authorization")
            .map(String::as_str),
        Some("Bearer token-secret"),
        "the controller token comes from the named variable"
    );
}

#[tokio::test]
async fn a_merged_pull_request_of_the_candidate_is_reused_not_reopened() {
    let http = github().await;
    http.json(REF, 200, json!({ "object": { "sha": CANDIDATE } }));
    http.json(SEARCH, 200, json!([pull(7, "closed", true)]));
    assert_eq!(open(&http).await.unwrap().number, 7);
    assert!(http.seen(CREATE).is_empty());
}

#[tokio::test]
async fn a_new_pull_request_is_opened_ready_for_review_with_the_evidence() {
    let http = github().await;
    http.json(REF, 200, json!({ "object": { "sha": CANDIDATE } }));
    // A closed, unmerged one is not the candidate's proposal any more.
    http.json(SEARCH, 200, json!([pull(5, "closed", false)]));
    http.json(CREATE, 201, pull(8, "open", false));
    let opened = open(&http).await.unwrap();
    assert_eq!((opened.number, opened.created), (8, true));
    let sent: Value = serde_json::from_str(&http.seen(CREATE)[0].body).unwrap();
    assert_eq!(
        sent,
        json!({"title": "eval: summarise documents", "head": BRANCH, "base": "main",
            "body": "the evidence", "draft": false})
    );
}

#[tokio::test]
async fn a_lost_or_failed_answer_is_asked_again_and_a_refusal_stops() {
    let http = github().await;
    http.json(REF, 200, json!({ "object": { "sha": CANDIDATE } }));
    http.json(SEARCH, 200, json!([]));
    for (status, retry) in [(0u16, true), (502, true), (401, false), (422, false)] {
        http.json(CREATE, status, json!({}));
        let refused = open(&http).await.unwrap_err();
        assert_eq!(refused.retry, retry, "{status}: {refused:?}");
    }
    http.json(REF, 404, json!({}));
    let missing = open(&http).await.unwrap_err();
    assert!(
        !missing.retry && missing.reason.contains("does not exist"),
        "{missing:?}"
    );
}

#[tokio::test]
async fn remote_error_fields_never_reach_a_reason() {
    let http = github().await;
    http.json(REF, 200, json!({ "object": { "sha": CANDIDATE } }));
    http.json(SEARCH, 200, json!([]));
    for field in ["code", "error", "message"] {
        for status in [400u16, 403, 404, 409, 422, 500] {
            http.json(CREATE, status, json!({ (field): "leaked-secret-value" }));
            let refused = open(&http).await.unwrap_err();
            assert!(
                !refused.reason.contains("leaked-secret"),
                "{field} {status}: {refused:?}"
            );
        }
    }
    http.json(
        REF,
        200,
        json!({ "object": { "sha": "leaked-secret-value" } }),
    );
    let refused = open(&http).await.unwrap_err();
    assert!(!refused.reason.contains("leaked-secret"), "{refused:?}");
}

#[tokio::test]
async fn an_unsafe_branch_or_a_short_sha_is_refused_before_any_call() {
    let http = github().await;
    for (branch, sha) in [
        ("feat/x;rm", CANDIDATE),
        ("feat/x y", CANDIDATE),
        (BRANCH, "bbbbbbb"),
        ("", CANDIDATE),
    ] {
        let refused = ensure(&http.url, branch, sha, "t", "b").await.unwrap_err();
        assert!(!refused.retry, "{branch} {sha}: {refused:?}");
    }
    assert!(http.log.lock().unwrap().is_empty(), "nothing was sent");
}
