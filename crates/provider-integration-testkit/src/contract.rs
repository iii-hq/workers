use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use axum::http::StatusCode;
use iii_sdk::errors::Error as IiiError;
use iii_sdk::{register_worker, IIIClient, RegisterFunction};
use llm_router::register::register_router;
use serde_json::{json, Value};

use crate::case::{CredentialMode, ProtocolFamily, ProviderCase};
use crate::protocol::{auth_response, happy_sse, quota_response, truncated_sse};
use crate::runtime::{call, test_init_options, Engine};
use crate::stub::{CapturedRequest, StubResponse, StubUpstream};

const API_KEY: &str = "provider-contract-api-key";
const OAUTH_TOKEN: &str = "provider-contract-oauth-token";
const ACCOUNT_ID: &str = "provider-contract-account";

async fn register_fake_vault(engine_url: &str, mode: CredentialMode) -> Option<IIIClient> {
    let credential = match mode {
        CredentialMode::ApiKey => return None,
        CredentialMode::ClaudeOauth => json!({
            "type": "oauth",
            "provider": "claude-code",
            "access_token": OAUTH_TOKEN,
            "expires_at": 4_102_444_800i64
        }),
        CredentialMode::CodexOauth => json!({
            "type": "oauth",
            "provider": "openai-codex",
            "access_token": OAUTH_TOKEN,
            "expires_at": 4_102_444_800i64,
            "provider_extra": { "account_id": ACCOUNT_ID }
        }),
    };
    let vault = register_worker(engine_url, test_init_options());
    vault.register_function(
        "auth::get_token",
        RegisterFunction::new_async(move |_input: Value| {
            let credential = credential.clone();
            async move { Ok::<Value, IiiError>(credential) }
        }),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let listed = call(
            &vault,
            "engine::functions::list",
            json!({ "include_internal": true }),
        )
        .await
        .ok()
        .and_then(|value| value.get("functions").cloned())
        .and_then(|value| value.as_array().cloned())
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("function_id").and_then(Value::as_str) == Some("auth::get_token")
            })
        });
        if listed || Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Some(vault)
}

async fn configure(router: &IIIClient, case: ProviderCase, endpoint: &str) -> anyhow::Result<()> {
    let slice = match case.credential {
        CredentialMode::ApiKey => json!({ "api_key": API_KEY, "api_url": endpoint }),
        CredentialMode::ClaudeOauth | CredentialMode::CodexOauth => {
            json!({ "api_url": endpoint })
        }
    };
    call(
        router,
        "configuration::set",
        json!({
            "id": "llm-router",
            "value": {
                "settings": {
                    "retry_max": 1,
                    "stream_timeout_ms": 15_000,
                    "idle_timeout_ms": 5_000
                },
                "providers": { case.id: slice }
            }
        }),
    )
    .await
    .context("configure llm-router")?;
    Ok(())
}

async fn wait_for_provider(router: &IIIClient, provider: &str) -> anyhow::Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let list = call(router, "router::provider::list", json!({})).await?;
        if let Some(found) = list["providers"]
            .as_array()
            .and_then(|providers| providers.iter().find(|item| item["id"] == provider))
        {
            return Ok(found.clone());
        }
        if Instant::now() >= deadline {
            bail!("provider {provider} did not register: {list}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_registration_token(
    provider: &IIIClient,
    provider_id: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let token = call(
            provider,
            "state::get",
            json!({
                "scope": format!("provider-{provider_id}"),
                "key": "registration_token"
            }),
        )
        .await?;
        if token.as_str().is_some_and(|value| !value.is_empty()) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("registration token was not persisted: {token}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_discovered_models(router: &IIIClient, case: ProviderCase) -> anyhow::Result<()> {
    if !case.requires_model_discovery {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let list = call(
            router,
            "router::models::list",
            json!({ "provider": case.id }),
        )
        .await?;
        let ids: Vec<&str> = list["models"]
            .as_array()
            .map(|models| {
                models
                    .iter()
                    .filter_map(|model| model["id"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        if ids.contains(&case.model) && ids.contains(&case.alternate_model) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "{} discovery did not reconcile {} and {}; have {ids:?}",
                case.id,
                case.model,
                case.alternate_model
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct ChatResult {
    response: Value,
    frames: Vec<Value>,
}

async fn chat(
    engine_url: &str,
    case: ProviderCase,
    model: &str,
    request_id: &str,
) -> anyhow::Result<ChatResult> {
    let consumer = register_worker(engine_url, test_init_options());
    let channel = iii_sdk::helpers::create_channel(&consumer, None).await?;
    let frames = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = frames.clone();
    channel
        .reader
        .on_message(move |message| {
            if let Ok(frame) = serde_json::from_str(&message) {
                captured.lock().expect("frames lock").push(frame);
            }
        })
        .await;
    let writer_ref = channel.writer_ref.clone();
    let pump = tokio::spawn(async move {
        let _ = channel.reader.read_all().await;
    });
    let response = call(
        &consumer,
        "router::chat",
        json!({
            "writer_ref": writer_ref,
            "request_id": request_id,
            "session_id": "provider-contract-session",
            "model": model,
            "provider": case.id,
            "system_prompt": "Be concise.",
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": "contract probe" }],
                "timestamp": 1
            }],
            "tools": [{
                "name": "contract::probe",
                "description": "Return deterministic contract data.",
                "parameters": {
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"]
                }
            }]
        }),
    )
    .await?;
    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
    consumer.shutdown();
    let collected = frames.lock().expect("frames lock").clone();
    Ok(ChatResult {
        response,
        frames: collected,
    })
}

pub(crate) async fn run_contract(case: ProviderCase) -> anyhow::Result<()> {
    eprintln!("provider contract: {} ({})", case.id, case.family.id());
    let engine = Engine::start().await?;
    let isolated_home = tempfile::tempdir()?;
    std::env::set_var("HOME", isolated_home.path());
    std::env::set_var("CODEX_HOME", isolated_home.path().join("codex"));
    std::env::set_var("CLAUDE_CONFIG_DIR", isolated_home.path().join("claude"));
    std::env::set_var("PROVIDER_READ_TIMEOUT_SECS", "5");
    if case.id == "command-code" {
        std::env::set_var("CMD_ZDR", "0");
    }

    let stub = StubUpstream::start(case).await?;
    stub.respond([StubResponse::sse(happy_sse(case.family))]);
    let router = register_worker(&engine.url, test_init_options());
    register_router(router.clone())
        .await
        .context("register router")?;
    configure(&router, case, &stub.endpoint(case.generation_path)).await?;
    let vault = register_fake_vault(&engine.url, case.credential).await;
    let provider = register_worker(&engine.url, test_init_options());
    (case.register)(provider.clone()).await?;

    let listed = wait_for_provider(&router, case.id).await?;
    match case.credential {
        CredentialMode::ApiKey => anyhow::ensure!(
            listed["configured"] == true,
            "API-key provider not configured: {listed}"
        ),
        CredentialMode::ClaudeOauth | CredentialMode::CodexOauth => anyhow::ensure!(
            listed["available"] == true,
            "OAuth provider not available: {listed}"
        ),
    }
    wait_for_discovered_models(&router, case).await?;
    wait_for_registration_token(&provider, case.id).await?;

    // Successful streaming plus exact request/auth/tool serialization.
    stub.clear_requests();
    stub.respond([StubResponse::sse(happy_sse(case.family))]);
    let first = chat(&engine.url, case, case.model, "contract-happy-1").await?;
    anyhow::ensure!(
        first.response["ok"] == true,
        "happy response: {}",
        first.response
    );
    assert_terminal(&first.frames, "done")?;
    let requests = stub.post_requests();
    anyhow::ensure!(
        requests.len() == 1,
        "happy request count: {}",
        requests.len()
    );
    assert_request(case, &requests[0], case.upstream_model)?;

    // A second model is honored without restarting the provider.
    stub.clear_requests();
    stub.respond([StubResponse::sse(happy_sse(case.family))]);
    let second = chat(&engine.url, case, case.alternate_model, "contract-happy-2").await?;
    anyhow::ensure!(
        second.response["ok"] == true,
        "second model: {}",
        second.response
    );
    let requests = stub.post_requests();
    anyhow::ensure!(
        requests.len() == 1,
        "second request count: {}",
        requests.len()
    );
    assert_request(case, &requests[0], case.alternate_upstream_model)?;

    // Billing/quota failures are terminal and are never retried.
    stub.clear_requests();
    stub.respond([quota_response(case)]);
    let quota = chat(&engine.url, case, case.model, "contract-quota").await?;
    anyhow::ensure!(
        quota.response["ok"] == false,
        "quota response: {}",
        quota.response
    );
    assert_error_kind(&quota.frames, "permanent")?;
    anyhow::ensure!(stub.post_requests().len() == 1, "quota request was retried");

    // A pre-content transient failure is retried once by the router.
    stub.clear_requests();
    stub.respond([
        StubResponse::json(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":{"message":"temporary upstream failure","type":"server_error"}}"#,
        ),
        StubResponse::sse(happy_sse(case.family)),
    ]);
    let transient = chat(&engine.url, case, case.model, "contract-transient").await?;
    anyhow::ensure!(
        transient.response["ok"] == true,
        "transient response: {}",
        transient.response
    );
    anyhow::ensure!(
        stub.post_requests().len() == 2,
        "transient retry count mismatch"
    );

    // A cut inside function-call arguments: one transient error, no retry
    // after forwarded content, and the next call succeeds without a restart.
    stub.clear_requests();
    stub.respond([StubResponse::sse(truncated_sse(case.family))]);
    let truncated = chat(&engine.url, case, case.model, "contract-truncated").await?;
    anyhow::ensure!(
        truncated.response["ok"] == false,
        "truncated response: {}",
        truncated.response
    );
    assert_error_kind(&truncated.frames, "transient")?;
    assert_truncated_partial(&truncated.frames)?;
    anyhow::ensure!(
        truncated
            .frames
            .iter()
            .filter(|frame| is_terminal(frame))
            .count()
            == 1,
        "truncated stream must end in exactly one terminal frame"
    );
    anyhow::ensure!(
        stub.post_requests().len() == 1,
        "truncated stream was retried after content reached the caller"
    );
    stub.clear_requests();
    stub.respond([StubResponse::sse(happy_sse(case.family))]);
    let recovered = chat(&engine.url, case, case.model, "contract-after-truncation").await?;
    anyhow::ensure!(
        recovered.response["ok"] == true,
        "call after truncation: {}",
        recovered.response
    );
    assert_terminal(&recovered.frames, "done")?;

    // Router abort reaches the provider and cancels the in-flight HTTP body.
    stub.clear_requests();
    let cancelled = Arc::new(AtomicBool::new(false));
    stub.respond([StubResponse::hanging(cancelled.clone())]);
    let engine_url = engine.url.clone();
    let abort_case = case;
    let pending = tokio::spawn(async move {
        chat(&engine_url, abort_case, abort_case.model, "contract-abort").await
    });
    stub.wait_for_post_count(1).await?;
    let aborted = call(
        &router,
        "router::abort",
        json!({ "request_id": "contract-abort" }),
    )
    .await?;
    anyhow::ensure!(aborted["aborted"] == true, "abort response: {aborted}");
    let aborted_chat = tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .context("aborted chat timed out")???;
    anyhow::ensure!(
        aborted_chat.response["stop_reason"] == "aborted",
        "aborted chat response: {}",
        aborted_chat.response
    );
    let cancel_deadline = Instant::now() + Duration::from_secs(5);
    while !cancelled.load(Ordering::SeqCst) && Instant::now() < cancel_deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    anyhow::ensure!(
        cancelled.load(Ordering::SeqCst),
        "upstream body was not cancelled"
    );

    // Authentication errors are terminal and clear the provider cache.
    stub.clear_requests();
    stub.respond([auth_response(case.family)]);
    let auth = chat(&engine.url, case, case.model, "contract-auth").await?;
    anyhow::ensure!(
        auth.response["ok"] == false,
        "auth response: {}",
        auth.response
    );
    assert_error_kind(&auth.frames, "auth_expired")?;
    anyhow::ensure!(stub.post_requests().len() == 1, "auth request was retried");

    write_contract_result(case)?;

    if let Some(vault) = vault {
        vault.shutdown();
    }
    provider.shutdown();
    router.shutdown();
    Ok(())
}

fn write_contract_result(case: ProviderCase) -> anyhow::Result<()> {
    let Some(directory) = std::env::var_os("PROVIDER_CONTRACT_ARTIFACTS_DIR") else {
        return Ok(());
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    let result = json!({
        "schema_version": 1,
        "provider": case.id,
        "protocol_family": case.family.id(),
        "status": "passed",
        "network": "loopback-only",
        "credentials": "synthetic",
        "scenarios": [
            "registration-and-configuration",
            "request-and-stream",
            "model-change",
            "quota-no-retry",
            "transient-retry",
            "truncated-stream-no-retry",
            "abort",
            "auth-no-retry"
        ]
    });
    let filename = format!("{}-{}.json", case.id, case.family.id());
    std::fs::write(
        directory.join(filename),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(())
}

fn assert_terminal(frames: &[Value], expected: &str) -> anyhow::Result<()> {
    let last = frames.last().context("stream emitted no frames")?;
    anyhow::ensure!(last["type"] == expected, "terminal frame: {last}");
    Ok(())
}

fn is_terminal(frame: &Value) -> bool {
    matches!(frame["type"].as_str(), Some("done") | Some("error"))
}

fn assert_truncated_partial(frames: &[Value]) -> anyhow::Result<()> {
    let last = frames
        .last()
        .context("truncated stream emitted no frames")?;
    let message = last["error"]["error_message"].as_str().unwrap_or_default();
    anyhow::ensure!(
        message.contains("stream truncated") && message.contains("phase=sse-decode"),
        "truncation error must name the decode phase: {last}"
    );
    let content = last["error"]["content"]
        .as_array()
        .context("truncated error carries no content")?;
    anyhow::ensure!(
        content
            .iter()
            .any(|block| block["type"] == "text" && block["text"] == "partial contract"),
        "truncated error must keep the streamed text: {last}"
    );
    let function_calls: Vec<_> = content
        .iter()
        .filter(|block| block["type"] == "function_call")
        .collect();
    anyhow::ensure!(
        !function_calls.is_empty(),
        "truncated error must keep the unfinished function call: {last}"
    );
    for block in function_calls {
        let arguments = &block["arguments"];
        anyhow::ensure!(
            arguments.get("_partial").is_some() || arguments.get("_raw").is_some(),
            "a cut function call must carry degraded arguments: {block}"
        );
    }
    Ok(())
}

fn assert_error_kind(frames: &[Value], expected: &str) -> anyhow::Result<()> {
    let last = frames.last().context("error stream emitted no frames")?;
    anyhow::ensure!(last["type"] == "error", "terminal frame: {last}");
    anyhow::ensure!(
        last["error"]["error_kind"] == expected,
        "error kind mismatch: {last}"
    );
    Ok(())
}

fn assert_request(
    case: ProviderCase,
    request: &CapturedRequest,
    expected_model: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        request.path == case.generation_path,
        "unexpected endpoint; request={}",
        serde_json::to_string(&request.redacted())?
    );
    match case.credential {
        CredentialMode::ApiKey
            if case.id == "command-code" || case.family != ProtocolFamily::AnthropicMessages =>
        {
            anyhow::ensure!(
                request.header("authorization") == Some(&format!("Bearer {API_KEY}")),
                "bearer key missing"
            );
        }
        CredentialMode::ApiKey => {
            anyhow::ensure!(
                request.header("x-api-key") == Some(API_KEY),
                "x-api-key missing"
            );
        }
        CredentialMode::ClaudeOauth => {
            anyhow::ensure!(
                request.header("authorization") == Some(&format!("Bearer {OAUTH_TOKEN}")),
                "Claude OAuth bearer missing"
            );
        }
        CredentialMode::CodexOauth => {
            anyhow::ensure!(
                request.header("authorization") == Some(&format!("Bearer {OAUTH_TOKEN}")),
                "Codex OAuth bearer missing"
            );
            anyhow::ensure!(
                request.header("chatgpt-account-id") == Some(ACCOUNT_ID),
                "Codex account header missing"
            );
        }
    }
    let body: Value = serde_json::from_str(&request.body).context("request body JSON")?;
    anyhow::ensure!(body["model"] == expected_model, "request model: {body}");
    anyhow::ensure!(body["stream"] == true, "stream flag missing: {body}");
    anyhow::ensure!(
        body.get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty()),
        "tool schema missing: {body}"
    );
    Ok(())
}
