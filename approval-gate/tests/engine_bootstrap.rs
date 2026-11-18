use approval_gate::testkit::{call, engine_bin, require_engine};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn supplied_engine_serves_real_state() {
    if engine_bin().is_none() {
        eprintln!("skipping: optional iii engine is absent");
        return;
    }
    let engine = require_engine().await.expect("supplied engine must boot");
    let client = iii_sdk::register_worker(&engine.url, iii_sdk::InitOptions::default());
    call(
        &client,
        "state::set",
        json!({"scope":"bootstrap", "key":"probe", "value":{"ready":true}}),
    )
    .await
    .expect("real state worker writes");
    let result = call(
        &client,
        "state::get",
        json!({"scope":"bootstrap", "key":"probe"}),
    )
    .await
    .expect("real state worker reads");
    assert_eq!(result["ready"], true);
    client.shutdown();
}

#[cfg(unix)]
#[test]
fn supplied_failed_engine_cannot_skip() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "supplied_engine_serves_real_state",
            "--nocapture",
        ])
        .env("III_ENGINE_BIN", "/bin/false")
        .output()
        .expect("run failed-engine regression in isolated process");
    assert!(
        !output.status.success(),
        "failed engine was reported as success"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("supplied integration process exited"),
        "{stderr}"
    );
    assert!(!stderr.contains("skipping:"), "{stderr}");
}
