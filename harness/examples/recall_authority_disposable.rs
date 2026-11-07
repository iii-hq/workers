//! Explicit opt-in disposable-engine check. Never connects to a configured,
//! default or user-supplied endpoint. No model, real store or paid service.
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use iii_sdk::protocol::CallRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

struct Resources {
    children: Vec<Child>,
    clients: Vec<Arc<IIIClient>>,
}
impl Drop for Resources {
    fn drop(&mut self) {
        for c in &self.clients {
            c.shutdown();
        }
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn binary(name: &str) -> Result<PathBuf> {
    let path =
        PathBuf::from(std::env::var_os(name).with_context(|| format!("explicit {name} required"))?);
    ensure!(
        path.is_absolute() && path.is_file(),
        "{name} must be an existing absolute binary"
    );
    Ok(path.canonicalize()?)
}
fn client(url: &str, name: &str, r: &mut Resources) -> Arc<IIIClient> {
    let c = Arc::new(register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: name.into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    ));
    r.clients.push(c.clone());
    c
}
async fn rpc(c: &IIIClient, id: &str, p: Value) -> Result<Value> {
    Ok(c.call(CallRequest {
        function_id: id.into(),
        payload: p,
        action: None,
        timeout_ms: Some(1500),
    })
    .await?)
}
async fn ready(c: &IIIClient, id: &str, p: Value) -> Result<Value> {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(v) = rpc(c, id, p.clone()).await {
            return Ok(v);
        }
        ensure!(
            Instant::now() < end,
            "isolated dependency {id} did not become ready"
        );
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Fail closed BEFORE a listener, child or SDK client exists. There is no
    // environment/default engine URL branch, connect-or-skip, or binary build.
    ensure!(
        std::env::var("III_RECALL_DISPOSABLE").as_deref() == Ok("1"),
        "III_RECALL_DISPOSABLE=1 required"
    );
    let engine = binary("III_RECALL_ENGINE_BIN")?;
    let state = binary("III_RECALL_STATE_BIN")?;
    ensure!(
        std::env::var_os("III_ENGINE_WS_URL").is_none() && std::env::var_os("III_URL").is_none(),
        "remove live endpoint environment variables"
    );
    ensure!(
        engine != state,
        "engine and State binaries must be distinct"
    );
    let dir = tempfile::tempdir()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    ensure!(port != 49134, "refuse the live default port");
    let url = format!("ws://127.0.0.1:{port}");
    let config = dir.path().join("engine.yaml");
    std::fs::write(
        &config,
        format!("workers:\n  - name: iii-worker-manager\n    config:\n      port: {port}\n"),
    )?;
    let mut resources = Resources {
        children: Vec::new(),
        clients: Vec::new(),
    };
    drop(listener);
    resources.children.push(
        Command::new(engine)
            .args(["--no-update-check", "--config"])
            .arg(&config)
            .current_dir(dir.path())
            .env_clear()
            .env("HOME", dir.path())
            .env("TMPDIR", dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start disposable engine")?,
    );
    let control = client(&url, "recall-fixture", &mut resources);
    let workers = ready(&control, "engine::workers::list", json!({})).await?;
    ensure!(
        !workers.to_string().contains("iii-state"),
        "builtin state must not own this test engine"
    );
    let value = json!({"adapter":{"name":"kv","config":{"store_method":"in_memory"}},"triggers_enabled":false});
    let stored = value.clone();
    control.register_function(
        "configuration::ensure",
        RegisterFunction::new_async(|_: Value| async {
            Ok::<_, iii_sdk::errors::Error>(json!({"ok":true}))
        }),
    );
    control.register_function(
        "configuration::get",
        RegisterFunction::new_async(move |p: Value| {
            let stored = stored.clone();
            async move {
                if p["id"] != "state" {
                    return Err(iii_sdk::errors::Error::Handler(
                        "unexpected config id".into(),
                    ));
                }
                Ok(json!({"value":stored}))
            }
        }),
    );
    let seed = dir.path().join("state.yaml");
    std::fs::write(&seed, serde_yaml::to_string(&value)?)?;
    resources.children.push(
        Command::new(state)
            .arg("--url")
            .arg(&url)
            .arg("--config")
            .arg(&seed)
            .current_dir(dir.path())
            .env_clear()
            .env("HOME", dir.path())
            .env("TMPDIR", dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start real State with synthetic in-memory config")?,
    );
    ready(
        &control,
        "state::get",
        json!({"scope":"probe","key":"none"}),
    )
    .await?;
    let owner = client(&url, "harness", &mut resources);
    let attacker = client(&url, "recall-foreign-worker", &mut resources);
    ready(&owner, "engine::workers::list", json!({})).await?;
    ready(&attacker, "engine::workers::list", json!({})).await?;
    let scope = "harness_recall_delivery";
    // Ensure each named worker completed registration before identity checks.
    for (c, name) in [(&owner, "harness"), (&attacker, "recall-foreign-worker")] {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let workers = rpc(c, "engine::workers::list", json!({})).await?;
            if workers["workers"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|w| w["name"] == name))
            {
                break;
            }
            ensure!(Instant::now() < end, "named worker {name} not registered");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    let foreign = rpc(
        &attacker,
        "state::claim-namespace",
        json!({"functions_prefix":"harness","scopes":[scope]}),
    )
    .await
    .expect_err("foreign worker cannot claim Harness namespace");
    ensure!(
        foreign.to_string().contains("FORBIDDEN"),
        "wrong denial: {foreign}"
    );
    let claim = rpc(
        &owner,
        "state::claim-namespace",
        json!({"functions_prefix":"harness","scopes":[scope]}),
    )
    .await?;
    ensure!(claim["claimed"] == true, "owner must be admitted: {claim}");
    let receipt = json!([{"session_id":"synthetic-session","entry_id":"e1","channel":"invoked-agent","query":{"entry_id":"e1","digest":format!("{:x}",Sha256::digest(b"Synthetic controlled delivery"))},"continuation":null}]);
    // Readiness is checked without a mutating retry: a timed-out CAS may
    // already have landed, and must not be relabelled as a clean first write.
    ready(
        &owner,
        "harness::state::get",
        json!({"scope":scope,"key":"synthetic-session"}),
    )
    .await?;
    let written = rpc(
        &owner,
        "harness::state::compare-and-set",
        json!({"scope":scope,"key":"synthetic-session","value":receipt,"durable":true}),
    )
    .await?;
    ensure!(
        written["swapped"] == true && written["durable"] == true,
        "owner durable write failed: {written}"
    );
    ensure!(
        rpc(
            &owner,
            "harness::state::get",
            json!({"scope":scope,"key":"synthetic-session"})
        )
        .await?
            == receipt,
        "real State must persist the owner receipt"
    );
    for function in ["state::get", "state::set", "state::delete"] {
        let error = rpc(
            &attacker,
            function,
            json!({"scope":scope,"key":"synthetic-session","value":{"channel":"direct"}}),
        )
        .await
        .expect_err("public state must not reach a reserved scope");
        ensure!(
            error.to_string().contains("RESERVED_SCOPE"),
            "wrong public-scope denial: {error}"
        );
    }
    // Private accessors are operator/worker APIs, not caller-authenticated by
    // State. The real Harness agent chokepoint must deny their use even when
    // the agent's allow-list names them. Do not claim a worker ACL here.
    let deps = harness::deps::Deps::new(
        owner.clone(),
        Arc::new(tokio::sync::RwLock::new(Arc::new(
            harness::config::WorkerConfig::default(),
        ))),
        harness::discovery::new_cell(),
        harness::skills::new_cell(),
        harness::events::TurnEvents::register(&owner),
        harness::hooks::HookRegistry::register(&owner),
    );
    let policy =
        harness::policy::CompiledPolicy::from(Some(&harness::types::turn::FunctionPolicy {
            allow: vec!["*".into()],
            deny: Vec::new(),
            expose: Default::default(),
        }));
    for function in [
        "harness::state::get",
        "harness::state::compare-and-set",
        "state::claim-namespace",
    ] {
        let denied = harness::functions::observe::invoke(
            &deps,
            &deps.engine().await,
            &policy,
            function,
            &json!({"scope":scope,"key":"synthetic-session","value":null}),
            "attacker-agent",
            false,
            None,
        )
        .await;
        ensure!(
            denied.is_error && denied.details["error"] == "policy_denied",
            "agent boundary did not deny {function}"
        );
    }
    ensure!(
        rpc(
            &owner,
            "harness::state::get",
            json!({"scope":scope,"key":"synthetic-session"})
        )
        .await?
            == receipt,
        "denied calls must not mutate receipt"
    );
    println!(
        "{}",
        json!({"check":"disposable-state-recall-authority","real_engine":true,"real_state":true,"store":"in_memory","owner_claim":true,"foreign_claim_denied":true,"public_scope_denied":true,"agent_private_calls_denied":true,"operator_private_acl_claimed":false,"endpoint":url})
    );
    Ok(())
}
