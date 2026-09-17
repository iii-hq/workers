use super::*;
use crate::functions::search_jev::JevSearch;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn tools() -> Vec<ToolSchema> {
    [
        ("mail::send", "Send an email message."),
        ("state::get", "Read a stored value."),
        ("engine::functions::list", "List functions."),
        ("directory::search_functions", "Find functions."),
    ]
    .into_iter()
    .map(|(name, description)| ToolSchema {
        name: name.into(),
        description: description.into(),
        parameters: json!({"type": "object"}),
    })
    .collect()
}

fn deps(server: &MockServer) -> Deps {
    Deps {
        config: SkillsConfig {
            function_search_mode: FunctionSearchMode::Jev,
            function_search_model_path: None,
            registry_search: false,
            ..SkillsConfig::default()
        }
        .into_shared(),
        catalog: Arc::new(RwLock::new(Arc::new(tools()))),
        sessions: Arc::default(),
        registry_cache: RegistryCache::new(std::time::Duration::ZERO),
        semantic: SemanticSearch::default(),
        jev: JevSearch::for_test(
            format!("{}/v1/systemone", server.uri()),
            Some("test-key".into()),
        ),
    }
}

fn reply(request: &Request, score: f64) -> ResponseTemplate {
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let answers: serde_json::Map<String, Value> = body["questions"]
        .as_object()
        .unwrap()
        .keys()
        .map(|key| (key.clone(), json!({"type":"noul", "noul":score})))
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({
        "model":"jev-1.13.0", "answers":answers,
        "usage":{"input_tokens":100,"output_tokens":10}
    }))
}

async fn ask(deps: &Deps, capabilities: &[&str]) -> SearchFunctionsResponse {
    search_functions(
        deps,
        SearchFunctionsRequest {
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
        },
    )
    .await
    .unwrap()
}

fn ids(response: &SearchFunctionsResponse) -> Vec<&str> {
    response
        .workers
        .iter()
        .flat_map(|w| w.functions.iter().map(|f| f.function_id.as_str()))
        .collect()
}

#[tokio::test]
async fn configured_jev_key_reloads_and_clearing_restores_the_boot_key() {
    use crate::configuration::{apply_config, SharedState};
    use crate::functions::skills::RegisteredWorkersCache;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| reply(r, 0.9))
        .expect(6)
        .mount(&server)
        .await;
    let deps = deps(&server);
    let state = SharedState::new(
        deps.config.clone(),
        Arc::default(),
        deps.registry_cache.clone(),
        Arc::new(RegisteredWorkersCache::new(0)),
        deps.config.load().topology(),
        crate::hook::HintBindingState::default(),
        deps.clone(),
    );
    for key in [
        Some(json!("configured-key")),
        Some(json!("rotated-key")),
        None,
        Some(Value::Null),
        Some(json!("")),
        Some(json!(" \t ")),
    ] {
        let mut value = deps.config.load().to_json();
        if let Some(key) = key {
            value["function_search_jev_api_key"] = key;
        } else {
            value
                .as_object_mut()
                .unwrap()
                .remove("function_search_jev_api_key");
        }
        apply_config(&state, SkillsConfig::from_json(&value).unwrap()).await;
        assert_eq!(
            ids(&ask(&deps, &["dispatch correspondence"]).await),
            ["mail::send", "state::get"]
        );
    }
    let requests = server.received_requests().await.unwrap();
    let headers: Vec<&str> = requests
        .iter()
        .map(|r| r.headers["authorization"].to_str().unwrap())
        .collect();
    assert_eq!(
        headers,
        [
            "Bearer configured-key",
            "Bearer rotated-key",
            "Bearer test-key",
            "Bearer test-key",
            "Bearer test-key",
            "Bearer test-key"
        ]
    );
}

#[tokio::test]
async fn configured_jev_key_works_without_a_boot_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| reply(r, 0.9))
        .expect(1)
        .mount(&server)
        .await;
    let mut deps = deps(&server);
    deps.jev = JevSearch::for_test(format!("{}/v1/systemone", server.uri()), None);
    let mut value = deps.config.load().to_json();
    value["function_search_jev_api_key"] = json!("configured-key");
    deps.config
        .store(Arc::new(SkillsConfig::from_json(&value).unwrap()));
    assert_eq!(
        ids(&ask(&deps, &["dispatch correspondence"]).await),
        ["mail::send", "state::get"]
    );
    value["function_search_jev_api_key"] = Value::Null;
    deps.config
        .store(Arc::new(SkillsConfig::from_json(&value).unwrap()));
    assert_eq!(
        ids(&ask(&deps, &["send an email message"]).await),
        ["mail::send"]
    );
}

#[tokio::test]
async fn credential_rotation_does_not_mix_keys_within_a_search() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| reply(r, 0.9).set_delay(std::time::Duration::from_millis(40)))
        .expect(10)
        .mount(&server)
        .await;
    let deps = deps(&server);
    *deps.catalog.write().await = Arc::new(
        (0..80)
            .map(|i| ToolSchema {
                name: format!("mail::send{i:02}"),
                description: "Send an email.".into(),
                parameters: json!({"type":"object"}),
            })
            .collect(),
    );
    let mut config = deps.config.load().to_json();
    config["function_search_jev_api_key"] = json!("first-key");
    deps.config
        .store(Arc::new(SkillsConfig::from_json(&config).unwrap()));
    let first_deps = deps.clone();
    let first = tokio::spawn(async move { ask(&first_deps, &["dispatch correspondence"]).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    config["function_search_jev_api_key"] = json!("second-key");
    deps.config
        .store(Arc::new(SkillsConfig::from_json(&config).unwrap()));
    assert!(!first.await.unwrap().workers.is_empty());
    assert!(!ask(&deps, &["dispatch correspondence"])
        .await
        .workers
        .is_empty());
    let requests = server.received_requests().await.unwrap();
    for request in &requests[..5] {
        assert_eq!(request.headers["authorization"], "Bearer first-key");
    }
    for request in &requests[5..] {
        assert_eq!(request.headers["authorization"], "Bearer second-key");
    }
}

#[tokio::test]
async fn jev_finds_nonlexical_candidates_without_a_local_model() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(|r: &Request| reply(r, 0.9))
        .expect(1)
        .mount(&server)
        .await;
    let response = ask(&deps(&server), &["dispatch correspondence"]).await;
    assert_eq!(ids(&response), ["mail::send", "state::get"]);
    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(!body.contains("engine::functions::list"));
    assert!(!body.contains("directory::search_functions"));
}

#[tokio::test]
async fn valid_no_match_does_not_restore_lexical_candidates() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| reply(r, 0.1))
        .expect(1)
        .mount(&server)
        .await;
    let response = ask(&deps(&server), &["send an email message"]).await;
    assert!(response.workers.is_empty());
}

#[tokio::test]
async fn failed_jev_uses_the_lexical_baseline() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;
    let deps = deps(&server);
    let remote = ask(&deps, &["send an email message"]).await;
    let mut cfg = (**deps.config.load()).clone();
    cfg.function_search_mode = FunctionSearchMode::Lexical;
    deps.config.store(Arc::new(cfg));
    let lexical = ask(&deps, &["send an email message"]).await;
    assert_eq!(ids(&remote), ids(&lexical));
    assert_eq!(ids(&remote), ["mail::send"]);
}

#[tokio::test]
async fn exact_ids_and_intrinsic_capabilities_need_no_remote_call() {
    let server = MockServer::start().await;
    let deps = deps(&server);
    assert_eq!(ids(&ask(&deps, &["mail::send"]).await), ["mail::send"]);
    assert!(ask(&deps, &["summarize provided text"])
        .await
        .workers
        .is_empty());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn exact_lane_survives_a_remote_no_match_for_the_other_capability() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let capabilities = body["state"]["capabilities"].as_object().unwrap();
            assert_eq!(capabilities.len(), 1);
            assert_eq!(
                capabilities.values().next().unwrap(),
                "dispatch correspondence"
            );
            reply(request, 0.1)
        })
        .expect(1)
        .mount(&server)
        .await;
    let response = ask(&deps(&server), &["mail::send", "dispatch correspondence"]).await;
    assert_eq!(ids(&response), ["mail::send"]);
}

#[tokio::test]
async fn missing_credentials_preserve_lexical_search() {
    let server = MockServer::start().await;
    let mut deps = deps(&server);
    deps.jev = JevSearch::default();
    assert_eq!(
        ids(&ask(&deps, &["send an email message"]).await),
        ["mail::send"]
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn timeout_budget_is_shared_across_capability_batches() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| reply(r, 0.9).set_delay(std::time::Duration::from_millis(200)))
        .expect(1)
        .mount(&server)
        .await;
    let deps = deps(&server);
    let mut cfg = (**deps.config.load()).clone();
    cfg.function_search_jev_timeout_ms = 40;
    deps.config.store(Arc::new(cfg));
    let response = ask(&deps, &["send an email message"; 18]).await;
    assert_eq!(ids(&response), ["mail::send"]);
}

#[tokio::test]
async fn jev_keeps_multiple_capabilities_within_existing_caps() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| reply(r, 0.8))
        .mount(&server)
        .await;
    let deps = deps(&server);
    let tools: Vec<ToolSchema> = (0..24)
        .map(|i| ToolSchema {
            name: format!("worker{i}::act"),
            description: "Perform an action.".into(),
            parameters: json!({"type":"object"}),
        })
        .collect();
    *deps.catalog.write().await = Arc::new(tools);
    let response = ask(&deps, &["perform an action"; 6]).await;
    assert_eq!(ids(&response).len(), MAX_SEARCH_FUNCTIONS);
    assert_eq!(response.workers.len(), 12);
}

#[tokio::test]
async fn jev_session_suppression_and_catalog_invalidation_still_work() {
    use opentelemetry::baggage::BaggageExt;
    use opentelemetry::{Context, KeyValue};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| reply(r, 0.9))
        .mount(&server)
        .await;
    let deps = deps(&server);
    let context =
        Context::current().with_baggage([KeyValue::new(SESSION_BAGGAGE_KEY, "jev-test-session")]);
    let _guard = context.attach();
    assert_eq!(ids(&ask(&deps, &["mail::send"]).await), ["mail::send"]);
    let repeated = ask(&deps, &["dispatch correspondence"]).await;
    assert_eq!(ids(&repeated), ["state::get"]);
    assert!(repeated.guidance.contains("Already provided"));
    let mut next = tools();
    next[0].description = "Deliver a message.".into();
    *deps.catalog.write().await = Arc::new(next);
    assert_eq!(
        ids(&ask(&deps, &["dispatch correspondence"]).await).len(),
        2
    );
}

async fn registry_fixture(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/w"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "workers":[{"name":"courier","version":"1.2.3","description":"Messages"}],
            "pagination":{}
        })))
        .mount(server)
        .await;
    let mut functions: Vec<Value> = (0..8)
        .map(|i| {
            json!({
                "name":format!("courier::send{i}"), "description":"Send an email message.",
                "request_schema":{"type":"object"}
            })
        })
        .collect();
    functions.extend([
        json!({"name":"courier::private","description":"Send an email message.","metadata":{"internal":true}}),
        json!({"name":"mail::send","description":"Send an email message."}),
    ]);
    Mock::given(method("GET"))
        .and(path("/w/courier"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "worker":{"name":"courier","functions":functions}
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/w/courier/skills"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"skills":[],"prompts":[]})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn jev_registry_candidates_preserve_owners_and_limits() {
    let remote = MockServer::start().await;
    let registry = MockServer::start().await;
    registry_fixture(&registry).await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| reply(r, 0.9))
        .expect(2)
        .mount(&remote)
        .await;
    let deps = deps(&remote);
    let mut cfg = (**deps.config.load()).clone();
    cfg.registry_search = true;
    cfg.registry_url = registry.uri();
    let mut value = cfg.to_json();
    value["function_search_jev_api_key"] = json!("registry-config-key");
    cfg = SkillsConfig::from_json(&value).unwrap();
    deps.config.store(Arc::new(cfg));
    let response = ask(&deps, &["dispatch correspondence"]).await;
    assert_eq!(response.installable.len(), 1);
    let worker = &response.installable[0];
    assert_eq!(worker.name, "courier");
    assert_eq!(worker.version, "1.2.3");
    assert_eq!(worker.install.payload, json!({"worker":"courier"}));
    assert_eq!(worker.functions.len(), MAX_INSTALLABLE_FUNCTIONS);
    assert!(worker
        .functions
        .iter()
        .all(|f| f.function_id.starts_with("courier::send")));
    assert!(ids(&response).iter().all(|id| !id.starts_with("courier::")));
    let requests = remote.received_requests().await.unwrap();
    assert!(requests
        .iter()
        .all(|r| r.headers["authorization"] == "Bearer registry-config-key"));
    assert!(requests
        .iter()
        .all(|r| !String::from_utf8_lossy(&r.body).contains("courier::private")));
}

#[tokio::test]
async fn registry_jev_failure_falls_back_to_its_lexical_pool() {
    let remote = MockServer::start().await;
    let registry = MockServer::start().await;
    registry_fixture(&registry).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(2)
        .mount(&remote)
        .await;
    let deps = deps(&remote);
    let mut cfg = (**deps.config.load()).clone();
    cfg.registry_search = true;
    cfg.registry_url = registry.uri();
    deps.config.store(Arc::new(cfg));
    let response = ask(&deps, &["send an email message"]).await;
    assert_eq!(ids(&response), ["mail::send"]);
    assert_eq!(
        response.installable[0].functions.len(),
        MAX_INSTALLABLE_FUNCTIONS
    );
}

#[tokio::test]
async fn registry_jev_failure_preserves_an_exact_lane_among_lexical_matches() {
    let remote = MockServer::start().await;
    let registry = MockServer::start().await;
    let mut functions: Vec<Value> = (0..8)
        .map(|i| json!({"name":format!("a::send{i}"),"description":"Send an email message."}))
        .collect();
    // These short ID tokens do not pass the ordinary BM25 admission rules.
    functions.push(json!({"name":"a::b","description":"Perform a task."}));
    Mock::given(method("GET"))
        .and(path("/w/a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "worker":{"name":"a","functions":functions}
        })))
        .mount(&registry)
        .await;
    Mock::given(method("GET"))
        .and(path("/w/a/skills"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"skills":[],"prompts":[]})))
        .mount(&registry)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&remote)
        .await;
    let deps = deps(&remote);
    let mut cfg = (**deps.config.load()).clone();
    cfg.registry_url = registry.uri();
    let workers = installable_from_candidates(
        &cfg,
        &deps.registry_cache,
        None,
        &[],
        &["a::b".into(), "send an email message".into()],
        &[RegistryCandidate {
            name: "a".into(),
            version: "1.0.0".into(),
            description: String::new(),
        }],
        Some((
            &deps.jev,
            tokio::time::Instant::now() + std::time::Duration::from_secs(3),
        )),
    )
    .await;
    assert_eq!(workers[0].functions.len(), MAX_INSTALLABLE_FUNCTIONS);
    assert!(workers[0].functions.iter().any(|f| f.function_id == "a::b"));
}

#[tokio::test]
async fn mode_reload_waits_for_an_in_progress_hybrid_activation() {
    use crate::configuration::{apply_config, SharedState};
    use crate::functions::skills::RegisteredWorkersCache;
    let server = MockServer::start().await;
    let deps = deps(&server);
    deps.semantic.set_enabled(false);
    let cfg = (**deps.config.load()).clone();
    let state = SharedState::new(
        deps.config.clone(),
        Arc::default(),
        deps.registry_cache.clone(),
        Arc::new(RegisteredWorkersCache::new(0)),
        cfg.topology(),
        crate::hook::HintBindingState::default(),
        deps.clone(),
    );
    // Block the activation after it publishes Hybrid, before its rebuild.
    let catalog_guard = deps.catalog.write().await;
    let hybrid_state = state.clone();
    let mut hybrid_cfg = cfg.clone();
    hybrid_cfg.function_search_mode = FunctionSearchMode::Hybrid;
    let hybrid = tokio::spawn(async move { apply_config(&hybrid_state, hybrid_cfg).await });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while deps.config.load().function_search_mode != FunctionSearchMode::Hybrid {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut jev = tokio::spawn(async move { apply_config(&state, cfg).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut jev)
            .await
            .is_err()
    );
    drop(catalog_guard);
    hybrid.await.unwrap();
    jev.await.unwrap();
    assert_eq!(
        deps.config.load().function_search_mode,
        FunctionSearchMode::Jev
    );
    let previous = deps.semantic.requested_fingerprint();
    let mut next = tools();
    next[0].description = "Later catalog while Jev is active.".into();
    activate_catalog(&deps.catalog, &deps.semantic, next).await;
    assert_eq!(deps.semantic.requested_fingerprint(), previous);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_refreshes_keep_the_requested_index_on_the_current_catalog() {
    let server = MockServer::start().await;
    let deps = deps(&server);
    let mut updates = tokio::task::JoinSet::new();
    for i in 0..64 {
        let deps = deps.clone();
        updates.spawn(async move {
            let mut next = tools();
            next[0].description = format!("Catalog version {i}.");
            activate_catalog(&deps.catalog, &deps.semantic, next).await;
        });
    }
    while let Some(result) = updates.join_next().await {
        result.unwrap();
    }
    assert_eq!(
        deps.semantic.requested_fingerprint(),
        Some(tool_fingerprint(&deps.catalog.read().await))
    );
}

#[tokio::test]
async fn switching_back_to_hybrid_rebuilds_an_unchanged_catalog() {
    use crate::configuration::{apply_config, SharedState};
    use crate::functions::skills::RegisteredWorkersCache;
    let server = MockServer::start().await;
    let deps = deps(&server);
    deps.semantic.set_enabled(false);
    // Refreshing the catalog in Jev mode still publishes its new entries.
    let mut next = tools();
    next[0].description = "Deliver correspondence.".into();
    assert!(activate_catalog(&deps.catalog, &deps.semantic, next).await);
    assert!(deps.semantic.requested_fingerprint().is_none());
    let cfg = (**deps.config.load()).clone();
    let state = SharedState::new(
        deps.config.clone(),
        Arc::default(),
        deps.registry_cache.clone(),
        Arc::new(RegisteredWorkersCache::new(0)),
        cfg.topology(),
        crate::hook::HintBindingState::default(),
        deps.clone(),
    );
    let mut hybrid = cfg.clone();
    hybrid.function_search_mode = FunctionSearchMode::Hybrid;
    apply_config(&state, hybrid).await;
    let fingerprint = tool_fingerprint(&deps.catalog.read().await);
    assert_eq!(
        deps.semantic.requested_fingerprint().as_deref(),
        Some(fingerprint.as_str())
    );
    apply_config(&state, cfg).await;
    let mut next = tools();
    next[0].description = "A later catalog.".into();
    assert!(activate_catalog(&deps.catalog, &deps.semantic, next).await);
    assert_eq!(
        deps.semantic.requested_fingerprint().as_deref(),
        Some(fingerprint.as_str()),
        "Jev catalog refresh must not schedule another dense rebuild"
    );
}

#[tokio::test]
async fn benchmark_keeps_intrinsic_and_exact_lanes_aligned() {
    let server = MockServer::start().await;
    let deps = deps(&server);
    let capabilities = vec!["summarize provided text".into(), "mail::send".into()];
    let result = benchmark_installed(&deps, &capabilities).await;
    assert!(result.rankings[0].is_empty());
    assert_eq!(result.rankings[1][0].0, "mail::send");
    assert_eq!(result.selected, ["mail::send"]);
    assert_eq!(result.jev_requests, 0);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn benchmark_preserves_known_usage_and_stage_time_after_a_later_block_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            if body["state"]["functions"].as_object().unwrap().len() == 16 {
                reply(request, 0.9)
            } else {
                ResponseTemplate::new(500).set_delay(std::time::Duration::from_millis(30))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let deps = deps(&server);
    *deps.catalog.write().await = Arc::new(
        (0..17)
            .map(|i| ToolSchema {
                name: format!("mail::send{i:02}"),
                description: "Send an email message.".into(),
                parameters: json!({"type":"object"}),
            })
            .collect(),
    );
    let result = benchmark_installed(&deps, &["send an email message".into()]).await;
    assert!(!result.jev_complete);
    assert_eq!(result.jev_requests, 1);
    assert_eq!(result.jev_questions, 16);
    assert_eq!(result.input_tokens, 100);
    assert_eq!(result.output_tokens, 10);
    assert!(result.jev_elapsed_ms >= 30);
    assert!(
        !result.selected.is_empty(),
        "lexical fallback still returns candidates"
    );
}
