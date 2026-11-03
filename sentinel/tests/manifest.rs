fn identity_is_read_inside_future(source: &str, function_id: &str) -> bool {
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    let registration = format!("iii.register_function({function_id},");
    let Some(start) = compact.find(&registration) else {
        return false;
    };
    let block = &compact[start..];
    let Some(end) = block.find(".description(") else {
        return false;
    };
    let block = &block[..end];
    let Some(future) = block.find("asyncmove{") else {
        return false;
    };
    let reads: Vec<_> = block.match_indices("get_baggage_entry").collect();
    reads.len() == 2
        && reads.iter().all(|(position, _)| *position > future)
        && block.contains("get_baggage_entry(\"iii.session.id\")")
        && block.contains("get_baggage_entry(\"iii.message.id\")")
        && !block.contains("tokio::spawn")
}

#[test]
fn source_proxy_reads_invocation_identity_inside_its_handler_future() {
    assert!(identity_is_read_inside_future(
        include_str!("../src/functions.rs"),
        "SOURCE_READ_ID"
    ));
}

#[test]
fn identity_guard_accepts_formatting_but_rejects_context_losing_reads() {
    let safe = "iii.register_function(\n SOURCE_READ_ID, handler(|request| { async move { get_baggage_entry(\"iii.session.id\"); get_baggage_entry(\"iii.message.id\"); } })).description(SOURCE_READ_DESC)";
    assert!(identity_is_read_inside_future(safe, "SOURCE_READ_ID"));
    let outside = safe.replace("async move { get_baggage_entry(\"iii.session.id\"); get_baggage_entry(\"iii.message.id\");", "get_baggage_entry(\"iii.session.id\"); get_baggage_entry(\"iii.message.id\"); async move {");
    assert!(!identity_is_read_inside_future(&outside, "SOURCE_READ_ID"));
    assert!(!identity_is_read_inside_future(
        &safe.replace("async move {", "async move { tokio::spawn("),
        "SOURCE_READ_ID"
    ));
}

#[test]
fn live_e2e_guard_precedes_client_creation_and_connection() {
    let source = include_str!("e2e_ingest.rs");
    let connect = &source[source.find("async fn connect(").unwrap()..];
    assert!(
        connect.find("if !live_e2e_opted_in()").unwrap()
            < connect.find("register_worker(").unwrap(),
        "opt-in is checked before any client exists"
    );
    let test = &source[source
        .find("async fn a_real_failure_becomes_a_group_attributed_to_the_worker_that_owns_it()")
        .unwrap()..];
    assert!(
        test.find("if !live_e2e_opted_in()").unwrap()
            < test.find("connect(\"sentinel-e2e\")").unwrap(),
        "default cargo test must not connect to a live engine"
    );
}

use sentinel::{manifest, WorkerConfig};

#[test]
fn manifest_builder_emits_registry_metadata_without_a_binary() {
    let built = manifest::build_manifest();
    let value = serde_json::to_value(&built).expect("serialize worker manifest");

    assert_eq!(value["name"], "sentinel");
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["description"], manifest::DESCRIPTION);
    assert_eq!(value["default_config"]["database"], "primary");
    assert_eq!(value["default_config"]["investigation"]["model"], "");
    assert_eq!(value["default_config"]["projects"], serde_json::json!([]));
    assert!(value["supported_targets"]
        .as_array()
        .is_some_and(|targets| targets.len() == 1));
    assert!(value["supported_targets"][0]
        .as_str()
        .is_some_and(|target| !target.is_empty()));
}

#[test]
fn the_public_manifest_ships_the_same_defaults_the_binary_seeds() {
    let source = include_str!("../iii.worker.yaml");
    let document: serde_yaml::Value = serde_yaml::from_str(source).expect("iii.worker.yaml parses");

    assert_eq!(document["name"].as_str(), Some("sentinel"));
    assert_eq!(document["bin"].as_str(), Some("sentinel"));
    assert_eq!(
        document["description"].as_str(),
        Some(manifest::DESCRIPTION)
    );
    assert!(
        document["tags"]
            .as_sequence()
            .is_some_and(|tags| !tags.is_empty()),
        "discovery tags are required for interface capture"
    );

    // The published defaults and the seeded defaults are the same document:
    // an operator reading the manifest sees what the worker actually starts
    // with, and a drift between the two fails here rather than in the field.
    let published: WorkerConfig = serde_yaml::from_value(document["config"].clone())
        .expect("the published config block parses as WorkerConfig");
    assert_eq!(published, WorkerConfig::default());
    published.validate().expect("published defaults validate");
}

#[test]
fn every_declared_dependency_is_one_this_worker_actually_calls() {
    let source = include_str!("../iii.worker.yaml");
    for dependency in [
        "database",
        "queue",
        "configuration",
        "cron",
        "harness",
        "session-manager",
        "ide",
    ] {
        assert!(
            source
                .lines()
                .any(|line| line == format!("  {dependency}: \"latest\"")),
            "{dependency} must be declared"
        );
    }
}

#[test]
fn manifest_subcommand_emits_valid_json_without_connecting_to_iii() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_sentinel"))
        .arg("--manifest")
        .output()
        .expect("spawn sentinel --manifest");
    assert!(
        output.status.success(),
        "binary exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("manifest stdout is JSON");
    assert_eq!(manifest["name"], "sentinel");
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(manifest["default_config"].is_object());
    assert!(manifest["supported_targets"]
        .as_array()
        .is_some_and(|targets| !targets.is_empty()));
}

#[test]
fn the_interface_registers_before_the_durable_dependencies_are_claimed() {
    let source = include_str!("../src/main.rs");
    let claim = source
        .find("dependencies::claim(")
        .expect("the durable dependencies are claimed");

    for registration in [
        "events::register_trigger_types(&iii, &subscribers)",
        "functions::register_all(&iii, &deps)",
        "configuration::bind_reload(&iii, cell.clone(), error_cell.clone())",
    ] {
        let position = source
            .find(registration)
            .unwrap_or_else(|| panic!("{registration} is part of boot"));
        assert!(
            position < claim,
            "{registration} must precede the durable dependency claim: interface capture boots \
             this binary against an engine with no database and no queue"
        );
    }
}

#[test]
fn the_caller_identity_is_read_inside_the_handler_future() {
    // The SDK evaluates `handler(request)` and *then* wraps the future it
    // returned with the invocation's OTel context
    // (`iii-sdk-0.23.0/src/iii.rs:2327`). A baggage read in the closure body
    // therefore runs outside that context and finds nothing — which showed up
    // on a live stack as an agent calling `record` six times in a row and
    // being told each time that its call carried no session.
    assert!(
        identity_is_read_inside_future(include_str!("../src/functions.rs"), "DIAGNOSIS_RECORD_ID"),
        "caller identity must be read inside the handler future without spawning away its context"
    );
}

#[test]
fn the_worker_never_logs_at_error_level() {
    // An ERROR log from this worker would come straight back through its own
    // `log` trigger and ingest itself. Internal failures are WARN plus a
    // counter in `sentinel::status`.
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .expect("read the source directory")
    {
        let path = entry.expect("read a source entry").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path).expect("read a source file");
            assert!(
                !source.contains("tracing::error!"),
                "{} emits tracing::error!, which the worker would ingest as its own failure",
                path.display()
            );
        }
    }
}
