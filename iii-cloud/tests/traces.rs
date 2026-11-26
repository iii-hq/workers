//! Through a real engine (`III_ENGINE_BIN`; skipped when unset): a value the
//! person gives never reaches the engine's traces. Its own test binary: the
//! SDK's telemetry is one per process and goes to the first engine it meets,
//! so another real-engine test running beside it would get the spans.

mod support;

use std::sync::Arc;
use std::time::Duration;

use iii_cloud::config::Config;
use iii_cloud::ctx::{Ctx, IiiPlatform};
use iii_cloud::functions::register_all;
use iii_cloud::store::Store;
use iii_sdk::Error;
use serde_json::{Value, json};
use support::real_engine::{RealEngine, call};
use tokio::sync::RwLock;

fn remote_code(e: Error) -> String {
    match e {
        Error::Remote { code, .. } => code,
        other => other.to_string(),
    }
}

/// Every trace the engine kept, with its spans and tree, as text. An engine
/// that can't list or read them fails the test: an empty text proves nothing.
async fn all_traces(probe: &iii_sdk::IIIClient) -> Result<(usize, String), String> {
    let list = call(probe, "engine::traces::list", json!({}))
        .await
        .map_err(|e| format!("engine::traces::list: {e}"))?;
    let mut text = list.to_string();
    let mut ids = vec![];
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(id) = m.get("trace_id").and_then(Value::as_str) {
                    out.push(id.to_string());
                }
                m.values().for_each(|x| walk(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(&list, &mut ids);
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err(format!("the engine listed no trace: {list}"));
    }
    let count = ids.len();
    for id in ids {
        for function in ["engine::traces::spans", "engine::traces::tree"] {
            let v = call(probe, function, json!({"trace_id": id}))
                .await
                .map_err(|e| format!("{function} {id}: {e}"))?;
            text.push_str(&v.to_string());
        }
    }
    Ok((count, text))
}

/// The engine's traces once `done` holds (it exports them a while after the
/// call), read again 500 ms later so late spans are in. Fails after 20 s.
async fn traces_when(
    probe: &iii_sdk::IIIClient,
    done: impl Fn(usize, &str) -> bool,
) -> (usize, String) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let read = all_traces(probe).await;
        if let Ok((n, text)) = &read
            && done(*n, text)
        {
            tokio::time::sleep(Duration::from_millis(500)).await;
            return all_traces(probe).await.unwrap();
        }
        if std::time::Instant::now() > deadline {
            panic!("the traces never got there: {read:?}");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// B2: the value the person types reaches the vault of the page call and nothing
/// else: the SDK must not keep it on the span of `execute iii-cloud::secret::provide`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_value_is_not_recorded_in_traces() {
    let Some(engine) =
        RealEngine::start_with_env(&[("OTEL_ENABLED", "true"), ("OTEL_EXPORTER_TYPE", "memory")])
            .await
    else {
        eprintln!(
            "SKIPPED a_secret_value_is_not_recorded_in_traces: III_ENGINE_BIN is unset, so no trace was checked"
        );
        return;
    };
    let iii = engine.client("iii-cloud").await;
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    // A repository: the run's `.env` is written only where git says it is untracked.
    support::repo::git(&ws, &["init", "-q", "-b", "main"]);
    let cfg = Config {
        cloud_url: "http://cloud.test".into(),
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        ..Config::default()
    };
    let store = Arc::new(Store::open(&cfg.data_path()));
    let ctx = Ctx::with_tick(
        Arc::new(RwLock::new(Arc::new(cfg))),
        store,
        Arc::new(IiiPlatform { iii: iii.clone() }),
        Some(Duration::from_millis(10)),
    );
    // A key connection and a selected environment, so `provide` has somewhere to write.
    ctx.store.update(|s| {
        use iii_cloud::types::*;
        let mut conn = Connection::signed_out("http://cloud.test");
        conn.status = ConnStatus::Connected;
        conn.method = Some(Method::ApiKey);
        conn.api_key = Some(ApiKeyRef {
            secret: "K".into(),
            team_id: "t".into(),
            environment_id: "e".into(),
        });
        s.connection = Some(conn);
        s.project = Some(Project {
            path: ws.to_string_lossy().into(),
            repository: "a/b".into(),
            checkout: Checkout {
                branch: None,
                head: "h".into(),
                changed_files: vec![],
                ahead: 0,
                not_served: None,
                branches: vec![],
            },
            team: None,
            teams: vec![],
            cloud_project: None,
            environment: Some(EnvironmentInfo {
                id: "e".into(),
                name: "production".into(),
                production: true,
                branch: "main".into(),
                manifest: None,
                url: None,
            }),
            environments: vec![],
            importable: None,
            github_access: None,
            cloud_projects: vec![],
            other_cloud_projects: vec![],
            default_branch: None,
            workspace: Some(Workspace {
                mode: WorkspaceMode::Checkout,
                path: ws.to_string_lossy().into(),
                branch: "main".into(),
                from_head: None,
                not_brought: vec![],
                source: None,
                not_brought_reasons: Default::default(),
                source_sha: None,
            }),
            fell_back_from: None,
        });
    });
    register_all(&iii, &ctx);
    let probe = engine.client("probe").await;
    engine.function(&probe, "iii-cloud::secret::provide").await;
    engine.function(&probe, "iii-cloud::values::save").await;
    let ask = |name: &str| {
        iii_cloud::secret::ask(
            &ctx,
            iii_cloud::types::SecretAsk {
                handoff_id: None,
                name: name.into(),
                description: "x".into(),
                mode: None,
                external: None,
            },
        )
    };
    let (first, second) = (ask("FIRST").await.unwrap(), ask("SECOND").await.unwrap());

    // Control: with payload capture on, the engine does keep the input. Without
    // this the test below would prove nothing.
    let leaked = "control-value-aaaaaaaa";
    call(
        &probe,
        "iii-cloud::secret::provide",
        json!({"request_id": first.id, "value": leaked}),
    )
    .await
    .unwrap();
    let (seen, control) = traces_when(&probe, |_, t| t.contains(leaked)).await;
    assert!(
        control.contains(leaked),
        "with capture on the engine must keep the input, or the check below proves nothing"
    );

    // The worker's own setting: the same call as main() makes before the SDK starts.
    unsafe { iii_cloud::disable_trace_payloads() };
    let value = "typed-secret-value-bbbbbbbb";
    call(
        &probe,
        "iii-cloud::secret::provide",
        json!({"request_id": second.id, "value": value}),
    )
    .await
    .unwrap();
    let (seen, traces) = traces_when(&probe, |n, _| n > seen).await;
    assert!(!traces.contains(value), "the secret value is in the traces");
    assert!(
        std::fs::read_to_string(ws.join(".env"))
            .unwrap()
            .contains(value),
        "it did reach the .env"
    );

    // values::save: an API key is refused, but the input still crossed the engine.
    let saved = "values-save-secret-cccccccc";
    let refused = call(
        &probe,
        "iii-cloud::values::save",
        json!({"environment_id": "e", "items": [
            {"name": "TOKEN", "from": "typed", "type": "secret", "value": saved}
        ]}),
    )
    .await
    .unwrap_err();
    assert_eq!(remote_code(refused), "NEEDS_BROWSER_SIGN_IN");
    let (_, traces) = traces_when(&probe, |n, _| n > seen).await;
    assert!(!traces.contains(saved), "a saved value is in the traces");
}
