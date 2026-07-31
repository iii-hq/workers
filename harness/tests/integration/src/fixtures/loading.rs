use crate::expand::CompiledFixtureV1;
use crate::scenarios::{ScenarioDriver, VerifyFn};
use crate::types::scenario::CompiledScenarioV1;
use crate::types::script::RouterScriptV1;

#[derive(Debug, Clone)]
pub struct ScenarioFixture {
    pub slug: String,
    pub driver: ScenarioDriver,
    /// Number of distinct terminal turns the run must observe. Usually 1;
    /// Playground turns and harness-initiated follow-on turns (e.g. a reseed
    /// after a message parks mid-final-step) push it higher. Always equals
    /// `expected_turn_statuses.len()`.
    pub expected_terminal_turns: usize,
    /// Each terminal turn's lifecycle status, in completion order. The last
    /// must be `completed` — the floor's durable-status check binds to it.
    pub expected_turn_statuses: Vec<String>,
    pub scenario: CompiledScenarioV1,
    pub script: RouterScriptV1,
    /// Compiled Harness default plus inferred session/policy aid.
    pub system_prompt_template: String,
    /// Scenario-specific checks over the collected run evidence.
    pub verify: VerifyFn,
    /// Probe-driven actions to fire between terminal turns. Each runs after
    /// its `after_turns` completion count is observed, so a reaction it trips
    /// runs while the tracked session is idle (ordinal-clean under the
    /// scripted router). `{{run_id}}`/`{{session_id}}` in the payload expand.
    pub probe_actions: Vec<ProbeAction>,
    /// Extra environment for the harness process under test. The integration
    /// knobs live here (e.g. `III_HARNESS_FIRE_WINDOW_MS` to shrink the
    /// fire-rate window so a coalescing scenario completes in seconds).
    pub harness_env: Vec<(String, String)>,
    /// Await this many controlled-function invocations (probe-side, whole-run)
    /// after every terminal turn arrived. This is the only way to await work
    /// that produces no session turn — a call-mode reaction's dispatch,
    /// including a trailing coalesced fire.
    pub await_target_calls: Option<usize>,
    /// Session-trace-count override for the floor. `None` keeps the driver
    /// formula. Needed when probe actions trip CALL-mode reactions, which
    /// dispatch a function without seeding any session turn — no extra trace
    /// tree ever forms.
    pub traces_override: Option<usize>,
    /// Optional runner-owned mid-flight control sequence. This is deliberately
    /// outside the compiled subject scenario: it is test synchronization, not
    /// a public harness request.
    pub intervention: Option<ScenarioIntervention>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioIntervention {
    StopCancelCascade {
        gate: String,
        expected_in_flight: usize,
        queued_message: String,
    },
    QueuedMessageEditUnqueue {
        gate: String,
        before_message: String,
        edit_message: String,
        edit_replacement: String,
        remove_message: String,
        after_message: String,
    },
}

/// A function the PROBE (test infra, not a model turn) invokes at a completion
/// boundary — the seam that lets a scenario trip a state key after the main
/// turn is idle instead of mid-turn.
#[derive(Debug, Clone)]
pub struct ProbeAction {
    pub after_turns: usize,
    /// Additionally wait until the controlled function served this many
    /// invocations (probe-side, whole-run) before firing. The only ordering
    /// signal an UNTRACKED session can emit: e.g. fire the next action only
    /// after a call-mode reaction proved a worker session completed.
    pub after_target_calls: Option<usize>,
    pub function_id: String,
    pub payload: serde_json::Value,
}

impl ScenarioFixture {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.slug.is_empty()
                && self
                    .slug
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "scenario slug {:?} is not filesystem-safe",
            self.slug
        );
        anyhow::ensure!(
            !self.scenario.id.is_empty()
                && self.scenario.id.len() <= 128
                && self
                    .scenario
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "scenario id {:?} is not filesystem-safe",
            self.scenario.id
        );
        anyhow::ensure!(
            !self.scenario.description.trim().is_empty(),
            "scenario description must not be empty"
        );
        anyhow::ensure!(
            self.expected_terminal_turns > 0,
            "scenario must expect at least one terminal turn"
        );
        anyhow::ensure!(
            self.expected_turn_statuses.len() == self.expected_terminal_turns,
            "scenario declares {} turn status(es) for {} terminal turn(s)",
            self.expected_turn_statuses.len(),
            self.expected_terminal_turns
        );
        for status in &self.expected_turn_statuses {
            anyhow::ensure!(
                matches!(status.as_str(), "completed" | "failed" | "cancelled"),
                "unknown terminal turn status {status:?}"
            );
        }
        // Controlled-call waits depend on a controlled function existing:
        // `Some(0)` would fail inside `wait_for_target_calls`, and a positive
        // count with no `target` registered waits until the deadline burns.
        // Both are authoring mistakes — fail them before the stack boots.
        if let Some(calls) = self.await_target_calls {
            anyhow::ensure!(calls > 0, "await_target_calls must be positive");
            anyhow::ensure!(
                self.scenario.target.is_some(),
                "await_target_calls needs a controlled function (`.function(...)`) to count"
            );
        }
        for action in &self.probe_actions {
            if let Some(calls) = action.after_target_calls {
                anyhow::ensure!(
                    calls > 0,
                    "probe action after_target_calls must be positive"
                );
                anyhow::ensure!(
                    self.scenario.target.is_some(),
                    "a probe action gated on target calls needs a controlled function to count"
                );
            }
        }
        if self.intervention.is_none() {
            anyhow::ensure!(
                self.expected_turn_statuses.last().map(String::as_str) == Some("completed"),
                "the last terminal turn must be completed"
            );
        }
        if let Some(intervention) = &self.intervention {
            match intervention {
                ScenarioIntervention::StopCancelCascade {
                    gate,
                    expected_in_flight,
                    queued_message,
                } => {
                    anyhow::ensure!(!gate.is_empty(), "stop-cascade gate must not be empty");
                    anyhow::ensure!(
                        *expected_in_flight > 0,
                        "stop-cascade must expect at least one in-flight generation"
                    );
                    anyhow::ensure!(
                        !queued_message.is_empty(),
                        "stop-cascade queued message must not be empty"
                    );
                    anyhow::ensure!(
                        self.expected_turn_statuses
                            .iter()
                            .all(|status| status == "cancelled"),
                        "stop-cascade terminal statuses must all be cancelled"
                    );
                }
                ScenarioIntervention::QueuedMessageEditUnqueue {
                    gate,
                    before_message,
                    edit_message,
                    edit_replacement,
                    remove_message,
                    after_message,
                } => {
                    anyhow::ensure!(!gate.is_empty(), "queued-edit gate must not be empty");
                    for (label, message) in [
                        ("before", before_message),
                        ("edit", edit_message),
                        ("replacement", edit_replacement),
                        ("remove", remove_message),
                        ("after", after_message),
                    ] {
                        anyhow::ensure!(
                            !message.is_empty(),
                            "queued-edit {label} message must not be empty"
                        );
                    }
                    anyhow::ensure!(
                        edit_message != edit_replacement,
                        "queued-edit replacement must change the message"
                    );
                    anyhow::ensure!(
                        self.expected_turn_statuses == ["completed"],
                        "queued-edit scenario must end with one completed turn"
                    );
                }
            }
        }
        // A direct scenario is one external send, but the harness may seed
        // further turns from it (a reseed after a parked message); those extra
        // terminal turns are declared with `terminal_turns(n)` and awaited the
        // same way Playground awaits its externally driven turns.
        if self.script.scenario_id != self.scenario.id {
            anyhow::bail!(
                "script scenario_id {:?} does not match scenario id {:?}",
                self.script.scenario_id,
                self.scenario.id
            );
        }
        if let Some(target) = &self.scenario.target {
            anyhow::ensure!(
                target.function_id.starts_with("{{run_id}}::"),
                "controlled function {:?} must be run-scoped",
                target.function_id
            );
        }
        if let Some(fault) = &self.scenario.fault {
            let target =
                self.scenario.target.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("fault injection needs a controlled function")
                })?;
            anyhow::ensure!(
                fault.after_target_calls > 0,
                "fault after_target_calls must be positive"
            );
            anyhow::ensure!(
                fault.function_id == target.function_id,
                "fault function {:?} does not match controlled target {:?}",
                fault.function_id,
                target.function_id
            );
            anyhow::ensure!(
                target.hold_response,
                "engine fault target must hold its response until SIGKILL"
            );
        }
        anyhow::ensure!(
            !self.script.generations.is_empty(),
            "router script has no generations"
        );
        let mut ordinals = std::collections::BTreeSet::new();
        // Serve-time capture wiring: a `[[cap:name]]` frame reference is only
        // resolvable when an earlier-or-same-ordinal generation declared the
        // capture (strict-ordinal consumption guarantees it ran first).
        let mut sorted: Vec<_> = self.script.generations.iter().collect();
        sorted.sort_by_key(|g| g.ordinal);
        let mut declared = std::collections::BTreeSet::new();
        for generation in sorted {
            for capture in &generation.captures {
                let regex = regex::Regex::new(&capture.pattern).map_err(|error| {
                    anyhow::anyhow!("capture {} pattern invalid: {error}", capture.name)
                })?;
                anyhow::ensure!(
                    regex.captures_len() >= 2,
                    "capture {} pattern must have a capture group",
                    capture.name
                );
                declared.insert(capture.name.clone());
            }
            let frames = serde_json::to_string(&generation.frames)?;
            let mut rest = frames.as_str();
            while let Some(start) = rest.find("[[cap:") {
                let name_start = &rest[start + 6..];
                let end = name_start.find("]]").ok_or_else(|| {
                    anyhow::anyhow!(
                        "generation {} has an unterminated [[cap: reference",
                        generation.ordinal
                    )
                })?;
                let name = &name_start[..end];
                anyhow::ensure!(
                    declared.contains(name),
                    "generation {} references [[cap:{name}]] before any generation captured it",
                    generation.ordinal
                );
                rest = &name_start[end..];
            }
        }
        for generation in &self.script.generations {
            anyhow::ensure!(
                ordinals.insert(generation.ordinal),
                "duplicate router generation ordinal {}",
                generation.ordinal
            );
            if generation.failure.is_some() {
                anyhow::ensure!(
                    generation.frames.is_empty(),
                    "failing generation {} must stream no frames",
                    generation.ordinal
                );
                continue;
            }
            anyhow::ensure!(
                generation
                    .frames
                    .last()
                    .is_some_and(|frame| frame.is_terminal()),
                "generation {} does not end in a terminal frame",
                generation.ordinal
            );
            anyhow::ensure!(
                !generation.frames[..generation.frames.len() - 1]
                    .iter()
                    .any(|frame| frame.is_terminal()),
                "generation {} contains an early terminal frame",
                generation.ordinal
            );
        }
        crate::expand::expand_compiled_fixture(
            &self.compiled(),
            "validate-run",
            "validate-session",
        )?;
        Ok(())
    }

    /// Distinct trace trees the run must produce. Turns chain into their
    /// initiating trace through the durable queue, so a direct scenario's one
    /// send yields exactly one trace no matter how many turns it seeds (a
    /// finalize reseed enqueues from inside the finalizing step); Playground
    /// turns are each externally initiated and trace separately.
    pub fn expected_traces(&self) -> usize {
        if let Some(traces) = self.traces_override {
            return traces;
        }
        match self.driver {
            // The send's own trace, plus one per probe action: a probe-tripped
            // TASK reaction fires from a fresh externally-initiated flow (the
            // state/cron worker's fan-out), so it forms its own trace tree —
            // exactly like a Playground-driven external turn. Scenarios whose
            // probe actions trip CALL reactions (no session turn, no trace)
            // override via `traces_override`.
            ScenarioDriver::Direct => 1 + self.probe_actions.len(),
            ScenarioDriver::Playground => self.expected_terminal_turns,
        }
    }

    pub fn compiled(&self) -> CompiledFixtureV1 {
        CompiledFixtureV1 {
            scenario: self.scenario.clone(),
            script: self.script.clone(),
            system_prompt_template: self.system_prompt_template.clone(),
        }
    }
}
