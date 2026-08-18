//! Deterministic relevance tests for `reflex::discover` over a frozen
//! snapshot of the live catalog (250 functions with their real
//! descriptions and request schemas, captured 2026-08-18 via
//! `engine::functions::info`). bm25 — the shipped
//! default — is purely lexical, so no engine, no model, and no judge are
//! involved: expectations pin exact function sets and run in
//! milliseconds. If a rank change breaks one of these, either the scorer
//! regressed or the expectation needs a *reviewed* update — never loosen
//! an assertion to make a run green without reading the new result.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::config::DiscoveryConfig;
use crate::functions::{search_functions, Deps, SearchFunctionsRequest, SearchFunctionsResponse};
use crate::search::ToolSchema;

const CATALOG_FIXTURE: &str = include_str!("../tests/fixtures/discover_catalog.json");

fn fixture_catalog() -> Vec<ToolSchema> {
    let entries: Vec<Value> = serde_json::from_str(CATALOG_FIXTURE).expect("fixture parses");
    let catalog: Vec<ToolSchema> = entries
        .iter()
        .map(|entry| ToolSchema {
            name: entry["name"].as_str().expect("fixture name").to_string(),
            description: entry["description"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            parameters: entry["parameters"].clone(),
        })
        .collect();
    assert!(catalog.len() >= 200, "fixture lost its catalog");
    catalog
}

fn fixture_deps() -> Deps {
    Deps {
        config: Arc::new(RwLock::new(DiscoveryConfig::default())),
        catalog: Arc::new(RwLock::new(Arc::new(fixture_catalog()))),
        sessions: Arc::default(),
        iii: None,
    }
}

async fn ask(deps: &Deps, query: &str) -> SearchFunctionsResponse {
    search_functions(
        deps,
        SearchFunctionsRequest {
            query: query.into(),
        },
    )
    .await
    .expect("search succeeds")
}

fn workers(response: &SearchFunctionsResponse) -> Vec<&str> {
    response
        .workers
        .iter()
        .map(|worker| worker.namespace.as_str())
        .collect()
}

fn function_ids(response: &SearchFunctionsResponse) -> Vec<&str> {
    response
        .workers
        .iter()
        .flat_map(|worker| worker.functions.iter().map(|f| f.function_id.as_str()))
        .collect()
}

#[tokio::test]
async fn state_persistence_query_returns_set_and_get() {
    let deps = fixture_deps();
    let response = ask(
        &deps,
        "store a value under a key in the state scope and read it back",
    )
    .await;
    let ids = function_ids(&response);
    assert_eq!(workers(&response)[0], "state", "ids: {ids:?}");
    assert!(ids.contains(&"state::set"), "ids: {ids:?}");
    assert!(ids.contains(&"state::get"), "ids: {ids:?}");
}

#[tokio::test]
async fn shell_command_query_returns_exec() {
    let deps = fixture_deps();
    let response = ask(&deps, "run a shell command on this machine").await;
    let ids = function_ids(&response);
    assert_eq!(workers(&response)[0], "shell", "ids: {ids:?}");
    assert!(ids.contains(&"shell::exec"), "ids: {ids:?}");
}

#[tokio::test]
async fn github_repository_query_returns_repo_view() {
    let deps = fixture_deps();
    let response = ask(&deps, "check the stargazers count of a github repository").await;
    let ids = function_ids(&response);
    assert_eq!(workers(&response)[0], "github", "ids: {ids:?}");
    // "stargazers" appears in no request contract (it is response-shape
    // vocabulary), so finding the repo via search or viewing it are both
    // correct lexical resolutions.
    assert!(
        ids.contains(&"github::repo::view") || ids.contains(&"github::search::repos"),
        "ids: {ids:?}"
    );
}

#[tokio::test]
async fn issue_comment_query_returns_issue_comment() {
    let deps = fixture_deps();
    let response = ask(&deps, "comment on an open github issue").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"github::issue::comment"), "ids: {ids:?}");
}

#[tokio::test]
async fn web_page_query_returns_web_fetch() {
    let deps = fixture_deps();
    let response = ask(&deps, "fetch the content of a web page by url").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"web::fetch"), "ids: {ids:?}");
}

#[tokio::test]
async fn python_code_query_returns_code_runner_run() {
    let deps = fixture_deps();
    let response = ask(&deps, "execute a snippet of python code").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"code-runner::run"), "ids: {ids:?}");
}

#[tokio::test]
async fn storage_upload_query_returns_put_object() {
    let deps = fixture_deps();
    let response = ask(&deps, "upload an object into the storage bucket").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"storage::putObject"), "ids: {ids:?}");
}

#[tokio::test]
async fn database_schema_query_returns_describe_functions() {
    let deps = fixture_deps();
    let response = ask(&deps, "describe the database schema and its tables").await;
    let ids = function_ids(&response);
    assert_eq!(workers(&response)[0], "database", "ids: {ids:?}");
    assert!(
        ids.contains(&"database::describeSchema") || ids.contains(&"database::describeTable"),
        "ids: {ids:?}"
    );
}

#[tokio::test]
async fn screenshot_query_returns_browser_screenshot() {
    let deps = fixture_deps();
    let response = ask(&deps, "take a screenshot of the current page").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"browser::screenshot"), "ids: {ids:?}");
}

#[tokio::test]
async fn gibberish_query_returns_refine_guidance() {
    let deps = fixture_deps();
    let response = ask(&deps, "zzzz qqqq wwww").await;
    assert!(response.workers.is_empty());
    assert!(
        response.guidance.contains("No functions matched"),
        "guidance: {}",
        response.guidance
    );
}

#[tokio::test]
async fn engine_and_reflex_never_appear_in_results() {
    let deps = fixture_deps();
    for query in [
        "list every available function on the engine",
        "enqueue a message onto a queue topic",
        "route my objective to the right worker",
    ] {
        let response = ask(&deps, query).await;
        for id in function_ids(&response) {
            assert!(
                !id.starts_with("engine::") && !id.starts_with("reflex::"),
                "query {query:?} leaked {id}"
            );
        }
    }
}

#[tokio::test]
async fn results_respect_worker_and_function_caps() {
    let deps = fixture_deps();
    let response = ask(
        &deps,
        "read write delete list get set update files values objects",
    )
    .await;
    assert!(
        response.workers.len() <= 3,
        "workers: {:?}",
        workers(&response)
    );
    assert!(function_ids(&response).len() <= 12);
}

#[tokio::test]
async fn clear_leader_queries_do_not_drag_config_handlers() {
    let deps = fixture_deps();
    let response = ask(&deps, "check the stargazers count of a github repository").await;
    for id in function_ids(&response) {
        assert!(
            !id.ends_with("on-config-change"),
            "config handler rode along: {id}"
        );
    }
}

#[tokio::test]
async fn same_query_is_deterministic_across_fresh_deps() {
    let first = ask(
        &fixture_deps(),
        "persist a value under a key and read it back later",
    )
    .await;
    let second = ask(
        &fixture_deps(),
        "persist a value under a key and read it back later",
    )
    .await;
    assert_eq!(
        serde_json::to_value(&first.workers).unwrap(),
        serde_json::to_value(&second.workers).unwrap()
    );
}

#[tokio::test]
async fn repeat_query_in_one_session_omits_delivered_contracts() {
    use opentelemetry::baggage::BaggageExt;
    use opentelemetry::{Context, KeyValue};
    let deps = fixture_deps();
    let context =
        Context::current_with_baggage(vec![KeyValue::new("iii.session.id", "relevance-session")]);
    let _guard = context.attach();
    let first = ask(&deps, "persist a value under a key and read it back later").await;
    let first_ids: Vec<&str> = function_ids(&first);
    assert!(first_ids.contains(&"state::set"));
    // A different query overlapping the first: overlapping contracts are
    // omitted and named in the guidance instead.
    let second = ask(&deps, "update the persisted value and list the state keys").await;
    for id in function_ids(&second) {
        assert!(
            !first_ids.contains(&id),
            "repeat query re-sent an already delivered contract: {id}"
        );
    }
    assert!(
        second
            .guidance
            .contains("Already provided earlier in this session"),
        "guidance: {}",
        second.guidance
    );
    // An IDENTICAL query selects only already-delivered ids: the all-repeat
    // path re-sends the full contracts (compaction recovery), no omission.
    let third = ask(&deps, "persist a value under a key and read it back later").await;
    assert!(
        function_ids(&third).contains(&"state::set"),
        "all-repeat query must re-send full contracts: {:?}",
        function_ids(&third)
    );
    assert!(!third
        .guidance
        .contains("Already provided earlier in this session"));
}

#[tokio::test]
async fn guidance_carries_the_override_and_requery_contract() {
    let deps = fixture_deps();
    let response = ask(&deps, "run a shell command on this machine").await;
    assert!(response.guidance.contains("OVERRIDES"));
    assert!(response
        .guidance
        .contains("discovery::search_functions again"));
    assert!(response.guidance.contains("agent_trigger"));
}

#[tokio::test]
#[ignore = "exploratory dump for precision tuning"]
async fn dump_probe_queries() {
    use crate::search::{canonical_tools, Bm25Index};
    let corpus = canonical_tools(&fixture_catalog());
    let index = Bm25Index::build(&corpus);
    for query in [
        "persist a value under a key and read it back later",
        "check the stargazers count of a github repository",
        "close a github issue",
        "kill a running process by pid",
        "list the files in a folder on the filesystem",
        "get the value",
        "send a message to a stream group",
        "read the browser console logs",
        "merge a pull request after checks pass",
        "create a new pull request",
    ] {
        let ranked = index.rank_with_matches(query);
        let leader = ranked.first().map(|(_, s, _)| *s).unwrap_or(0.0);
        let rows: Vec<String> = ranked
            .iter()
            .take(10)
            .map(|(id, s, m)| format!("{id}={:.0}%/m{m}", s / leader * 100.0))
            .collect();
        println!("Q: {query}\n   {rows:?}\n");
    }
    let deps = fixture_deps();
    for query in [
        "kill a running process by pid",
        "list the files in a directory",
        "close a github issue",
        "create a new pull request",
        "delete a stored value",
        "send a message to a stream group",
        "schedule a recurring job",
        "search for a text pattern in files",
        "start and stop a managed worker",
        "presign a temporary download url",
        "count the tokens in the context window",
        "get the value",
        "read the browser console logs",
        "merge a pull request after checks pass",
        "compare and set a state key atomically",
    ] {
        let response = ask(&deps, query).await;
        let ids = function_ids(&response);
        println!("Q: {query}\n   -> {} fns: {ids:?}\n", ids.len());
    }
}

// ---- precision battery: sharp queries must not drag whole families or
// ---- cross-worker tails. Added while tuning the scorer (camelCase split +
// ---- name-token boost + function floor); these pin the *desired* shape.

#[tokio::test]
async fn presign_url_query_finds_the_camel_cased_function() {
    let deps = fixture_deps();
    let response = ask(&deps, "presign a temporary download url").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"storage::presignUrl"), "ids: {ids:?}");
}

#[tokio::test]
async fn file_listing_query_returns_filesystem_listers_not_registries() {
    let deps = fixture_deps();
    // "directory" collides with the literal `directory` worker, which no
    // lexical scorer can untangle — real fs-listing intents phrase it as
    // folder/filesystem, which is what this pins.
    let response = ask(&deps, "list the files in a folder on the filesystem").await;
    let ids = function_ids(&response);
    assert!(
        ids.iter()
            .any(|id| id.ends_with("fs::ls") || id.ends_with("list-folder")),
        "no filesystem lister in: {ids:?}"
    );
    assert!(!ids.contains(&"state::list_keys"), "ids: {ids:?}");
}

#[tokio::test]
async fn close_issue_query_prunes_the_issue_family() {
    let deps = fixture_deps();
    let response = ask(&deps, "close a github issue").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"github::issue::close"), "ids: {ids:?}");
    assert!(!ids.contains(&"github::issue::create"), "ids: {ids:?}");
    assert!(!ids.contains(&"github::issue::list"), "ids: {ids:?}");
    assert!(ids.len() <= 4, "family rode along: {ids:?}");
}

#[tokio::test]
async fn create_pr_query_stays_within_github() {
    let deps = fixture_deps();
    let response = ask(&deps, "create a new pull request").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"github::pr::create"), "ids: {ids:?}");
    assert!(
        !ids.iter().any(|id| id.starts_with("directory::")),
        "directory rode along: {ids:?}"
    );
}

#[tokio::test]
async fn merge_pr_query_keeps_merge_and_checks() {
    let deps = fixture_deps();
    let response = ask(&deps, "merge a pull request after checks pass").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"github::pr::merge"), "ids: {ids:?}");
    assert!(ids.contains(&"github::pr::checks"), "ids: {ids:?}");
    assert!(!ids.contains(&"github::pr::create"), "ids: {ids:?}");
    assert!(ids.len() <= 6, "family rode along: {ids:?}");
}

#[tokio::test]
async fn console_logs_query_stays_tight() {
    let deps = fixture_deps();
    let response = ask(&deps, "read the browser console logs").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"browser::console::read"), "ids: {ids:?}");
    assert!(!ids.contains(&"browser::sessions::start"), "ids: {ids:?}");
    assert!(!ids.contains(&"browser::sessions::attach"), "ids: {ids:?}");
    assert!(ids.len() <= 6, "family rode along: {ids:?}");
}

#[tokio::test]
async fn generic_get_value_returns_getters_not_the_fp_family() {
    let deps = fixture_deps();
    let response = ask(&deps, "get the value").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"state::get"), "ids: {ids:?}");
    for tail in ["fp::drop", "fp::take", "fp::when", "fp::sortBy", "fp::nth"] {
        assert!(!ids.contains(&tail), "fp tail rode along: {ids:?}");
    }
}

#[tokio::test]
async fn stream_send_query_prunes_the_stream_family() {
    let deps = fixture_deps();
    let response = ask(&deps, "send a message to a stream group").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"stream::send"), "ids: {ids:?}");
    assert!(
        !ids.contains(&"iii::queue::redrive_message"),
        "ids: {ids:?}"
    );
    assert!(ids.len() <= 5, "family rode along: {ids:?}");
}

#[tokio::test]
async fn kill_process_query_skips_the_status_tail() {
    let deps = fixture_deps();
    let response = ask(&deps, "kill a running process by pid").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"shell::kill"), "ids: {ids:?}");
    // worker::status legitimately covers process/pid vocabulary the kill
    // contract lacks ("job"); pure lexical ranking cannot exclude it, so
    // pin the tail to that single semi-relevant survivor.
    assert!(ids.len() <= 2, "ids: {ids:?}");
}

#[tokio::test]
async fn compare_and_set_query_puts_the_atomic_function_first() {
    let deps = fixture_deps();
    let response = ask(&deps, "compare and set a state key atomically").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"state::compare-and-set"), "ids: {ids:?}");
    assert!(ids.len() <= 6, "family rode along: {ids:?}");
}

#[tokio::test]
async fn multi_intent_query_returns_every_clause_in_one_call() {
    let deps = fixture_deps();
    let response = ask(
        &deps,
        "register javascript functions on the engine bus, read and write persistent state values, and take a screenshot of the page",
    )
    .await;
    let ids = function_ids(&response);
    assert!(
        ids.contains(&"code-runner::register_function"),
        "ids: {ids:?}"
    );
    assert!(ids.contains(&"state::set"), "ids: {ids:?}");
    assert!(ids.contains(&"state::get"), "ids: {ids:?}");
    assert!(ids.contains(&"browser::screenshot"), "ids: {ids:?}");
    assert!(
        response.workers.len() <= 3,
        "workers: {:?}",
        workers(&response)
    );
}

#[tokio::test]
async fn todo_app_query_resolves_in_one_call() {
    let deps = fixture_deps();
    let response = ask(
        &deps,
        "register todo CRUD functions on the bus with the code runner, and read and write persistent todo state under a scope",
    )
    .await;
    let ids = function_ids(&response);
    assert!(
        ids.contains(&"code-runner::register_function"),
        "ids: {ids:?}"
    );
    assert!(ids.contains(&"state::set"), "ids: {ids:?}");
    assert!(ids.contains(&"state::get"), "ids: {ids:?}");
}

#[tokio::test]
async fn two_empty_results_widen_the_third_to_single_term_matches() {
    use opentelemetry::baggage::BaggageExt;
    use opentelemetry::{Context, KeyValue};
    let deps = fixture_deps();
    // Without a desperation streak, a single-term query stays empty.
    let cold = ask(&deps, "zzz presign qqq").await;
    assert!(cold.workers.is_empty(), "min-match must hold normally");
    let context =
        Context::current_with_baggage(vec![KeyValue::new("iii.session.id", "desperate-session")]);
    let _guard = context.attach();
    assert!(ask(&deps, "zzz qqq").await.workers.is_empty());
    assert!(ask(&deps, "xxx yyy").await.workers.is_empty());
    // Third consecutive would-be-empty answer widens to single-term matches.
    let widened = ask(&deps, "zzz presign qqq").await;
    let ids = function_ids(&widened);
    assert!(ids.contains(&"storage::presignUrl"), "ids: {ids:?}");
    // The widened (non-empty) answer resets the streak, so the next weak
    // query is strict — and empty — again.
    let strict_again = ask(&deps, "xxx presign yyy").await;
    assert!(
        strict_again.workers.is_empty(),
        "streak must reset after a non-empty answer"
    );
}
