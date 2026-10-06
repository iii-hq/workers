//! The real `secrets` binary against a fake engine: boot (configuration
//! entry, catalog, trigger type), the set → list → resolve flow with
//! engine-stamped callers, detect/import, `secrets::changed` delivery, and a
//! clean SIGTERM/SIGINT shutdown. No value may appear anywhere except the one
//! authorized `secrets::resolve` result.
mod support;

use std::collections::BTreeSet;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;

const VALUE: &str = "sk-ant-api03-boot-test-value-9f2c";
const DOTENV_VALUE: &str = "sk-proj-from-dotenv-123456";
const CONSOLE: &str = "11111111-1111-1111-1111-111111111111";
const ROUTER: &str = "22222222-2222-2222-2222-222222222222";
const INTRUDER: &str = "33333333-3333-3333-3333-333333333333";
const FOREIGN_ROUTER: &str = "44444444-4444-4444-4444-444444444444";

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn invocation(step: u32, function_id: &str, mut data: Value, caller: &str) -> Value {
    data["_caller_worker_id"] = json!(caller);
    json!({
        "type": "invokefunction",
        "invocation_id": format!("00000000-0000-0000-0000-{step:012}"),
        "function_id": function_id,
        "data": data,
    })
}

/// The scripted conversation: each step is sent once the previous result
/// arrives.
fn steps() -> Vec<Value> {
    vec![
        invocation(
            1,
            "secrets::set",
            json!({"name":"ANTHROPIC_API_KEY","value":VALUE,"consumers":["llm-router"],"description":"Anthropic"}),
            CONSOLE,
        ),
        invocation(2, "secrets::list", json!({}), CONSOLE),
        invocation(
            3,
            "secrets::resolve",
            json!({"ref":"secret://ANTHROPIC_API_KEY"}),
            ROUTER,
        ),
        invocation(
            4,
            "secrets::resolve",
            json!({"ref":"ANTHROPIC_API_KEY"}),
            INTRUDER,
        ),
        invocation(
            5,
            "secrets::resolve",
            json!({"ref":"secret://ANTHROPIC_API_KEY"}),
            FOREIGN_ROUTER,
        ),
        invocation(6, "secrets::resolve", json!({"ref":"not a ref"}), ROUTER),
        invocation(
            7,
            "secrets::detect",
            json!({"names":["OPENAI_API_KEY","ANTHROPIC_API_KEY"]}),
            CONSOLE,
        ),
        invocation(
            8,
            "secrets::import",
            json!({"name":"OPENAI_API_KEY","source":"dotenv","consumers":["llm-router"]}),
            CONSOLE,
        ),
        invocation(9, "secrets::get", json!({"name":"OPENAI_API_KEY"}), CONSOLE),
        invocation(10, "secrets::status", json!({}), CONSOLE),
    ]
}

fn reply(frame: &Value, result: Value) -> Value {
    json!({
        "type": "invocationresult",
        "invocation_id": frame["invocation_id"],
        "function_id": frame["function_id"],
        "result": result,
    })
}

#[tokio::test]
async fn boot_serves_the_contract_and_stops_on_sigterm() {
    check(Some("-TERM")).await;
}

#[cfg(unix)]
#[tokio::test]
async fn sigint_also_stops_cleanly() {
    check(Some("-INT")).await;
}

async fn check(signal: Option<&str>) {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(".env"),
        format!("# provider keys\nexport OPENAI_API_KEY=\"{DOTENV_VALUE}\"\n"),
    )
    .unwrap();
    let xdg = root.path().join("xdg");

    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let mut script = steps().into_iter();
    let engine = support::fake_engine(move |frame| {
        tx.send(frame.clone()).unwrap();
        let mut replies = Vec::new();
        match frame["type"].as_str().unwrap_or_default() {
            "invokefunction" if frame["invocation_id"].is_string() => {
                let result = match frame["function_id"].as_str().unwrap() {
                    "configuration::ensure" => {
                        json!({"action":"seeded","entry":{"id":frame["data"]["id"],"value":frame["data"]["initial_value"]}})
                    }
                    "configuration::get" => json!({"value":{"data_dir":"data/secrets","key_file":null}}),
                    "engine::workers::list" => json!({"workers":[
                        {"id":CONSOLE,"name":"ade","namespace":"default"},
                        {"id":ROUTER,"name":"llm-router","namespace":"default"},
                        {"id":INTRUDER,"name":"harness","namespace":"default"},
                        {"id":FOREIGN_ROUTER,"name":"llm-router","namespace":"someone-else"}
                    ]}),
                    _ => json!({}),
                };
                replies.push(reply(&frame, result));
            }
            // Registered last (bind_reload): the catalog is complete.
            "registerfunction" if frame["id"] == "secrets::on-config-change" => {
                replies.push(json!({
                    "type":"registertrigger","id":"sub-1","trigger_type":"secrets::changed",
                    "function_id":"subscriber::on-secret","config":{"names":["secret://ANTHROPIC_API_KEY"]}
                }));
            }
            "triggerregistrationresult" if frame["id"] == "sub-1" => {
                replies.extend(script.next());
            }
            "invocationresult" => replies.extend(script.next()),
            _ => {}
        }
        replies
    })
    .await;

    let mut child = Command::new(env!("CARGO_BIN_EXE_secrets"))
        .env("III_URL", &engine.url)
        .env("III_COMPOSE_DIR", &project)
        .env("XDG_CONFIG_HOME", &xdg)
        .env("HOME", root.path().join("home"))
        .env("RUST_LOG", "debug")
        .env("OTEL_ENABLED", "false")
        .env_remove("III_SECRETS_KEY")
        .env_remove("III_CONFIG_NAME")
        .env_remove("SHELL")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let logs = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let mut worker = Worker(child);

    let mut frames = Vec::new();
    let mut results = std::collections::BTreeMap::new();
    let mut delivered = Vec::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        while results.len() < 10 || delivered.is_empty() {
            let frame = rx.recv().await.expect("engine connection stays open");
            if frame["type"] == "invocationresult" {
                let step: u32 = frame["invocation_id"].as_str().unwrap()[24..]
                    .parse()
                    .unwrap();
                results.insert(step, frame.clone());
            }
            if frame["type"] == "invokefunction" && frame["function_id"] == "subscriber::on-secret"
            {
                delivered.push(frame["data"].clone());
            }
            frames.push(frame);
        }
    })
    .await
    .unwrap_or_else(|_| panic!("scripted conversation finished; frames so far: {frames:#?}"));

    // Catalog: nine functions, the reload sink, the trigger type with schemas.
    let functions: BTreeSet<&str> = frames
        .iter()
        .filter(|f| f["type"] == "registerfunction")
        .filter_map(|f| f["id"].as_str())
        .collect();
    let mut expected: BTreeSet<&str> = secrets::api::ids::ALL_FUNCTIONS.into_iter().collect();
    expected.insert("secrets::on-config-change");
    assert_eq!(functions, expected);
    // Registry publication rejects the permissive "unknown" schema.
    let typed = |schema: &Value| {
        [
            "type",
            "properties",
            "$ref",
            "allOf",
            "anyOf",
            "oneOf",
            "enum",
            "items",
            "const",
        ]
        .iter()
        .any(|key| schema.get(key).is_some())
    };
    for frame in frames.iter().filter(|f| f["type"] == "registerfunction") {
        for field in ["request_format", "response_format"] {
            assert!(
                typed(&frame[field]),
                "{} {field}: {}",
                frame["id"],
                frame[field]
            );
        }
    }
    let trigger_type = frames
        .iter()
        .find(|f| f["type"] == "registertriggertype" && f["id"] == "secrets::changed")
        .expect("secrets::changed registered");
    assert!(trigger_type["call_request_format"]["properties"]["action"].is_object());
    assert!(trigger_type["trigger_request_format"]["properties"]["names"].is_object());
    let set_schema = frames
        .iter()
        .find(|f| f["type"] == "registerfunction" && f["id"] == "secrets::set")
        .unwrap();
    assert!(set_schema["request_format"]["properties"]["value"].is_object());
    assert!(set_schema["response_format"]["properties"]
        .get("value")
        .is_none());
    let ensure = frames
        .iter()
        .find(|f| f["function_id"] == "configuration::ensure")
        .expect("configuration entry registered");
    assert_eq!(ensure["data"]["id"], "secrets");
    assert_eq!(
        ensure["data"]["initial_value"],
        json!({"data_dir":"data/secrets","key_file":null,"env_file":".env"})
    );

    let result = |step: u32| results[&step]["result"].clone();
    let error = |step: u32| results[&step]["error"].clone();

    let set = result(1);
    assert_eq!(set["ref"], "secret://ANTHROPIC_API_KEY");
    assert_eq!(set["hint"], "sk-ant…9f2c");
    assert_eq!(set["consumers"], json!(["llm-router"]));
    assert!(set.get("value").is_none());
    let listed = result(2);
    assert_eq!(listed["secrets"].as_array().unwrap().len(), 1);
    assert!(!listed.to_string().contains("boot-test-value"));

    assert_eq!(result(3), json!({"name":"ANTHROPIC_API_KEY","value":VALUE}));
    assert_eq!(error(4)["code"], "SECRET_FORBIDDEN", "{}", results[&4]);
    assert_eq!(
        error(5)["code"],
        "SECRET_FORBIDDEN",
        "another namespace's router is not ours"
    );
    assert_eq!(error(6)["code"], "INVALID_REFERENCE");

    let detected = result(7);
    let openai = &detected["results"][0];
    assert_eq!(openai["name"], "OPENAI_API_KEY");
    assert_eq!(openai["stored"], false);
    assert_eq!(openai["sources"][0]["kind"], "dotenv");
    assert_eq!(openai["sources"][0]["hint"], "sk-pro…3456");
    assert_eq!(
        openai["sources"][0]["location"],
        project.join(".env").display().to_string()
    );
    let anthropic = &detected["results"][1];
    assert_eq!(anthropic["stored"], true);
    assert_eq!(anthropic["stored_hint"], "sk-ant…9f2c");
    assert_eq!(anthropic["sources"], json!([]));

    let imported = result(8);
    assert_eq!(imported["name"], "OPENAI_API_KEY");
    assert_eq!(imported["hint"], "sk-pro…3456");
    assert_eq!(result(9)["fingerprint"], imported["fingerprint"]);

    let status = result(10);
    let vault_path = project.join("data/secrets/vault.json");
    assert_eq!(status["vault_path"], vault_path.display().to_string());
    assert_eq!(status["key_source"], "file");
    assert_eq!(status["count"], 2);
    assert_eq!(status["version"], 1);
    let key_path = std::path::PathBuf::from(status["key_path"].as_str().unwrap());
    assert!(
        key_path.starts_with(xdg.join("iii/secrets")),
        "{key_path:?}"
    );

    // The subscriber filtered on ANTHROPIC_API_KEY: one creation, no value.
    assert_eq!(delivered.len(), 1, "{delivered:?}");
    assert_eq!(delivered[0]["name"], "ANTHROPIC_API_KEY");
    assert_eq!(delivered[0]["action"], "created");
    assert_eq!(delivered[0]["fingerprint"], set["fingerprint"]);

    // The only frame carrying a value is the authorized resolve result.
    let carrying: Vec<&Value> = frames
        .iter()
        .filter(|f| {
            let text = f.to_string();
            text.contains(VALUE) || text.contains(DOTENV_VALUE)
        })
        .collect();
    assert_eq!(carrying, vec![&results[&3]]);

    let vault = std::fs::read_to_string(&vault_path).unwrap();
    assert!(!vault.contains(VALUE) && !vault.contains(DOTENV_VALUE));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&vault_path), 0o600);
        assert_eq!(mode(vault_path.parent().unwrap()), 0o700);
        assert_eq!(mode(&key_path), 0o600);
        assert_eq!(mode(key_path.parent().unwrap()), 0o700);
    }

    #[cfg(unix)]
    if let Some(signal) = signal {
        let status = Command::new("kill")
            .arg(signal)
            .arg(worker.0.id().to_string())
            .status()
            .unwrap();
        assert!(status.success());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let exit = loop {
            if let Some(exit) = worker.0.try_wait().unwrap() {
                break exit;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "worker exits after {signal}"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(exit.success(), "{exit:?}");
    }
    #[cfg(not(unix))]
    let _ = signal;
    let _ = worker.0.kill();
    let _ = worker.0.wait();
    let logs = logs.join().unwrap();
    assert!(logs.contains("secrets ready"), "{logs}");
    assert!(
        !logs.contains(VALUE) && !logs.contains(DOTENV_VALUE),
        "{logs}"
    );
}
