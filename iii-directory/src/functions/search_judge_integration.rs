use super::*;
use crate::functions::search_judge::{JudgeError, JudgeSearch};
use judge_contract::EvaluateRequest;
use std::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tools() -> Vec<ToolSchema> {
    [
        ("mail::send", "Send an email message."),
        ("state::get", "Read a stored value."),
        ("engine::functions::list", "List functions."),
        ("directory::search_functions", "Find functions."),
        (
            judge_contract::FUNCTION_ID,
            "Evaluate typed questions through the configured judge provider.",
        ),
    ]
    .into_iter()
    .map(|(name, description)| ToolSchema {
        name: name.into(),
        description: description.into(),
        parameters: json!({"type": "object"}),
    })
    .collect()
}

/// Skill roots for judge tests: the defaults resolve to the crate's shipped
/// `skills/` and the developer's `~/.agents/skills`, which would add a
/// nondeterministic skill evaluation to every search.
fn skill_roots(root: &std::path::Path) -> SkillsConfig {
    SkillsConfig {
        function_search_mode: FunctionSearchMode::Judge,
        function_search_model_path: None,
        registry_search: false,
        skills_folder: root.join("skills").display().to_string(),
        local_skills_folder: root.join("local").display().to_string(),
        agents_skills_folder: root.join("agents").display().to_string(),
        global_agents_skills_folder: root.join("global-agents").display().to_string(),
        ..SkillsConfig::default()
    }
}

fn empty_skill_root() -> &'static std::path::Path {
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

type Requests = Arc<Mutex<Vec<EvaluateRequest>>>;

/// A judge hub double: records every `judge::evaluate` payload and answers
/// through `reply` after `delay_ms`.
fn mock_hub<F>(delay_ms: u64, reply: F) -> (JudgeSearch, Requests)
where
    F: Fn(&EvaluateRequest) -> Result<Value, JudgeError> + Send + Sync + 'static,
{
    let requests: Requests = Arc::default();
    let seen = requests.clone();
    let client = JudgeSearch::from_evaluator(move |request| {
        let result = reply(&request);
        seen.lock().unwrap().push(request);
        async move {
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            result
        }
    });
    (client, requests)
}

fn scoring(score: f64) -> (JudgeSearch, Requests) {
    mock_hub(0, move |request| Ok(reply(request, score)))
}

fn deps(judge: JudgeSearch) -> Deps {
    deps_with_skill_root(judge, empty_skill_root())
}

fn deps_with_skill_root(judge: JudgeSearch, root: &std::path::Path) -> Deps {
    Deps {
        config: skill_roots(root).into_shared(),
        catalog: Arc::new(RwLock::new(Arc::new(tools()))),
        sessions: Arc::default(),
        registry_cache: RegistryCache::new(std::time::Duration::ZERO),
        semantic: SemanticSearch::default(),
        registered_workers: None,
        iii: None,
        judge,
    }
}

/// Every question answered `score`, except rows about the judge itself,
/// which the provider never finds relevant to a directory query.
fn reply(request: &EvaluateRequest, score: f64) -> Value {
    let evaluation = &request.evaluations[0];
    let answers: serde_json::Map<String, Value> = evaluation
        .questions
        .keys()
        .map(|key| {
            let (_, f) = key.split_once('_').unwrap();
            let own =
                evaluation.state["functions"][f]["function_id"] == judge_contract::FUNCTION_ID;
            let noul = if own { 0.0 } else { score };
            (key.clone(), json!({"type":"noul", "noul":noul}))
        })
        .collect();
    let questions = answers.len();
    let mut results = serde_json::Map::new();
    results.insert(evaluation.id.clone(), json!({"answers": answers}));
    json!({
        "status":"ok", "model":"jev-1.13.0", "results": results,
        "stats":{"attempts":1,"requests":1,"questions":questions,
            "input_tokens":100,"output_tokens":10,"elapsed_ms":1,"usage_complete":true}
    })
}

fn hub_error(code: &str) -> Value {
    json!({"status":"error","code":code,
        "stats":{"attempts":1,"requests":0,"questions":0,"input_tokens":0,
            "output_tokens":0,"elapsed_ms":0,"usage_complete":false}})
}

fn is_skill_block(request: &EvaluateRequest) -> bool {
    request.evaluations[0].state["skills"].is_object()
}

fn bodies(requests: &Requests) -> Vec<Value> {
    requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| serde_json::to_value(&request.evaluations[0]).unwrap())
        .collect()
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
async fn judge_finds_nonlexical_candidates_without_a_local_model() {
    let (judge, requests) = scoring(0.9);
    let response = ask(&deps(judge), &["dispatch correspondence"]).await;
    assert_eq!(ids(&response), ["mail::send", "state::get"]);
    assert_eq!(response.search_mode, FunctionSearchMode::Judge);
    let bodies = bodies(&requests);
    assert_eq!(bodies.len(), 1);
    let body = bodies[0].to_string();
    assert!(!body.contains("engine::functions::list"));
    assert!(!body.contains("directory::search_functions"));
}

#[tokio::test]
async fn valid_no_match_does_not_restore_lexical_candidates() {
    let (judge, requests) = scoring(0.1);
    let response = ask(&deps(judge), &["send an email message"]).await;
    assert!(response.workers.is_empty());
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_judge_uses_lexical_when_hybrid_is_unavailable() {
    let (judge, requests) = mock_hub(0, |_| Ok(hub_error("http")));
    let mut deps = deps(judge);
    let missing_bundle = tempfile::tempdir().unwrap();
    for semantic in [
        SemanticSearch::default(),
        SemanticSearch::new(Some(missing_bundle.path().into())),
    ] {
        deps.semantic = semantic;
        let outcome = benchmark_installed(&deps, &["send an email message".into()]).await;
        assert!(!outcome.judge_complete);
        assert!(!outcome.hybrid_complete);
        assert_eq!(outcome.selected, ["mail::send"]);
    }
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn an_unregistered_judge_worker_uses_the_local_lanes_without_a_call() {
    let (judge, requests) = scoring(0.9);
    let deps = deps(judge);
    let catalog: Vec<ToolSchema> = tools()
        .into_iter()
        .filter(|tool| tool.name != judge_contract::FUNCTION_ID)
        .collect();
    *deps.catalog.write().await = Arc::new(catalog);
    let response = ask(&deps, &["send an email message"]).await;
    assert_eq!(ids(&response), ["mail::send"]);
    assert_ne!(response.search_mode, FunctionSearchMode::Judge);
    let outcome = benchmark_installed(&deps, &["send an email message".into()]).await;
    assert!(!outcome.judge_complete);
    assert_eq!(outcome.judge_requests, 0);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_hub_without_a_provider_falls_back_like_an_absent_judge() {
    for error in [
        hub_error("provider_unavailable"),
        hub_error("missing_key"),
        json!({"status":"error","code":"deadline"}),
        json!("not an envelope"),
    ] {
        let (judge, requests) = mock_hub(0, move |_| Ok(error.clone()));
        let response = ask(&deps(judge), &["send an email message"]).await;
        assert_eq!(ids(&response), ["mail::send"]);
        assert_ne!(response.search_mode, FunctionSearchMode::Judge);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
    let (judge, _) = mock_hub(0, |_| Err(JudgeError::Unavailable));
    let response = ask(&deps(judge), &["send an email message"]).await;
    assert_eq!(ids(&response), ["mail::send"]);
}

#[cfg(minilm)]
async fn load_local_model(deps: &mut Deps) {
    let path = std::env::var("III_DIRECTORY_MINILM_MODEL_PATH")
        .expect("set III_DIRECTORY_MINILM_MODEL_PATH to the pinned local bundle");
    deps.semantic = SemanticSearch::new(Some(path.into()));
    let tools = deps.catalog.read().await.clone();
    let fingerprint = tool_fingerprint(&tools);
    deps.semantic.rebuild(tools);
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while deps
            .semantic
            .rank(&fingerprint, &["compose an email".into()], -1.0)
            .await
            .is_err()
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("local MiniLM index must become ready");
}

#[cfg(minilm)]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires III_DIRECTORY_MINILM_MODEL_PATH and the pinned ONNX runtime"]
async fn judge_failures_use_the_available_hybrid_ranking() {
    let mut deps = deps(scoring(0.9).0);
    load_local_model(&mut deps).await;
    let query = ["compose an email".into()];
    let mut cfg = (**deps.config.load()).clone();
    cfg.function_search_mode = FunctionSearchMode::Hybrid;
    deps.config.store(Arc::new(cfg.clone()));
    let hybrid = benchmark_installed(&deps, &query).await;
    assert!(hybrid.hybrid_complete);
    assert!(hybrid.selected.contains(&"mail::send".into()));
    cfg.function_search_mode = FunctionSearchMode::Lexical;
    deps.config.store(Arc::new(cfg.clone()));
    assert!(benchmark_installed(&deps, &query).await.selected.is_empty());
    cfg.function_search_mode = FunctionSearchMode::Judge;
    cfg.function_search_judge_timeout_ms = 40;
    deps.config.store(Arc::new(cfg));
    for (delay, response) in [
        (0, Some(hub_error("http"))),
        (0, Some(hub_error("transport"))),
        (0, Some(hub_error("provider_unavailable"))),
        (
            0,
            Some(json!({"status":"ok","model":"jev-1.13.0","results":{},"stats":{}})),
        ),
        (200, None),
    ] {
        deps.judge = mock_hub(delay, move |request| {
            Ok(response.clone().unwrap_or_else(|| reply(request, 0.9)))
        })
        .0;
        let fallback = benchmark_installed(&deps, &query).await;
        assert!(!fallback.judge_complete);
        assert!(fallback.hybrid_complete);
        assert_eq!(fallback.selected, hybrid.selected);
        assert_eq!(fallback.rankings, hybrid.rankings);
    }
    deps.judge = JudgeSearch::default();
    let absent = benchmark_installed(&deps, &query).await;
    assert!(!absent.judge_complete);
    assert!(absent.hybrid_complete);
    assert_eq!(absent.selected, hybrid.selected);

    deps.judge = scoring(0.1).0;
    let no_match = benchmark_installed(&deps, &query).await;
    assert!(no_match.judge_complete);
    assert!(!no_match.hybrid_complete);
    assert!(
        no_match.selected.is_empty(),
        "valid no-match must not fall back"
    );

    // A stale local index must not be used for a changed catalog after the judge fails.
    deps.judge = mock_hub(0, |_| Ok(hub_error("http"))).0;
    let mut next = tools();
    next[0].name = "mail::deliver".into();
    *deps.catalog.write().await = Arc::new(next);
    let stale = benchmark_installed(&deps, &["send an email message".into()]).await;
    assert!(!stale.judge_complete);
    assert!(!stale.hybrid_complete);
    assert_eq!(stale.selected, ["mail::deliver"]);
}

#[cfg(minilm)]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires III_DIRECTORY_MINILM_MODEL_PATH and the pinned ONNX runtime"]
async fn registry_judge_failure_uses_the_available_hybrid_ranking() {
    let registry = MockServer::start().await;
    registry_fixture(&registry).await;
    let (judge, requests) = mock_hub(0, |_| Ok(hub_error("http")));
    let mut deps = deps(judge);
    load_local_model(&mut deps).await;
    let mut cfg = (**deps.config.load()).clone();
    cfg.registry_search = true;
    cfg.registry_url = registry.uri();
    deps.config.store(Arc::new(cfg.clone()));
    let fallback = ask(&deps, &["compose an email"]).await;
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(ids(&fallback).contains(&"mail::send"));
    assert_eq!(fallback.installable.len(), 1);
    assert_eq!(fallback.installable[0].name, "courier");
    assert_eq!(fallback.installable[0].version, "1.2.3");
    assert_eq!(
        fallback.installable[0].functions.len(),
        MAX_INSTALLABLE_FUNCTIONS
    );
    cfg.function_search_mode = FunctionSearchMode::Hybrid;
    deps.config.store(Arc::new(cfg.clone()));
    let hybrid = ask(&deps, &["compose an email"]).await;
    assert_eq!(
        serde_json::to_value(&fallback.installable).unwrap(),
        serde_json::to_value(&hybrid.installable).unwrap()
    );
    cfg.function_search_mode = FunctionSearchMode::Lexical;
    deps.config.store(Arc::new(cfg));
    assert!(ask(&deps, &["compose an email"])
        .await
        .installable
        .is_empty());
}

#[tokio::test]
async fn exact_ids_and_intrinsic_capabilities_need_no_remote_call() {
    let (judge, requests) = scoring(0.9);
    let deps = deps(judge);
    assert_eq!(ids(&ask(&deps, &["mail::send"]).await), ["mail::send"]);
    assert!(ask(&deps, &["summarize provided text"])
        .await
        .workers
        .is_empty());
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn exact_lane_survives_a_remote_no_match_for_the_other_capability() {
    let (judge, requests) = scoring(0.1);
    let response = ask(&deps(judge), &["mail::send", "dispatch correspondence"]).await;
    assert_eq!(ids(&response), ["mail::send"]);
    let bodies = bodies(&requests);
    assert_eq!(bodies.len(), 1);
    let capabilities = bodies[0]["state"]["capabilities"].as_object().unwrap();
    assert_eq!(capabilities.len(), 1);
    assert_eq!(
        capabilities.values().next().unwrap(),
        "dispatch correspondence"
    );
}

#[tokio::test]
async fn an_unavailable_judge_preserves_lexical_search() {
    let mut deps = deps(scoring(0.9).0);
    deps.judge = JudgeSearch::default();
    assert_eq!(
        ids(&ask(&deps, &["send an email message"]).await),
        ["mail::send"]
    );
}

#[tokio::test]
async fn timeout_budget_is_shared_across_capability_batches() {
    let (judge, requests) = mock_hub(200, |request| Ok(reply(request, 0.9)));
    let deps = deps(judge);
    let mut cfg = (**deps.config.load()).clone();
    cfg.function_search_judge_timeout_ms = 40;
    deps.config.store(Arc::new(cfg));
    let response = ask(&deps, &["send an email message"; 18]).await;
    assert_eq!(ids(&response), ["mail::send"]);
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn judge_keeps_multiple_capabilities_within_existing_caps() {
    let deps = deps(scoring(0.8).0);
    let tools: Vec<ToolSchema> = (0..24)
        .map(|i| ToolSchema {
            name: format!("worker{i}::act"),
            description: "Perform an action.".into(),
            parameters: json!({"type":"object"}),
        })
        .chain(tools().into_iter().skip(4))
        .collect();
    *deps.catalog.write().await = Arc::new(tools);
    let response = ask(&deps, &["perform an action"; 6]).await;
    assert_eq!(ids(&response).len(), MAX_SEARCH_FUNCTIONS);
    assert_eq!(response.workers.len(), 12);
}

#[tokio::test]
async fn judge_session_suppression_and_catalog_invalidation_still_work() {
    use opentelemetry::baggage::BaggageExt;
    use opentelemetry::{Context, KeyValue};
    let deps = deps(scoring(0.9).0);
    let context =
        Context::current().with_baggage([KeyValue::new(SESSION_BAGGAGE_KEY, "judge-test-session")]);
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
async fn judge_registry_candidates_preserve_owners_and_limits() {
    let registry = MockServer::start().await;
    registry_fixture(&registry).await;
    let (judge, requests) = scoring(0.9);
    let deps = deps(judge);
    let mut cfg = (**deps.config.load()).clone();
    cfg.registry_search = true;
    cfg.registry_url = registry.uri();
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
    let bodies = bodies(&requests);
    assert_eq!(bodies.len(), 2);
    assert!(bodies
        .iter()
        .all(|body| !body.to_string().contains("courier::private")));
}

#[tokio::test]
async fn registry_judge_failure_falls_back_to_its_lexical_pool() {
    let registry = MockServer::start().await;
    registry_fixture(&registry).await;
    let (judge, requests) = mock_hub(0, |_| Ok(hub_error("http")));
    let deps = deps(judge);
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
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn registry_judge_failure_preserves_an_exact_lane_among_lexical_matches() {
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
    let (judge, requests) = mock_hub(0, |_| Ok(hub_error("http")));
    let deps = deps(judge);
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
            &deps.judge,
            tokio::time::Instant::now() + std::time::Duration::from_secs(3),
        )),
    )
    .await;
    assert_eq!(workers[0].functions.len(), MAX_INSTALLABLE_FUNCTIONS);
    assert!(workers[0].functions.iter().any(|f| f.function_id == "a::b"));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn mode_reload_waits_for_an_in_progress_hybrid_activation() {
    use crate::configuration::{apply_config, SharedState};
    use crate::functions::skills::RegisteredWorkersCache;
    let deps = deps(JudgeSearch::default());
    deps.semantic.set_enabled(false);
    let mut initial = (**deps.config.load()).clone();
    initial.function_search_mode = FunctionSearchMode::Lexical;
    deps.config.store(Arc::new(initial));
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
    let mut judge_cfg = cfg;
    judge_cfg.function_search_mode = FunctionSearchMode::Judge;
    let mut judge = tokio::spawn(async move { apply_config(&state, judge_cfg).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut judge)
            .await
            .is_err()
    );
    drop(catalog_guard);
    hybrid.await.unwrap();
    judge.await.unwrap();
    assert_eq!(
        deps.config.load().function_search_mode,
        FunctionSearchMode::Judge
    );
    let mut next = tools();
    next[0].description = "Later catalog while the judge is active.".into();
    activate_catalog(&deps.catalog, &deps.semantic, next).await;
    assert_eq!(
        deps.semantic.requested_fingerprint(),
        Some(tool_fingerprint(&deps.catalog.read().await))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_refreshes_keep_the_requested_index_on_the_current_catalog() {
    let deps = deps(JudgeSearch::default());
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
async fn switching_from_lexical_to_judge_prepares_hybrid_and_keeps_the_index_fresh() {
    use crate::configuration::{apply_config, SharedState};
    use crate::functions::skills::RegisteredWorkersCache;
    let deps = deps(JudgeSearch::default());
    deps.semantic.set_enabled(false);
    let mut initial = (**deps.config.load()).clone();
    initial.function_search_mode = FunctionSearchMode::Lexical;
    deps.config.store(Arc::new(initial));
    // Refreshing the catalog in Lexical mode still publishes its new entries.
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
    let mut judge = cfg.clone();
    judge.function_search_mode = FunctionSearchMode::Judge;
    apply_config(&state, judge).await;
    let fingerprint = tool_fingerprint(&deps.catalog.read().await);
    assert_eq!(
        deps.semantic.requested_fingerprint().as_deref(),
        Some(fingerprint.as_str())
    );
    let mut next = tools();
    next[0].description = "A later catalog.".into();
    assert!(activate_catalog(&deps.catalog, &deps.semantic, next).await);
    assert_eq!(
        deps.semantic.requested_fingerprint(),
        Some(tool_fingerprint(&deps.catalog.read().await)),
        "judge catalog refresh must keep its hybrid fallback current"
    );
    apply_config(&state, cfg).await;
    let previous = deps.semantic.requested_fingerprint();
    activate_catalog(&deps.catalog, &deps.semantic, tools()).await;
    assert_eq!(
        deps.semantic.requested_fingerprint(),
        previous,
        "Lexical must disable local rebuilds"
    );
}

#[tokio::test]
async fn benchmark_keeps_intrinsic_and_exact_lanes_aligned() {
    let (judge, requests) = scoring(0.9);
    let deps = deps(judge);
    let capabilities = vec!["summarize provided text".into(), "mail::send".into()];
    let result = benchmark_installed(&deps, &capabilities).await;
    assert!(result.rankings[0].is_empty());
    assert_eq!(result.rankings[1][0].0, "mail::send");
    assert_eq!(result.selected, ["mail::send"]);
    assert_eq!(result.judge_requests, 0);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn benchmark_preserves_known_usage_and_stage_time_after_a_later_block_fails() {
    let (judge, requests) = mock_hub(30, |request| {
        let functions = request.evaluations[0].state["functions"]
            .as_object()
            .unwrap();
        Ok(if functions.len() == 16 {
            reply(request, 0.9)
        } else {
            hub_error("http")
        })
    });
    let deps = deps(judge);
    *deps.catalog.write().await = Arc::new(
        (0..17)
            .map(|i| ToolSchema {
                name: format!("mail::send{i:02}"),
                description: "Send an email message.".into(),
                parameters: json!({"type":"object"}),
            })
            .chain(tools().into_iter().skip(4))
            .collect(),
    );
    let result = benchmark_installed(&deps, &["send an email message".into()]).await;
    assert!(!result.judge_complete);
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(result.judge_requests, 1);
    assert_eq!(result.judge_questions, 16);
    assert_eq!(result.input_tokens, 100);
    assert_eq!(result.output_tokens, 10);
    assert!(result.judge_elapsed_ms >= 30);
    assert!(
        !result.selected.is_empty(),
        "lexical fallback still returns candidates"
    );
}

fn skills_root(files: &[(&str, &str)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = root.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    root
}

const COMPOSE_SKILL: &str = "---\ndescription: How to compose and send a message with mail::send.\n---\n# Compose mail\n\nSteps.\n";

#[tokio::test]
async fn judge_lists_installed_skills_matching_the_capabilities() {
    let (judge, requests) = scoring(0.9);
    let root = skills_root(&[
        ("skills/mail/compose.md", COMPOSE_SKILL),
        // No `browser::*` function is installed: the skill is not a candidate.
        (
            "skills/browser/scrape.md",
            "---\ndescription: Scrape a page.\n---\n# Scrape\n\nSteps.\n",
        ),
    ]);
    let response = ask(
        &deps_with_skill_root(judge, root.path()),
        &["dispatch correspondence"],
    )
    .await;
    assert_eq!(ids(&response), ["mail::send", "state::get"]);
    assert_eq!(response.search_mode, FunctionSearchMode::Judge);
    let skills: Vec<(&str, &str, &str)> = response
        .skills
        .iter()
        .map(|skill| {
            (
                skill.id.as_str(),
                skill.title.as_str(),
                skill.description.as_str(),
            )
        })
        .collect();
    assert_eq!(
        skills,
        [(
            "mail/compose",
            "Compose mail",
            "How to compose and send a message with mail::send."
        )]
    );
    assert!(response
        .guidance
        .contains("directory::skills::get { \"id\": \"<id>\" }"));
    let bodies = bodies(&requests);
    let skill_block = bodies
        .iter()
        .find(|body| body["state"]["skills"].is_object())
        .expect("one skill evaluation was sent");
    assert_eq!(
        skill_block["state"]["skills"]["f0"]["skill_id"],
        "mail/compose"
    );
    assert_eq!(
        skill_block["state"]["skills"]["f0"]["description"],
        "Compose mail: How to compose and send a message with mail::send."
    );
    assert!(skill_block["state"].get("functions").is_none());
    assert!(skill_block["questions"]["c0_f0"]["instructions"]
        .as_str()
        .unwrap()
        .starts_with("Does the skill document described in state.skills.f0"));
    // The function evaluation is untouched by the skills corpus.
    let function_block = bodies
        .iter()
        .find(|body| body["state"]["functions"].is_object())
        .expect("one function evaluation was sent");
    assert!(function_block["state"].get("skills").is_none());
}

#[tokio::test]
async fn skills_below_the_relevance_threshold_are_omitted() {
    let (judge, _) = mock_hub(0, |request| {
        Ok(reply(
            request,
            if is_skill_block(request) { 0.1 } else { 0.9 },
        ))
    });
    let root = skills_root(&[("skills/mail/compose.md", COMPOSE_SKILL)]);
    let response = ask(
        &deps_with_skill_root(judge, root.path()),
        &["dispatch correspondence"],
    )
    .await;
    assert_eq!(ids(&response), ["mail::send", "state::get"]);
    assert!(response.skills.is_empty());
    assert!(!response.guidance.contains("`skills` entries"));
}

#[tokio::test]
async fn a_failed_skill_evaluation_keeps_the_function_results() {
    let (judge, _) = mock_hub(0, |request| {
        Ok(if is_skill_block(request) {
            hub_error("http")
        } else {
            reply(request, 0.9)
        })
    });
    let root = skills_root(&[("skills/mail/compose.md", COMPOSE_SKILL)]);
    let response = ask(
        &deps_with_skill_root(judge, root.path()),
        &["dispatch correspondence"],
    )
    .await;
    assert_eq!(ids(&response), ["mail::send", "state::get"]);
    assert!(response.skills.is_empty());
}

#[tokio::test]
async fn lexical_mode_ranks_skills_without_calling_the_judge() {
    // Lexical mode ranks the skills locally with BM25: the judge is never called.
    let (judge, requests) = scoring(0.9);
    let root = skills_root(&[("skills/mail/compose.md", COMPOSE_SKILL)]);
    let mut deps = deps_with_skill_root(judge, root.path());
    deps.config = SkillsConfig {
        function_search_mode: FunctionSearchMode::Lexical,
        filter_unregistered: false,
        ..skill_roots(root.path())
    }
    .into_shared();
    let response = ask(&deps, &["send an email message"]).await;
    assert_eq!(ids(&response), ["mail::send"]);
    assert_eq!(response.search_mode, FunctionSearchMode::Lexical);
    let skill_ids: Vec<&str> = response.skills.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(skill_ids, ["mail/compose"]);
    assert!(response.guidance.contains("`skills` entries"));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn judge_ranks_registered_triggers_through_the_triggers_corpus() {
    let (judge, requests) = scoring(0.9);
    let deps = deps(judge);
    let docs = trigger_docs(&json!({ "registered_triggers": [
        { "id": "t-1", "trigger_type": "cron", "function_id": "harness::sweep-pending",
          "worker_name": "harness", "config": { "expression": "0 0 0 * * *" } },
        { "id": "t-2", "trigger_type": "console:style", "function_id": "state::ui-content" },
    ] }));
    let ranked = side_lane(
        &deps,
        &deps.config.load_full(),
        &["run a job every night".to_string()],
        Some(tokio::time::Instant::now() + std::time::Duration::from_secs(5)),
        docs,
        JudgeCorpus::Triggers,
    )
    .await;
    let ids: Vec<&str> = ranked.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["t-1"]);
    assert_eq!(ranked[0].function_id, "harness::sweep-pending");
    let bodies = bodies(&requests);
    assert_eq!(bodies.len(), 1);
    let body = &bodies[0];
    assert!(body["state"]["triggers"]["f0"]["description"]
        .as_str()
        .unwrap()
        .starts_with("cron trigger runs harness::sweep-pending"));
    assert!(body["state"]["triggers"]["f0"].get("trigger_id").is_none());
    assert!(body["state"].get("functions").is_none());
    assert!(body["questions"]["c0_f0"]["instructions"]
        .as_str()
        .unwrap()
        .contains("state.triggers.f0"));
}

#[tokio::test]
async fn a_judge_ranked_side_lane_keeps_search_mode_at_judge_when_functions_fell_back() {
    let (judge, _) = mock_hub(0, |request| {
        Ok(if is_skill_block(request) {
            reply(request, 0.9)
        } else {
            hub_error("http")
        })
    });
    let root = skills_root(&[("skills/mail/compose.md", COMPOSE_SKILL)]);
    let response = ask(
        &deps_with_skill_root(judge, root.path()),
        &["send an email message"],
    )
    .await;
    // Functions fell back to BM25 (no local model in tests); the skill was
    // still judge-ranked, and the response says so.
    assert_eq!(ids(&response), ["mail::send"]);
    let skill_ids: Vec<&str> = response.skills.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(skill_ids, ["mail/compose"]);
    assert_eq!(response.search_mode, FunctionSearchMode::Judge);
}
