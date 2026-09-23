//! Fully local integration: real HTTP listener -> SDK binary ChannelReader ->
//! GitHub handlers -> SQLite inbox/outbox -> deterministic queue -> callback.
//! No external engine, GitHub, cloudflared, credentials, or skipped tests.
//! Run: CARGO_TARGET_DIR=target-integration-82238077 cargo test --offline --test webhooks_integration
#[path = "support/webhook_engine.rs"]
mod engine;
#[path = "support/webhook_mocks.rs"]
mod mocks;

use engine::{wait, Engine};
use futures::FutureExt;
use github::{config::Config, configuration::ConfigCell, webhooks::WebhookConfig};
use hmac::{Hmac, Mac};
use iii_http::{
    boot::{self, BootHandle},
    config::{RestApiConfig, WebhookListenerConfig},
};
use iii_sdk::{protocol::RegisterTriggerInput, Error, IIIClient, RegisterFunction};
use mocks::{call, Queue, Tunnel, CONSUMER, PROVIDER};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::RwLock;

const CALLBACK: &str = "integration::pr-event";
const COMMENT: &str = "Ação: café ☃ — comentário\n  Preserve whitespace.";
fn metadata() -> Value {
    json!({"__binding":"integration-82238077", "tenant":"ação", "payload":{"ticket":17}})
}
fn pr(number: u64) -> Value {
    json!({"number":number,"head":{"sha":format!("head-{number}")},"state":"open","merged":false,
        "updated_at":"2026-01-01T00:00:00Z", "html_url":format!("https://github.com/owner/repo/pull/{number}")})
}
fn comment(id: u64) -> Value {
    json!({"repository":{"full_name":"owner/repo"},"action":"created",
        "issue":{"number":1,"pull_request":{"url":"https://api.github.com/repos/owner/repo/pulls/1"}},
        "comment":{"id":id,"body":COMMENT,"user":{"login":"reviewer"},
            "html_url":format!("https://github.com/owner/repo/pull/1#issuecomment-{id}"),"updated_at":"2026-01-01T00:01:00Z"}})
}
fn raw_json(value: &Value) -> Vec<u8> {
    // Deliberately different from a compact JSON reserialization, including UTF-8.
    format!(" \r\n{}  \n", serde_json::to_string_pretty(value).unwrap()).into_bytes()
}
fn signature(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

type CallbackLog = Arc<Mutex<Vec<(Value, Option<Value>)>>>;

struct Fixture {
    engine: Engine,
    iii: Arc<IIIClient>,
    boot: BootHandle,
    queue: Queue,
    tunnel: Tunnel,
    callbacks: CallbackLog,
    callback_fail: Arc<AtomicBool>,
    http: reqwest::Client,
    gh_path: PathBuf,
    db_path: PathBuf,
    _dir: tempfile::TempDir,
}
impl Fixture {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let gh_path = dir.path().join("gh-state.json");
        std::fs::write(
            &gh_path,
            json!({"prs":{"1":pr(1),"2":pr(2)},"hook":null,"calls":[]}).to_string(),
        )
        .unwrap();
        let db_path = dir.path().join("private/store.sqlite3");
        let mut engine = Engine::start().await;
        let iii = engine.client(PROVIDER).await;
        let consumer = engine.client(CONSUMER).await;
        let queue = Queue::default();
        let tunnel = Tunnel::default();
        queue.register(&iii);
        tunnel.register(&iii);
        let boot = boot::start(
            iii.clone(),
            RestApiConfig {
                host: "127.0.0.1".into(),
                port: 0,
                webhook_listener: Some(WebhookListenerConfig {
                    host: "127.0.0.1".into(),
                    port: 0,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(Config {
            token: gh_path.to_string_lossy().into_owned(),
            gh_executable: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/integration-gh.py")
                .to_string_lossy()
                .into_owned(),
            webhooks: WebhookConfig {
                enabled: true,
                storage_path: db_path.to_string_lossy().into_owned(),
                ..Default::default()
            },
            ..Default::default()
        })));
        github::webhooks::register(&iii, &cell, &engine.url).await;
        for (kind, function) in [
            ("http", "github::webhooks::receive"),
            ("durable:subscriber", "github::webhooks::process"),
            ("quick-tunnel::changed", "github::webhooks::tunnel-changed"),
            ("cron", "github::webhooks::maintain"),
        ] {
            engine.registered(kind, function).await;
        }
        let callbacks = Arc::new(Mutex::new(Vec::new()));
        let callback_fail = Arc::new(AtomicBool::new(false));
        let received = callbacks.clone();
        let fail = callback_fail.clone();
        consumer.register_function(
            CALLBACK,
            RegisterFunction::new(move |payload: Value, meta: Option<Value>| {
                if fail.load(Ordering::SeqCst) {
                    return Err(Error::Handler("injected callback failure".into()));
                }
                received.lock().unwrap().push((payload, meta));
                Ok(json!({"captured":true}))
            })
            .description("Capture actual PR notifications")
            .request_format(json!({"type":"object"}))
            .response_format(json!({"type":"object"})),
        );
        engine.function(CONSUMER, CALLBACK).await;
        let mut binding =
            RegisterTriggerInput::new("github::pr::event", CALLBACK, json!({"repo":"owner/repo"}));
        binding.namespace = Some(CONSUMER.into());
        binding.trigger_namespace = Some(PROVIDER.into());
        binding.metadata = Some(metadata());
        consumer.register_trigger(binding).unwrap();
        // Binding ACK precedes BOTH watch calls, not just local SDK submission.
        engine.registered("github::pr::event", CALLBACK).await;
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        Self {
            engine,
            iii,
            boot,
            queue,
            tunnel,
            callbacks,
            callback_fail,
            http,
            gh_path,
            db_path,
            _dir: dir,
        }
    }
    fn gh(&self) -> Value {
        serde_json::from_slice(&std::fs::read(&self.gh_path).unwrap()).unwrap()
    }
    fn database(&self) -> Value {
        let data = github::webhooks::store::Store::inspect(&self.db_path).unwrap();
        serde_json::to_value(data).unwrap()
    }
    fn calls(&self, method: &str) -> Vec<Value> {
        self.gh()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["method"] == method)
            .cloned()
            .collect()
    }
    fn events(&self) -> Vec<Value> {
        self.callbacks
            .lock()
            .unwrap()
            .iter()
            .map(|(event, meta)| {
                assert_eq!(meta, &Some(metadata()));
                event.clone()
            })
            .collect()
    }
    async fn status(&self, number: u64) -> Value {
        call(
            &self.iii,
            "github::pr::watch-status",
            json!({"watch_id":format!("watch-{number}")}),
        )
        .await
    }
    async fn flush(&self) {
        call(&self.iii, "github::webhooks::maintain", json!({})).await;
        self.queue.drain(&self.iii).await.unwrap();
    }
    async fn two_watches_ready(&self) {
        let expires = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        for number in [1, 2] {
            let value = call(
                &self.iii,
                "github::pr::watch",
                json!({"watch_id":format!("watch-{number}"),
                "repo":"owner/repo","number":number,"expires_at":expires}),
            )
            .await;
            assert_eq!(value["snapshot"]["head_sha"], format!("head-{number}"));
            assert_eq!(value["health"]["tunnel_status"], "starting");
            assert_eq!(value["health"]["hook_ready"], false);
        }
        assert_eq!(self.tunnel.lease_count(), 2);
        assert!(
            self.calls("POST").is_empty(),
            "starting tunnel cannot create a hook"
        );
        self.queue.drain(&self.iii).await.unwrap();
        self.tunnel
            .ready(&self.iii, "https://one.trycloudflare.com", "generation-one")
            .await;
        self.flush().await;
        for number in [1, 2] {
            let value = self.status(number).await;
            assert_eq!(value["status"], "active", "{value}");
            assert_eq!(value["health"]["hook_ready"], true, "{value}");
        }
        let posts = self.calls("POST");
        assert_eq!(
            posts.len(),
            1,
            "two PRs must share exactly one hook: {posts:?}"
        );
        assert_eq!(posts[0]["endpoint"], "repos/owner/repo/hooks");
        let config = &posts[0]["body"]["config"];
        assert!(config["url"]
            .as_str()
            .unwrap()
            .starts_with("https://one.trycloudflare.com/webhooks/github/"));
        assert_eq!(config["content_type"], "json");
        assert_eq!(config["insecure_ssl"], "0");
        assert!(config["secret"].as_str().unwrap().len() >= 32);
        assert_eq!(self.gh()["hook"]["config"], *config);
        assert!(self.events().iter().any(|e| e["watch_id"] == "watch-1"));
        assert!(self.events().iter().any(|e| e["watch_id"] == "watch-2"));
        self.callbacks.lock().unwrap().clear();
        assert!(self.database()["jobs"].as_object().unwrap().is_empty());
    }
    async fn post(&self, event: &str, delivery: &str, raw: &[u8], signature_raw: &[u8]) -> u16 {
        let state = self.gh();
        let config = &state["hook"]["config"];
        let hook_url = reqwest::Url::parse(config["url"].as_str().unwrap()).unwrap();
        let addr = self.boot.current_webhook_addr().await.unwrap();
        let response = self
            .http
            .post(format!("http://{addr}{}", hook_url.path()))
            .header("content-type", "application/json")
            .header(
                "x-hub-signature-256",
                signature(config["secret"].as_str().unwrap(), signature_raw),
            )
            .header("x-github-hook-id", "42")
            .header("x-github-event", event)
            .header("x-github-delivery", delivery)
            .body(raw.to_vec())
            .send()
            .await
            .unwrap();
        let code = response.status().as_u16();
        let _ = response.bytes().await.unwrap();
        code
    }
    async fn deliver(&self, event: &str, delivery: &str, value: &Value) {
        let raw = raw_json(value);
        assert_eq!(self.post(event, delivery, &raw, &raw).await, 202);
        wait(|| self.queue.len() > 0).await;
        self.queue.drain(&self.iii).await.unwrap();
    }
    fn merge(&self, number: u64) -> Value {
        // Queue dispatch is paused; no gh child can concurrently mutate this file.
        let mut state = self.gh();
        let pr = &mut state["prs"][number.to_string()];
        pr["merged"] = json!(true);
        pr["state"] = json!("closed");
        pr["updated_at"] = json!("2026-01-01T01:00:00Z");
        let body =
            json!({"repository":{"full_name":"owner/repo"},"action":"closed","pull_request":pr});
        std::fs::write(&self.gh_path, state.to_string()).unwrap();
        body
    }
    async fn shutdown(self) {
        let normal = self.boot.local_addr;
        let public = self.boot.current_webhook_addr().await.unwrap();
        self.boot.shutdown().await;
        self.engine.shutdown().await;
        // Joining is not enough: prove that both listening sockets were released.
        let _normal = tokio::net::TcpListener::bind(normal).await.unwrap();
        let _public = tokio::net::TcpListener::bind(public).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovery_follows_delivery_cursors_and_stops_after_five_pages() {
    let fixture = Fixture::start().await;
    let outcome = std::panic::AssertUnwindSafe(async {
        fixture.two_watches_ready().await;
        let mut state = fixture.gh();
        state["delivery_pages"] = json!([
            [{"id": 11, "status_code": 502}, {"id": 12, "status_code": 202}],
            [{"id": 13, "status_code": 500}]
        ]);
        std::fs::write(&fixture.gh_path, state.to_string()).unwrap();
        call(
            &fixture.iii,
            "github::pr::recover",
            json!({"repo":"owner/repo"}),
        )
        .await;
        let calls = fixture.calls("GET");
        assert!(calls.iter().any(|call| {
            call["endpoint"]
                == "repos/owner/repo/hooks/42/deliveries?per_page=100&cursor=opaque-1%3D"
        }));
        let posts = fixture.calls("POST");
        for id in [11, 13] {
            assert!(posts.iter().any(|call| {
                call["endpoint"] == format!("repos/owner/repo/hooks/42/deliveries/{id}/attempts")
            }));
        }
        assert!(!posts.iter().any(|call| {
            call["endpoint"] == "repos/owner/repo/hooks/42/deliveries/12/attempts"
        }));
        let mut state = fixture.gh();
        state["delivery_pages"] = json!([[], [], [], [], [], []]);
        state["calls"] = json!([]);
        std::fs::write(&fixture.gh_path, state.to_string()).unwrap();
        call(
            &fixture.iii,
            "github::pr::recover",
            json!({"repo":"owner/repo"}),
        )
        .await;
        let count = fixture
            .calls("GET")
            .iter()
            .filter(|call| call["endpoint"].as_str().unwrap().contains("/deliveries?"))
            .count();
        assert_eq!(count, 5);
        async fn assert_ready(fixture: &Fixture, number: u64) {
            let status = fixture.status(number).await;
            assert_eq!(status["status"], "active", "{status}");
            assert_eq!(status["health"]["hook_ready"], true, "{status}");
            assert!(status["health"]["last_error"]
                .as_str()
                .unwrap()
                .contains("five pages"));
            assert!(fixture.database()["repos"]["owner/repo"]["error"].is_null());
        }
        for number in [1, 2] {
            assert_ready(&fixture, number).await;
        }
        // Recovery of healthy hooks must not enter a perpetual PATCH loop.
        call(
            &fixture.iii,
            "github::pr::recover",
            json!({"repo":"owner/repo"}),
        )
        .await;
        assert!(fixture.calls("PATCH").is_empty());
        let expires = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        call(
            &fixture.iii,
            "github::pr::watch",
            json!({"watch_id":"watch-3", "repo":"owner/repo", "number":1,
                "expires_at":expires}),
        )
        .await;
        assert_ready(&fixture, 3).await;
        assert!(fixture.calls("PATCH").is_empty());
        // A real URL change still configures the hook once, drains the lifecycle
        // job, and leaves all watches active despite the bounded-history warning.
        fixture
            .tunnel
            .ready(
                &fixture.iii,
                "https://two.trycloudflare.com",
                "generation-two",
            )
            .await;
        fixture.flush().await;
        assert_eq!(fixture.calls("PATCH").len(), 1);
        for number in [1, 2, 3] {
            assert_ready(&fixture, number).await;
        }
        assert!(fixture.database()["jobs"].as_object().unwrap().is_empty());
        assert!(
            fixture.calls("POST").is_empty(),
            "must not create another hook"
        );
    })
    .catch_unwind()
    .await;
    fixture.shutdown().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_raw_webhooks_preserve_events_isolate_routes_and_cleanup_shared_hook() {
    let fixture = Fixture::start().await;
    let outcome = std::panic::AssertUnwindSafe(async {
        fixture.two_watches_ready().await;
        // The private route exists on the normal listener: public 404 proves isolation.
        fixture.iii.register_function("integration::private", RegisterFunction::new(|_: Value| {
            Ok::<_, Error>(json!({"body":"private"}))
        }).description("Private route isolation control").request_format(json!({"type":"object"})).response_format(json!({"type":"object"})));
        fixture.iii.register_trigger(RegisterTriggerInput::new("http", "integration::private",
            json!({"api_path":"/private","http_method":"GET"}))).unwrap();
        fixture.engine.registered("http", "integration::private").await;
        assert_eq!(fixture.http.get(format!("http://{}/private", fixture.boot.local_addr)).send().await.unwrap().status(), 200);
        let public = fixture.boot.current_webhook_addr().await.unwrap();
        for path in ["/private", "/admin", "/console", "/engine", "/github::pr::watch"] {
            assert_eq!(fixture.http.get(format!("http://{public}{path}")).send().await.unwrap().status(), 404, "{path}");
        }
        let body = comment(9001);
        let raw = raw_json(&body);
        assert_ne!(raw, serde_json::to_vec(&body).unwrap());
        assert_eq!(fixture.post("issue_comment", "comment-1", &raw, &raw).await, 202);
        // Queue accepts but does not dispatch until SQLite has been inspected.
        wait(|| fixture.queue.len() > 0).await;
        let database = fixture.database();
        let inbox = &database["jobs"]["inbox:42:comment-1"];
        assert_eq!(inbox["kind"], "inbox");
        assert_eq!(inbox["body"], body);
        assert!(fixture.events().is_empty());
        fixture.queue.drain(&fixture.iii).await.unwrap();
        let events = fixture.events();
        assert_eq!(events.len(), 1, "comment belongs only to PR 1: {events:?}");
        let event = &events[0];
        assert_eq!(event["watch_id"], "watch-1");
        assert_eq!(event["repo"], "owner/repo");
        assert_eq!(event["number"], 1);
        assert_eq!(event["category"], "comments");
        assert_eq!(event["kind"], "issue_comment:created");
        assert_eq!(event["entity"], "9001");
        assert_eq!(event["detail"]["body"], COMMENT);
        assert_eq!(event["detail"]["actor"], "reviewer");
        assert_eq!(event["detail"]["html_url"], body["comment"]["html_url"]);
        assert_eq!(event["snapshot"]["head_sha"], "head-1");
        assert_eq!(event["final_event"], false);
        assert!(fixture.engine.bytes().contains(&raw), "channel bytes differ from signed HTTP input");
        let frame = fixture.engine.frames().into_iter().find(|f| f["type"] == "invokefunction"
            && f["function_id"] == CALLBACK && f["data"]["entity"] == "9001").unwrap();
        assert_eq!(frame["namespace"], CONSUMER);
        assert_eq!(frame["metadata"], metadata());
        let mut delivered = frame["data"].clone();
        delivered["_caller_worker_id"] = json!("test-worker");
        assert_eq!(delivered, *event);
        let published = fixture.queue.published.load(Ordering::SeqCst);
        assert_eq!(fixture.post("issue_comment", "comment-1", &raw, &raw).await, 202);
        fixture.flush().await;
        assert_eq!(fixture.events().len(), 1, "same delivery must not repeat effect");
        assert_eq!(fixture.queue.published.load(Ordering::SeqCst), published);
        let mut tampered = raw.clone();
        tampered.push(b' '); // Still valid JSON, but no longer signed bytes.
        assert_eq!(fixture.post("issue_comment", "tampered", &tampered, &raw).await, 401);
        assert!(!fixture.database()["deliveries"].as_array().unwrap().contains(&json!("42:tampered")));
        fixture.flush().await;
        assert_eq!(fixture.events().len(), 1);
        let ci = json!({"repository":{"full_name":"owner/repo"},"action":"completed", "sender":{"login":"ci-bot"},
            "check_run":{"id":77,"name":"Rust CI café","head_sha":"head-1","status":"completed","conclusion":"success",
                "run_attempt":2,"completed_at":"2026-01-01T00:05:00Z","html_url":"https://github.com/owner/repo/actions/runs/77"}});
        fixture.deliver("check_run", "ci-1", &ci).await;
        let event = fixture.events().into_iter().find(|e| e["category"] == "ci").unwrap();
        assert_eq!(event["number"], 1, "PR 2 has a different authoritative SHA");
        assert_eq!(event["detail"]["name"], "Rust CI café");
        assert_eq!(event["detail"]["conclusion"], "success");
        assert_eq!(event["detail"]["html_url"], ci["check_run"]["html_url"]);
        assert_eq!(event["snapshot"]["ci"]["check_run:77"]["attempt"], 2);
        assert_eq!(event["snapshot"]["ci"]["check_run:77"]["name"], "Rust CI café");
        assert_eq!(event["snapshot"]["ci"]["check_run:77"]["conclusion"], "success");
        assert!(fixture.status(2).await["snapshot"]["ci"].as_object().unwrap().is_empty());
        let old_hook = fixture.gh()["hook"].clone();
        fixture.tunnel.ready(&fixture.iii, "https://two.trycloudflare.com", "generation-two").await;
        fixture.flush().await;
        let new_hook = fixture.gh()["hook"].clone();
        assert_eq!(fixture.calls("POST").len(), 1, "rotation must PATCH, not create/adopt");
        assert_eq!(fixture.calls("PATCH").len(), 1);
        assert_eq!(new_hook["id"], 42);
        assert_eq!(new_hook["config"]["secret"], old_hook["config"]["secret"]);
        assert_eq!(new_hook["config"]["url"].as_str().unwrap(), old_hook["config"]["url"].as_str().unwrap().replace("one.trycloudflare.com", "two.trycloudflare.com"));
        fixture.deliver("pull_request", "merge-1", &fixture.merge(1)).await;
        assert_eq!(fixture.status(1).await["status"], "completed");
        assert_eq!(fixture.status(2).await["status"], "active");
        assert_eq!(fixture.gh()["hook"]["id"], 42);
        assert!(fixture.calls("DELETE").is_empty());
        assert_eq!(fixture.tunnel.lease_count(), 1);
        assert_eq!(fixture.tunnel.releases.lock().unwrap().len(), 1);
        assert!(fixture.events().iter().any(|e| e["number"] == 1 && e["kind"] == "merged" && e["final_event"] == true));
        fixture.deliver("pull_request", "merge-2", &fixture.merge(2)).await;
        assert_eq!(fixture.status(2).await["status"], "completed");
        assert!(fixture.gh()["hook"].is_null());
        assert_eq!(fixture.calls("DELETE").len(), 1);
        assert_eq!(fixture.calls("DELETE")[0]["endpoint"], "repos/owner/repo/hooks/42");
        assert_eq!(fixture.tunnel.lease_count(), 0);
        assert_eq!(fixture.tunnel.releases.lock().unwrap().len(), 2);
        assert!(fixture.database()["jobs"].as_object().unwrap().is_empty());
        assert!(fixture.events().iter().any(|e| e["number"] == 2 && e["kind"] == "merged" && e["final_event"] == true));
    }).catch_unwind().await;
    fixture.shutdown().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_publication_and_callback_keep_durable_work_until_real_handler_ack() {
    let fixture = Fixture::start().await;
    let outcome = std::panic::AssertUnwindSafe(async {
        fixture.two_watches_ready().await;
        fixture.queue.fail.store(true, Ordering::SeqCst);
        let raw = raw_json(&comment(9002));
        assert_eq!(
            fixture
                .post("issue_comment", "queue-failure", &raw, &raw)
                .await,
            202
        );
        wait(|| fixture.queue.failed_publications.load(Ordering::SeqCst) > 0).await;
        assert_eq!(
            fixture.database()["jobs"]["inbox:42:queue-failure"]["body"]["comment"]["body"],
            COMMENT
        );
        assert_eq!(fixture.queue.len(), 0);
        assert!(fixture.events().is_empty());
        wait(|| {
            fixture.database()["last_error"]
                .as_str()
                .is_some_and(|s| s.contains("publication"))
        })
        .await;
        fixture.queue.fail.store(false, Ordering::SeqCst);
        // Public recovery clears backoff; do not mutate production SQLite state.
        call(
            &fixture.iii,
            "github::pr::recover",
            json!({"repo":"owner/repo"}),
        )
        .await;
        fixture.callback_fail.store(true, Ordering::SeqCst);
        let error = fixture.queue.drain(&fixture.iii).await.unwrap_err();
        assert!(
            error.to_string().contains("injected callback failure"),
            "{error}"
        );
        assert!(fixture.events().is_empty());
        let db = fixture.database();
        assert!(
            db["jobs"].get("inbox:42:queue-failure").is_none(),
            "inbox effect commits before callback"
        );
        let notify = db["jobs"]
            .as_object()
            .unwrap()
            .values()
            .find(|j| j["kind"] == "notify" && j["event"]["entity"] == "9002")
            .unwrap();
        let event_id = notify["event"]["event_id"].clone();
        assert_eq!(notify["target"]["metadata"], metadata());
        assert_eq!(notify["target"]["namespace"], CONSUMER);
        assert!(fixture.queue.len() > 0, "failed handler must not be ACKed");
        fixture.callback_fail.store(false, Ordering::SeqCst);
        fixture.queue.drain(&fixture.iii).await.unwrap();
        let events = fixture.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event_id"], event_id);
        assert_eq!(events[0]["detail"]["body"], COMMENT);
        assert!(fixture.database()["jobs"].as_object().unwrap().is_empty());
        for number in [1, 2] {
            call(
                &fixture.iii,
                "github::pr::unwatch",
                json!({"watch_id":format!("watch-{number}")}),
            )
            .await;
        }
        assert_eq!(fixture.tunnel.lease_count(), 0);
        assert!(fixture.gh()["hook"].is_null());
    })
    .catch_unwind()
    .await;
    fixture.shutdown().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
