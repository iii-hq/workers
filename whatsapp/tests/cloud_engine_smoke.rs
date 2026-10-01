//! Isolated loopback integration. Requires III_BINARY and HTTP_BINARY; optionally
//! WHATSAPP_BINARY overrides Cargo's worker. No native connection or Meta call.
#![cfg(unix)]

use hmac::{Hmac, KeyInit, Mac};
use reqwest::Client;
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    fs::{self, File},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const NAMESPACE: &str = "whatsapp-cloud-smoke";
const CONFIG_ID: &str = "whatsapp-cloud-smoke";
const APP_SECRET: &str = "isolated-test-app-secret";
const VERIFY_TOKEN: &str = "isolated-test-verify-token";
const ACCESS_TOKEN: &str = "isolated-test-access-token";

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(command: &mut Command, directory: &Path, name: &str) -> Process {
    let log = File::create(directory.join(format!("{name}.log"))).unwrap();
    Process(
        command
            .current_dir(directory)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap_or_else(|error| panic!("unable to start {name}: {error}")),
    )
}

fn trigger(engine: &Path, port: u16, function: &str, payload: Value) -> Result<Value, String> {
    let namespace = if function.starts_with("whatsapp::") || function.starts_with("http::") {
        NAMESPACE
    } else {
        "default"
    };
    let output = Command::new(engine)
        .args([
            "trigger",
            "--engine",
            &format!("ws://127.0.0.1:{port}"),
            "--namespace",
            namespace,
            function,
            "--json",
            &payload.to_string(),
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    if output.stdout.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
}

fn wait_for(label: &str, directory: &Path, mut probe: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !probe() {
        if Instant::now() >= deadline {
            for name in ["engine", "http", "whatsapp"] {
                if let Ok(log) = fs::read_to_string(directory.join(format!("{name}.log"))) {
                    eprintln!("{name}: {log}");
                }
            }
            panic!("{label} did not become ready");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn signature(raw: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(APP_SECRET.as_bytes()).unwrap();
    mac.update(raw);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn delivery(account: &str, phone: &str, message_id: &str) -> Value {
    json!({
        "object": "whatsapp_business_account",
        "entry": [{"id": account, "changes": [{"field": "messages", "value": {
            "messaging_product": "whatsapp",
            "metadata": {"phone_number_id": phone},
            "contacts": [{"wa_id": "15551234567", "profile": {"name": "Local test sender"}}],
            "messages": [{"id": message_id, "from": "15551234567", "timestamp": "1700000000",
                "type": "text", "text": {"body": "Loopback test: café"}}]
        }}]}]
    })
}

async fn post(client: &Client, url: &str, raw: &[u8], signed: &str) -> u16 {
    client
        .post(url)
        .header("content-type", "application/json")
        .header("x-hub-signature-256", signed)
        .body(raw.to_vec())
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
#[ignore = "requires III_BINARY and HTTP_BINARY; starts isolated loopback processes, never contacts Meta"]
async fn real_http_webhook_verifies_authenticates_and_persists_without_native_sessions() {
    let engine =
        PathBuf::from(std::env::var("III_BINARY").expect("set III_BINARY to the native engine"));
    let http = PathBuf::from(
        std::env::var("HTTP_BINARY")
            .expect("set HTTP_BINARY to the standalone HTTP worker with webhook_listener support"),
    );
    let worker =
        std::env::var("WHATSAPP_BINARY").unwrap_or_else(|_| env!("CARGO_BIN_EXE_whatsapp").into());
    let temp = TempDir::new().unwrap();
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let engine_url = format!("ws://127.0.0.1:{port}");
    let engine_config = temp.path().join("engine.yaml");
    fs::write(&engine_config, format!("workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: ./configuration\n")).unwrap();
    let _engine = spawn(
        Command::new(&engine)
            .arg("--config")
            .arg(&engine_config)
            .arg("--no-update-check")
            .env("III_NAMESPACE", "default"),
        temp.path(),
        "engine",
    );
    wait_for("engine", temp.path(), || {
        trigger(&engine, port, "engine::workers::list", json!({})).is_ok()
    });

    // Both listeners use OS-assigned ports and loopback. The test exercises the
    // restricted listener, never the admin/general listener or a live service.
    let http_config = temp.path().join("http.yaml");
    fs::write(
        &http_config,
        "host: 127.0.0.1\nport: 0\nwebhook_listener:\n  host: 127.0.0.1\n  port: 0\n",
    )
    .unwrap();
    let _http = spawn(
        Command::new(&http)
            .args(["--url", &engine_url, "--config"])
            .arg(&http_config)
            .env("III_NAMESPACE", NAMESPACE)
            .env("III_CONFIG_NAME", "whatsapp-cloud-smoke-http")
            .env("III_DISABLE_TRACE_PAYLOADS", "1"),
        temp.path(),
        "http",
    );
    let mut webhook_port = 0;
    wait_for("restricted HTTP listener", temp.path(), || {
        let Ok(status) = trigger(&engine, port, "http::webhook-listener::status", json!({})) else {
            return false;
        };
        assert_eq!(status["applied"]["host"], "127.0.0.1");
        webhook_port = status["applied"]["port"].as_u64().unwrap() as u16;
        webhook_port > 0
    });
    let worker_config = temp.path().join("whatsapp.yaml");
    let configuration = json!({
        "data_dir": temp.path().join("whatsapp-data"),
        "cloud": {"enabled": true, "business_account_id": "123", "phone_number_id": "456",
            "access_token": ACCESS_TOKEN, "app_secret": APP_SECRET, "verify_token": VERIFY_TOKEN}
    });
    fs::write(
        &worker_config,
        serde_yaml::to_string(&configuration).unwrap(),
    )
    .unwrap();
    let _worker = spawn(
        Command::new(worker)
            .args(["--url", &engine_url, "--config"])
            .arg(&worker_config)
            .env("III_NAMESPACE", NAMESPACE)
            .env("III_CONFIG_NAME", CONFIG_ID)
            .env_remove("III_WHATSAPP_UI_WATCH")
            // Fail closed on the local machine if a future startup unexpectedly
            // attempts Graph HTTP. This test invokes only local status/archive APIs.
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("https_proxy", "http://127.0.0.1:9")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost"),
        temp.path(),
        "whatsapp",
    );
    wait_for("WhatsApp central configuration", temp.path(), || {
        trigger(&engine, port, "whatsapp::config-status", json!({})).is_ok_and(|status| {
            status["id"] == CONFIG_ID
                && status["source"] == "central"
                && status["last_outcome"] == "applied"
                && status["active"]["cloud"]["enabled"] == true
        })
    });
    let status = trigger(&engine, port, "whatsapp::cloud::status", json!({})).unwrap();
    assert_eq!(status["configured"], true);
    assert_eq!(status["webhook_ready"], true);
    assert_eq!(status["missing_fields"], json!([]));
    assert_eq!(
        status["webhook_path"],
        format!("/webhooks/whatsapp-cloud/{CONFIG_ID}")
    );
    for function in ["whatsapp::cloud::status", "whatsapp::config-status"] {
        let encoded = trigger(&engine, port, function, json!({}))
            .unwrap()
            .to_string();
        for secret in [APP_SECRET, VERIFY_TOKEN, ACCESS_TOKEN] {
            assert!(!encoded.contains(secret), "{function} exposed a credential");
        }
    }

    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(12))
        .build()
        .unwrap();
    let url = format!(
        "http://127.0.0.1:{webhook_port}{}",
        status["webhook_path"].as_str().unwrap()
    );
    let query = [
        ("hub.mode", "subscribe"),
        ("hub.verify_token", VERIFY_TOKEN),
        ("hub.challenge", "00123456"),
    ];
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let response = client.get(&url).query(&query).send().await.unwrap();
        if response.status().as_u16() == 200 {
            assert!(
                response.headers()["content-type"]
                    .to_str()
                    .unwrap()
                    .starts_with("text/plain")
            );
            assert_eq!(response.text().await.unwrap(), "00123456");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "webhook route was not registered"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        client
            .get(&url)
            .query(&[
                ("hub.mode", "subscribe"),
                ("hub.verify_token", "wrong"),
                ("hub.challenge", "1")
            ])
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );

    // Pretty JSON, CRLF, UTF-8 and trailing whitespace prove authentication
    // uses original channel bytes rather than the HTTP worker's JSON projection.
    let raw = format!(
        " {} \r\n",
        serde_json::to_string_pretty(&delivery("123", "456", "wamid.local-one")).unwrap()
    )
    .into_bytes();
    let signed = signature(&raw);
    assert_eq!(post(&client, &url, &raw, &signed).await, 200);
    assert_eq!(post(&client, &url, &raw, &signed).await, 200);
    let compact = serde_json::to_vec(&serde_json::from_slice::<Value>(&raw).unwrap()).unwrap();
    assert_eq!(post(&client, &url, &compact, &signed).await, 403);
    let invalid = serde_json::to_vec(&delivery("123", "456", "invalid-signature")).unwrap();
    assert_eq!(
        post(
            &client,
            &url,
            &invalid,
            "sha256=0000000000000000000000000000000000000000000000000000000000000000"
        )
        .await,
        403
    );
    for (account, phone, id) in [
        ("999", "456", "wrong-account"),
        ("123", "999", "wrong-phone"),
    ] {
        let raw = serde_json::to_vec(&delivery(account, phone, id)).unwrap();
        assert_eq!(post(&client, &url, &raw, &signature(&raw)).await, 200);
    }
    let page = trigger(
        &engine,
        port,
        "whatsapp::cloud::messages::list",
        json!({"chat_id":"+15551234567"}),
    )
    .unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page["messages"][0]["message_id"], "wamid.local-one");
    assert_eq!(page["messages"][0]["text"], "Loopback test: café");
    assert_eq!(page["messages"][0]["sender_name"], "Local test sender");
    assert_eq!(page["messages"][0]["chat_id"], "+15551234567");
    assert_eq!(page["messages"][0]["from_me"], false);
    assert_eq!(
        trigger(&engine, port, "whatsapp::sessions::list", json!({})).unwrap(),
        json!([])
    );
    assert!(
        temp.path()
            .join("whatsapp-data/.cloud/456/inbox.db")
            .is_file()
    );

    // A central config change disables the same registered routes immediately.
    let mut disabled = configuration;
    disabled["cloud"]["enabled"] = false.into();
    trigger(
        &engine,
        port,
        "configuration::set",
        json!({"id":CONFIG_ID, "value":disabled}),
    )
    .unwrap();
    wait_for("disabled Cloud API", temp.path(), || {
        trigger(&engine, port, "whatsapp::cloud::status", json!({}))
            .is_ok_and(|status| status["enabled"] == false)
    });
    assert_eq!(
        client
            .get(&url)
            .query(&query)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        503
    );
    assert_eq!(post(&client, &url, &raw, &signed).await, 503);
}
