//! The scenario run loop (spec § Scenario lifecycle): Allocate → Boot →
//! Probe → Arm → Send → Await → Collect → Grade → Report → Teardown.
//!
//! Classification ownership: fixture defects are `runner_error`; anything
//! that fails before Send is `setup_error` (early process exit outranks it
//! as `process_crash`); after Send, a blown scenario deadline is `timeout`,
//! runner-owned collection faults are `runner_error`, and everything graded
//! is `pass`/`contract_failure`. Precedence lives in
//! `Classification::combine`.

use std::path::Path;

use serde_json::{json, Value};

use crate::artifacts::{scenario_file, trim_passing_run, write_json};
use crate::client::Client;
use crate::expand::Placeholders;
use crate::fixtures::ScenarioFixture;
use crate::grader::{self, Evidence};
use crate::readiness::{self, ReadinessSpec};
use crate::recorder::Recorder;
use crate::scripted_router::ScriptedRouter;
use crate::stack::{
    expected_config_entries, expected_harness_config_entry, RunPaths, Stack, StackBins,
};
use crate::types::scenario::{Classification, ConformanceResultV1, ConformanceScenarioV1};
use crate::types::script::{RouterScriptV1, SchemaVersion1};

pub struct RunOutcome {
    pub result: ConformanceResultV1,
    pub run_root: std::path::PathBuf,
}

pub async fn run_scenario(
    bins: &StackBins,
    fixture: &ScenarioFixture,
    artifacts_dir: &Path,
    retain_success: bool,
) -> RunOutcome {
    let started_at = now_rfc3339();
    let started = std::time::Instant::now();
    let run_id = format!("cr{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let session_id = format!("s_{}", uuid::Uuid::new_v4().simple());

    let mut runner = ScenarioRunner {
        bins,
        fixture,
        retain_success,
        run_id: run_id.clone(),
        session_id,
        artifacts: Vec::new(),
        invariants: Vec::new(),
    };

    let (classification, run_root) = match runner.execute(artifacts_dir).await {
        Ok((classification, run_root)) => (classification, run_root),
        Err(fatal) => {
            tracing::error!(scenario = %fixture.scenario.id, "runner fault: {fatal:#}");
            (Classification::RunnerError, artifacts_dir.join(&run_id))
        }
    };

    let result = ConformanceResultV1 {
        schema_version: SchemaVersion1::V1,
        run_id,
        scenario_id: fixture.scenario.id.clone(),
        classification,
        invariants: runner.invariants,
        artifacts: runner.artifacts,
        started_at,
        duration_ms: started.elapsed().as_millis() as u64,
    };
    if let Ok(value) = serde_json::to_value(&result) {
        let _ = write_json(&run_root, &run_root.join("result.json"), &value);
    }
    if classification == Classification::Pass && !retain_success {
        trim_passing_run(&run_root);
    }
    RunOutcome { result, run_root }
}

struct ScenarioRunner<'a> {
    bins: &'a StackBins,
    fixture: &'a ScenarioFixture,
    #[allow(dead_code)]
    retain_success: bool,
    run_id: String,
    session_id: String,
    artifacts: Vec<String>,
    invariants: Vec<crate::types::scenario::InvariantResultV1>,
}

impl ScenarioRunner<'_> {
    async fn execute(
        &mut self,
        artifacts_dir: &Path,
    ) -> anyhow::Result<(Classification, std::path::PathBuf)> {
        // ---- Allocate ----------------------------------------------------
        let paths = RunPaths::allocate(artifacts_dir, &self.run_id)?;
        let scenario_dir = paths.scenario_dir(&self.fixture.scenario.id)?;
        let run_root = paths.root.clone();

        // Expand fixtures for this run's identity. A failed expansion is a
        // fixture defect → runner_error.
        let (scenario, script, expected_prompt) = match self.expand_fixture() {
            Ok(expanded) => expanded,
            Err(e) => {
                tracing::error!("fixture expansion failed: {e:#}");
                return Ok((Classification::RunnerError, run_root));
            }
        };
        let deadlines = scenario.deadlines;

        // ---- Boot ---------------------------------------------------------
        let mut stack = match Stack::boot(self.bins, paths).await {
            Ok(stack) => stack,
            Err(e) => {
                tracing::error!("stack boot failed: {e:#}");
                return Ok((Classification::SetupError, run_root));
            }
        };

        let outcome = self
            .run_armed(
                &mut stack,
                &scenario,
                script,
                &expected_prompt,
                &scenario_dir,
                deadlines,
            )
            .await;

        // ---- Teardown ------------------------------------------------------
        stack.teardown().await;

        match outcome {
            Ok(classification) => Ok((classification, run_root)),
            Err(e) => {
                tracing::error!("scenario execution fault: {e:#}");
                Ok((Classification::RunnerError, run_root))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_armed(
        &mut self,
        stack: &mut Stack,
        scenario: &ConformanceScenarioV1,
        script: RouterScriptV1,
        expected_prompt: &str,
        scenario_dir: &Path,
        deadlines: crate::types::scenario::DeadlinesV1,
    ) -> anyhow::Result<Classification> {
        let run_root = stack.paths.root.clone();

        // Test-support workers (in-process, own connections).
        let router = ScriptedRouter::start(&stack.ws_url, script).await?;
        let recorder = Recorder::start(&stack.ws_url, run_root.join("recorder.log.jsonl")).await?;
        let client = Client::connect(&stack.ws_url, "conformance-runner");

        let classification = self
            .drive(
                stack,
                scenario,
                expected_prompt,
                scenario_dir,
                deadlines,
                &router,
                &recorder,
                &client,
            )
            .await;

        router.shutdown().await;
        recorder.shutdown().await;
        client.shutdown().await;
        classification
    }

    /// Probe → Arm → Send → Await → Collect → Grade.
    #[allow(clippy::too_many_arguments)]
    async fn drive(
        &mut self,
        stack: &mut Stack,
        scenario: &ConformanceScenarioV1,
        expected_prompt: &str,
        scenario_dir: &Path,
        deadlines: crate::types::scenario::DeadlinesV1,
        router: &ScriptedRouter,
        recorder: &Recorder,
        client: &Client,
    ) -> anyhow::Result<Classification> {
        let run_root = stack.paths.root.clone();

        // ---- Probe (pre-harness surface) -------------------------------------
        let spec = ReadinessSpec::pre_harness(expected_config_entries(&stack.paths));
        if let Err(report) = readiness::probe(client, &spec, deadlines.readiness_ms).await {
            return self.readiness_failed(stack, scenario_dir, &run_root, report);
        }

        // ---- Arm -----------------------------------------------------------
        // The run-scoped target must exist BEFORE the harness boots: native
        // exposure reads the harness's boot-seeded registry snapshot
        // (`harness/src/discovery.rs`), and arming after boot races its
        // reactive refresh.
        if let Err(e) = self.arm(scenario, client).await {
            if let Some(exit) = stack.early_exit() {
                tracing::error!("process {} exited during arm: {}", exit.name, exit.status);
                return Ok(Classification::ProcessCrash);
            }
            tracing::error!("arm failed: {e:#}");
            self.artifacts.push(write_json(
                &run_root,
                &scenario_file(scenario_dir, "arm-failure.json"),
                &json!({ "error": format!("{e:#}") }),
            )?);
            return Ok(Classification::SetupError);
        }

        // ---- Boot the subject, then probe its surface ------------------------
        if let Err(e) = stack.spawn_harness(self.bins) {
            tracing::error!("harness spawn failed: {e:#}");
            return Ok(Classification::SetupError);
        }
        let spec = ReadinessSpec::harness_surface(expected_harness_config_entry(&stack.paths));
        if let Err(report) = readiness::probe(client, &spec, deadlines.readiness_ms).await {
            return self.readiness_failed(stack, scenario_dir, &run_root, report);
        }

        // The lifecycle binding needs the harness-registered trigger type,
        // so it is the one Arm step that runs after the subject boots (the
        // probe above proved the type is registered).
        if let Err(e) = recorder
            .bind_lifecycle(
                scenario.recorder.lifecycle.trigger_type.as_str(),
                &self.session_id,
            )
            .await
        {
            tracing::error!("lifecycle binding failed: {e:#}");
            return Ok(Classification::SetupError);
        }
        // The sha256 the router matches was computed from this template
        // expansion; persist it for diagnosis.
        std::fs::write(
            scenario_dir.join("expected-system-prompt.txt"),
            expected_prompt,
        )?;

        // ---- Send ----------------------------------------------------------
        self.artifacts.push(write_json(
            &run_root,
            &scenario_file(scenario_dir, "request.json"),
            &scenario.send,
        )?);
        let send_started = tokio::time::Instant::now();
        let send_response = client
            .call_with_timeout("harness::send", scenario.send.clone(), 30_000)
            .await;
        let (send_value, send_failed) = match &send_response {
            Ok(value) => (value.clone(), false),
            Err(e) => (json!({ "error": e }), true),
        };
        self.artifacts.push(write_json(
            &run_root,
            &scenario_file(scenario_dir, "send-response.json"),
            &send_value,
        )?);
        if send_failed {
            tracing::error!("harness::send failed: {send_value}");
            return Ok(Classification::ContractFailure);
        }
        let turn_id = send_value
            .get("turn_id")
            .and_then(Value::as_str)
            .map(String::from);

        // ---- Fault injection (explicit seed) --------------------------------
        // `engine_sigkill`: wait until the dispatched target call is durably
        // observed (it is executing inside its response delay), SIGKILL the
        // engine, hold the downtime, respawn. Workers survive and their SDK
        // connections replay registrations against the restarted engine.
        let scenario_deadline =
            send_started + std::time::Duration::from_millis(deadlines.scenario_ms);
        if let Some(fault) = &scenario.fault {
            let crate::types::scenario::FaultKind::EngineSigkill = fault.kind;
            let observed = loop {
                let count = recorder
                    .events()
                    .iter()
                    .filter(|e| e.kind == crate::types::recorder::RecorderEventKind::TargetCall)
                    .count() as u64;
                if count >= fault.after_target_calls {
                    break true;
                }
                if tokio::time::Instant::now() >= scenario_deadline {
                    break false;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            };
            if !observed {
                tracing::error!(
                    "fault trigger never armed: fewer than {} target calls before the deadline",
                    fault.after_target_calls
                );
                return Ok(Classification::Timeout);
            }
            stack.kill_engine()?;
            tokio::time::sleep(std::time::Duration::from_millis(fault.restart_delay_ms)).await;
            stack.respawn_engine()?;
            tracing::info!("engine respawned after fault injection");
        }

        // ---- Await ----------------------------------------------------------
        // Key on the returned session and turn: durable terminal status is
        // the clock; the lifecycle notification gets a short grace window
        // afterwards but can never hang collection.
        let mut timed_out = true;
        let mut final_status = Value::Null;
        while tokio::time::Instant::now() < scenario_deadline {
            if let Some(exit) = stack.early_exit() {
                tracing::error!("process {} exited mid-turn: {}", exit.name, exit.status);
                return Ok(Classification::ProcessCrash);
            }
            let status = client
                .call("harness::status", json!({ "session_id": self.session_id }))
                .await
                .unwrap_or(Value::Null);
            let terminal = matches!(
                status.get("status").and_then(Value::as_str),
                Some("completed") | Some("failed") | Some("cancelled")
            );
            final_status = status;
            if terminal {
                timed_out = false;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }

        if !timed_out {
            let grace = std::time::Duration::from_secs(3)
                .min(scenario_deadline.saturating_duration_since(tokio::time::Instant::now()));
            let grace_deadline = tokio::time::Instant::now() + grace;
            while tokio::time::Instant::now() < grace_deadline {
                let has_lifecycle = recorder
                    .events()
                    .iter()
                    .any(|e| e.kind == crate::types::recorder::RecorderEventKind::Lifecycle);
                if has_lifecycle {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        // ---- Collect ---------------------------------------------------------
        let transcript = self.collect_transcript(client).await?;
        self.artifacts.push(write_json(
            &run_root,
            &scenario_file(scenario_dir, "transcript.json"),
            &json!(transcript),
        )?);
        self.artifacts.push(write_json(
            &run_root,
            &scenario_file(scenario_dir, "status.json"),
            &final_status,
        )?);
        self.artifacts.push(write_json(
            &run_root,
            &scenario_file(scenario_dir, "router-calls.json"),
            &router.evidence(),
        )?);
        let recorder_events = recorder.events();
        let (target_calls, lifecycle_events): (Vec<_>, Vec<_>) = recorder_events
            .iter()
            .partition(|e| e.kind == crate::types::recorder::RecorderEventKind::TargetCall);
        self.artifacts.push(write_json(
            &run_root,
            &scenario_file(scenario_dir, "target-calls.json"),
            &json!(target_calls),
        )?);
        self.artifacts.push(write_json(
            &run_root,
            &scenario_file(scenario_dir, "lifecycle-events.json"),
            &json!(lifecycle_events),
        )?);

        // ---- Grade ------------------------------------------------------------
        let evidence = Evidence {
            session_id: self.session_id.clone(),
            turn_id,
            send_response: Some(send_value),
            status: final_status,
            transcript,
            generations_consumed: router.generations_consumed(),
            generations_total: router.total_generations(),
            recorder_events,
        };
        self.invariants = grader::grade(&scenario.invariants, &evidence);
        self.artifacts.push(write_json(
            &run_root,
            &scenario_file(scenario_dir, "invariants.json"),
            &serde_json::to_value(&self.invariants)?,
        )?);

        if timed_out {
            return Ok(Classification::Timeout);
        }
        let all_passed = self.invariants.iter().all(|i| i.passed);
        if all_passed && !router.contract_failed() {
            Ok(Classification::Pass)
        } else {
            Ok(Classification::ContractFailure)
        }
    }

    fn readiness_failed(
        &mut self,
        stack: &mut Stack,
        scenario_dir: &Path,
        run_root: &Path,
        report: readiness::ReadinessReport,
    ) -> anyhow::Result<Classification> {
        if let Some(exit) = stack.early_exit() {
            tracing::error!(
                "process {} exited during readiness: {}",
                exit.name,
                exit.status
            );
            return Ok(Classification::ProcessCrash);
        }
        self.artifacts.push(write_json(
            run_root,
            &scenario_file(scenario_dir, "readiness-failure.json"),
            &json!({ "missing": report.missing }),
        )?);
        tracing::error!("readiness failed: {:?}", report.missing);
        Ok(Classification::SetupError)
    }

    /// Arm (spec § recorder contract): configure the run-scoped target,
    /// verify the registered schema digest, reset the log, create the
    /// lifecycle binding, and hold Send until the target is discoverable and
    /// a direct snapshot is empty.
    async fn arm(&self, scenario: &ConformanceScenarioV1, client: &Client) -> anyhow::Result<()> {
        let configure = json!({
            "schema_version": "1",
            "run_id": self.run_id,
            "config": scenario.recorder,
        });
        let response = client
            .call("conformance-recorder::configure", configure)
            .await
            .map_err(|e| anyhow::anyhow!("configure: {e}"))?;
        let expected_digest = crate::canonical::sha256_of_canonical(&Value::Object(
            scenario.recorder.target.request_schema.clone(),
        ));
        let digest = response
            .get("target_schema_sha256")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if digest != expected_digest {
            anyhow::bail!("target schema digest mismatch: {digest} != {expected_digest}");
        }

        client
            .call(
                "conformance-recorder::reset",
                json!({ "schema_version": "1", "run_id": self.run_id }),
            )
            .await
            .map_err(|e| anyhow::anyhow!("reset: {e}"))?;

        // Target discoverable + empty snapshot before Send.
        let target_id = &scenario.recorder.target.function_id;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let listed = client
                .call(
                    "engine::functions::list",
                    json!({ "include_internal": true }),
                )
                .await
                .unwrap_or_default();
            if listed.to_string().contains(target_id.as_str()) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("target {target_id} never appeared in function discovery");
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let snapshot = client
            .call(
                "conformance-recorder::snapshot",
                json!({ "schema_version": "1", "run_id": self.run_id }),
            )
            .await
            .map_err(|e| anyhow::anyhow!("snapshot: {e}"))?;
        let events = snapshot.get("events").and_then(Value::as_array);
        if events.is_none_or(|e| !e.is_empty()) {
            anyhow::bail!("recorder snapshot not empty after reset");
        }
        Ok(())
    }

    async fn collect_transcript(&self, client: &Client) -> anyhow::Result<Vec<Value>> {
        let mut messages = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut payload = json!({
                "session_id": self.session_id,
                "limit": 100,
                "include_custom": true,
            });
            if let Some(c) = &cursor {
                payload["cursor"] = json!(c);
            }
            let page = client
                .call("session::messages", payload)
                .await
                .map_err(|e| anyhow::anyhow!("session::messages: {e}"))?;
            if let Some(items) = page.get("messages").and_then(Value::as_array) {
                messages.extend(items.iter().cloned());
            }
            match page.get("next_cursor").and_then(Value::as_str) {
                Some(next) => cursor = Some(next.to_string()),
                None => break,
            }
        }
        Ok(messages)
    }

    /// Render the expected system prompt, hash it, and expand every fixture
    /// with the full placeholder set.
    fn expand_fixture(&self) -> anyhow::Result<(ConformanceScenarioV1, RouterScriptV1, String)> {
        let base = Placeholders::new(&self.run_id, &self.session_id);
        let expected_prompt = base.expand_str(&self.fixture.system_prompt_template)?;
        let digest = crate::canonical::sha256_of_bytes(expected_prompt.as_bytes());
        let placeholders = base.with_system_prompt_sha256(&digest);

        let mut scenario_value = serde_json::to_value(&self.fixture.scenario)?;
        placeholders.expand_value(&mut scenario_value)?;
        let scenario: ConformanceScenarioV1 = serde_json::from_value(scenario_value)?;

        let mut script_value = serde_json::to_value(&self.fixture.script)?;
        placeholders.expand_value(&mut script_value)?;
        let script: RouterScriptV1 = serde_json::from_value(script_value)?;

        Ok((scenario, script, expected_prompt))
    }
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}
