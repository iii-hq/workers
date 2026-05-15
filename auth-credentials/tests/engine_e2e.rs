use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::{register_worker, InitOptions, TriggerRequest, III};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

struct WorkerProcess {
    child: Child,
}

impl WorkerProcess {
    fn spawn(engine_url: &str, nonce: &str) -> Self {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let config_path = format!("{manifest_dir}/config.yaml");
        let child = Command::new(env!("CARGO_BIN_EXE_auth-credentials"))
            .args(["--url", engine_url, "--config", &config_path])
            .env("AUTH_CREDENTIALS_STORE", "iii_state")
            .env("ANTHROPIC_API_KEY", format!("sk-env-anthropic-{nonce}"))
            .env("OPENAI_API_KEY", format!("sk-env-openai-{nonce}"))
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn auth-credentials worker");
        Self { child }
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn trigger(client: &III, function_id: &str, payload: Value) -> anyhow::Result<Value> {
    timeout(
        Duration::from_secs(8),
        client.trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        }),
    )
    .await
    .map_err(|_| anyhow::anyhow!("{function_id} timed out"))?
    .map_err(|err| anyhow::anyhow!("{function_id} failed: {err}"))
}

async fn wait_for_auth_functions(client: &III) -> anyhow::Result<()> {
    let mut last_err = None;
    for _ in 0..40 {
        match trigger(client, "auth::list_providers", json!({})).await {
            Ok(_) => return Ok(()),
            Err(err) => last_err = Some(err),
        }
        sleep(Duration::from_millis(250)).await;
    }
    Err(anyhow::anyhow!(
        "auth worker did not register in time: {:?}",
        last_err.map(|err| err.to_string())
    ))
}

async fn best_effort_delete(client: &III, provider: &str) {
    let _ = trigger(
        client,
        "auth::delete_token",
        json!({ "provider": provider }),
    )
    .await;
}

#[tokio::test]
async fn auth_worker_real_engine_contract_scenarios() -> anyhow::Result<()> {
    let Some(engine_url) = std::env::var("IIITEST_ENGINE_URL").ok() else {
        eprintln!("skipping auth worker engine e2e: set IIITEST_ENGINE_URL to a running engine");
        return Ok(());
    };

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
        .to_string();
    let client = register_worker(&engine_url, InitOptions::default());
    let _worker = WorkerProcess::spawn(&engine_url, &nonce);
    wait_for_auth_functions(&client).await?;

    let alpha = format!("auth-e2e-alpha-{nonce}");
    let beta = format!("auth-e2e-beta-{nonce}");
    let oauth = format!("auth-e2e-oauth-{nonce}");
    let old_secret = format!("sk-old-{nonce}");
    let new_secret = format!("sk-new-{nonce}");

    for provider in [&alpha, &beta, &oauth, "anthropic", "openai"] {
        best_effort_delete(&client, provider).await;
    }

    trigger(
        &client,
        "auth::set_token",
        json!({
            "provider": format!("  {alpha}  "),
            "credential": { "type": "api_key", "key": format!("sk-alpha-{nonce}") }
        }),
    )
    .await?;
    let trimmed = trigger(&client, "auth::get_token", json!({ "provider": alpha })).await?;
    assert_eq!(trimmed["type"].as_str(), Some("api_key"));
    assert_eq!(
        trimmed["key"].as_str(),
        Some(format!("sk-alpha-{nonce}").as_str())
    );

    trigger(
        &client,
        "auth::set_token",
        json!({
            "provider": beta,
            "credential": { "type": "api_key", "key": old_secret }
        }),
    )
    .await?;
    trigger(
        &client,
        "auth::set_token",
        json!({
            "provider": beta,
            "credential": { "type": "api_key", "key": new_secret }
        }),
    )
    .await?;
    let rotated = trigger(&client, "auth::get_token", json!({ "provider": beta })).await?;
    assert_eq!(rotated["key"].as_str(), Some(new_secret.as_str()));
    assert!(!serde_json::to_string(&rotated)?.contains(&old_secret));

    trigger(
        &client,
        "auth::set_token",
        json!({
            "provider": oauth,
            "credential": {
                "type": "oauth",
                "access_token": format!("oauth-access-{nonce}"),
                "refresh_token": format!("oauth-refresh-{nonce}"),
                "expires_at": 1893456000_i64,
                "scopes": ["models:read", "messages:write"],
                "provider_extra": { "workspace": "prod" }
            }
        }),
    )
    .await?;
    let oauth_value = trigger(&client, "auth::get_token", json!({ "provider": oauth })).await?;
    assert_eq!(oauth_value["type"].as_str(), Some("oauth"));
    assert_eq!(
        oauth_value["access_token"].as_str(),
        Some(format!("oauth-access-{nonce}").as_str())
    );
    assert_eq!(
        oauth_value["refresh_token"].as_str(),
        Some(format!("oauth-refresh-{nonce}").as_str())
    );
    assert_eq!(
        oauth_value["scopes"],
        json!(["models:read", "messages:write"])
    );

    let oauth_status = trigger(&client, "auth::status", json!({ "provider": oauth })).await?;
    let oauth_status_rendered = serde_json::to_string(&oauth_status)?;
    assert_eq!(oauth_status["configured"].as_bool(), Some(true));
    assert_eq!(oauth_status["source"].as_str(), Some("stored"));
    assert_eq!(oauth_status["label"].as_str(), Some("oauth"));
    assert!(!oauth_status_rendered.contains(&format!("oauth-access-{nonce}")));
    assert!(!oauth_status_rendered.contains(&format!("oauth-refresh-{nonce}")));

    let env_openai = trigger(&client, "auth::get_token", json!({ "provider": "openai" })).await?;
    assert_eq!(env_openai["type"].as_str(), Some("api_key"));
    assert_eq!(
        env_openai["key"].as_str(),
        Some(format!("sk-env-openai-{nonce}").as_str())
    );

    trigger(
        &client,
        "auth::set_token",
        json!({
            "provider": "openai",
            "credential": { "type": "api_key", "key": format!("sk-stored-openai-{nonce}") }
        }),
    )
    .await?;
    let stored_status = trigger(&client, "auth::status", json!({ "provider": "openai" })).await?;
    let stored_status_rendered = serde_json::to_string(&stored_status)?;
    assert_eq!(stored_status["source"].as_str(), Some("stored"));
    assert!(stored_status["label"]
        .as_str()
        .is_some_and(|label| label.starts_with("api-key:sk-st")));
    assert!(!stored_status_rendered.contains(&format!("sk-stored-openai-{nonce}")));
    assert!(!stored_status_rendered.contains(&format!("sk-env-openai-{nonce}")));

    trigger(
        &client,
        "auth::delete_token",
        json!({ "provider": "openai" }),
    )
    .await?;
    let fallback_after_delete =
        trigger(&client, "auth::get_token", json!({ "provider": "openai" })).await?;
    assert_eq!(
        fallback_after_delete["key"].as_str(),
        Some(format!("sk-env-openai-{nonce}").as_str())
    );

    let unknown = trigger(
        &client,
        "auth::get_token",
        json!({ "provider": format!("unknown-{nonce}") }),
    )
    .await?;
    assert!(unknown.is_null());
    let unknown_status = trigger(
        &client,
        "auth::status",
        json!({ "provider": format!("unknown-{nonce}") }),
    )
    .await?;
    assert_eq!(unknown_status["configured"].as_bool(), Some(false));
    assert!(unknown_status.get("source").is_none());
    assert!(unknown_status.get("label").is_none());

    let blank_error = trigger(
        &client,
        "auth::set_token",
        json!({
            "provider": " ",
            "credential": { "type": "api_key", "key": format!("sk-invalid-{nonce}") }
        }),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(blank_error.contains("provider must be non-empty"));

    let listed = trigger(&client, "auth::list_providers", json!({})).await?;
    let providers = listed["providers"]
        .as_array()
        .expect("providers is an array")
        .iter()
        .filter_map(Value::as_str)
        .filter(|provider| provider.contains(&nonce))
        .collect::<Vec<_>>();
    assert_eq!(
        providers,
        vec![alpha.as_str(), beta.as_str(), oauth.as_str()]
    );
    let listed_rendered = serde_json::to_string(&listed)?;
    assert!(!listed_rendered.contains(&new_secret));
    assert!(!listed_rendered.contains(&format!("oauth-access-{nonce}")));

    for provider in [&alpha, &beta, &oauth, "anthropic", "openai"] {
        best_effort_delete(&client, provider).await;
    }
    client.shutdown_async().await;
    Ok(())
}
