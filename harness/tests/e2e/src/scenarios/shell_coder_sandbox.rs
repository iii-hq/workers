use std::path::{Component, Path, PathBuf};

use anyhow::bail;
use serde_json::{json, Value};

use crate::context::E2eContext;

use super::common;
use super::{
    CleanupFuture, CriterionSpec, EvaluationFuture, ExecutionPolicy, ObjectiveEvaluation,
    ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "shell_coder_sandbox";
const DRAFT_NAME: &str = "draft_check.py";
const FINAL_NAME: &str = "checks/check.py";
const DRAFT_SCRIPT: &str = "values = [2, 3, 5, 7]\nprint(\"host-check:draft\")\n";
const FINAL_SCRIPT: &str = "values = [2, 3, 5, 7]\nprint(f\"host-check:{sum(values)}\")\n";
const HOST_STDOUT: &str = "host-check:17";
const SANDBOX_STDOUT: &str = "sandbox-check:35";
const EXPECTED_CORE_OPERATIONS: usize = 12;
const PASS_THRESHOLD: u8 = 50;
const EXECUTION_QUALITY_WEIGHT: u8 = 45;

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let sandbox_name = sandbox_name(run_id);
    ScenarioSpec {
        id: ID,
        version: 1,
        prompt: format!(
            "Perform this verification entirely in the current workspace and in the stated order.\n\n\
             1. Add the `shell` worker from the public registry and wait for that add operation to \
             finish. Only after it finishes, add the `iii-sandbox` worker from the public registry \
             and wait for that operation too, even if either worker is already installed. Do not \
             launch the two add operations in parallel.\n\
             2. Invoke the dedicated code-file capability's own access-contract inspection \
             operation. Generic engine function discovery does not satisfy this step.\n\
             3. With dedicated code-file operations, create `{DRAFT_NAME}` with exactly:\n\n\
             ```python\n{DRAFT_SCRIPT}```\n\n\
             4. With the dedicated code-file update operation, change only the second line so the \
             file content becomes exactly:\n\n\
             ```python\n{FINAL_SCRIPT}```\n\n\
             5. With the dedicated code-file move operation, move it to `{FINAL_NAME}`.\n\
             6. Read `{FINAL_NAME}` back with the dedicated code-file read operation and verify \
             the exact final content. Do not create, edit, move, or read this code file with a \
             general shell filesystem operation or by running a command; these steps specifically \
             validate the code-file capabilities.\n\
             7. Run `{FINAL_NAME}` with Python 3 on the host workspace and verify stdout is exactly \
             `{HOST_STDOUT}`.\n\
             8. Create a short-lived isolated Python sandbox named `{sandbox_name}` with exactly \
             1 CPU, 256 MB of memory, and networking disabled. Use the sandbox's own command \
             execution capability to run Python inside it and produce stdout exactly \
             `{SANDBOX_STDOUT}`. Do not use the shell's sandbox target for this step.\n\
             9. List the sandboxes and confirm that named sandbox is present and active, then stop \
             it and wait for shutdown.\n\n\
             Only report completion after every operation succeeds. Include both exact output \
             lines in your final response."
        ),
        filesystem_root: Some(workspace_root(run_id)),
        execution: ExecutionPolicy {
            max_turns: 40,
            max_output_tokens: Some(8_192),
            max_total_tokens: 1_638_400,
            stuck_timeout_seconds: 600,
        },
        denied_functions: &[],
        threshold: PASS_THRESHOLD,
        criteria: vec![
            CriterionSpec {
                id: "worker_setup",
                weight: 10,
                description: "Both registry workers are added and expose their required surfaces.",
            },
            CriterionSpec {
                id: "coder_workflow",
                weight: 20,
                description:
                    "Coder inspection, create, update, move, and read operations produce the exact file.",
            },
            CriterionSpec {
                id: "host_execution",
                weight: 10,
                description: "The final file runs successfully on the host with exact stdout.",
            },
            CriterionSpec {
                id: "sandbox_lifecycle",
                weight: 10,
                description:
                    "A named isolated sandbox is created, executed in, listed, and stopped.",
            },
            CriterionSpec {
                id: "completion_report",
                weight: 5,
                description: "The final response includes both exact observed outputs.",
            },
            CriterionSpec {
                id: "execution_quality",
                weight: EXECUTION_QUALITY_WEIGHT,
                description:
                    "The workflow completes without function-call errors; recovered errors lower quality without overriding validated effects.",
            },
        ],
        judge_reference: None,
        setup: None,
        evaluate,
        cleanup: Some(cleanup),
    }
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let invocations = common::function_invocations(&observation.transcript);
        let calls = invocations
            .iter()
            .map(|invocation| invocation.call.clone())
            .collect::<Vec<_>>();
        let installed_shell = calls.iter().any(|call| is_registry_install(call, "shell"));
        let installed_sandbox = calls
            .iter()
            .any(|call| is_registry_install(call, "iii-sandbox"));

        let shell_surface_ready = context.function_exists("shell::exec").await?;
        let coder_surface_ready = context.function_exists("coder::create-file").await?;
        let sandbox_surface_ready = context.function_exists("sandbox::create").await?;
        let surfaces_ready = shell_surface_ready && coder_surface_ready && sandbox_surface_ready;
        let worker_setup = installed_shell && installed_sandbox && surfaces_ready;

        let root = workspace_root(run_id);
        let coder_info = calls.iter().any(|call| call.function_id == "coder::info");
        let coder_create = calls.iter().any(|call| is_exact_create(call, &root));
        let coder_update = calls.iter().any(|call| is_exact_update(call, &root));
        let coder_move = calls.iter().any(|call| is_exact_move(call, &root));
        let coder_read = calls.iter().any(|call| is_exact_read(call, &root));
        let coder_ordered = calls_are_ordered(
            &calls,
            &[
                "coder::info",
                "coder::create-file",
                "coder::update-file",
                "coder::move",
                "coder::read-file",
            ],
        );
        let coder_results_succeeded = correlated_call_succeeded(
            &observation.transcript,
            &invocations,
            |call| call.function_id == "coder::info",
            |_| true,
        ) && correlated_call_succeeded(
            &observation.transcript,
            &invocations,
            |call| is_exact_create(call, &root),
            batch_result_succeeded,
        ) && correlated_call_succeeded(
            &observation.transcript,
            &invocations,
            |call| is_exact_update(call, &root),
            batch_result_succeeded,
        ) && correlated_call_succeeded(
            &observation.transcript,
            &invocations,
            |call| is_exact_move(call, &root),
            batch_result_succeeded,
        ) && correlated_call_succeeded(
            &observation.transcript,
            &invocations,
            |call| is_exact_read(call, &root),
            successful_coder_read,
        );

        let draft_path = root.join(DRAFT_NAME);
        let final_path = root.join(FINAL_NAME);
        let final_read = std::fs::read_to_string(&final_path);
        let final_matches = final_read
            .as_deref()
            .is_ok_and(|content| content == FINAL_SCRIPT);
        let draft_removed = !draft_path.exists();
        let coder_workflow = coder_info
            && coder_create
            && coder_update
            && coder_move
            && coder_read
            && coder_ordered
            && coder_results_succeeded
            && final_matches
            && draft_removed;

        let host_exec = calls.iter().any(|call| is_exact_host_exec(call, &root));
        let host_output = correlated_call_succeeded(
            &observation.transcript,
            &invocations,
            |call| is_exact_host_exec(call, &root),
            |result| successful_output_result(result, HOST_STDOUT),
        );
        let host_execution = host_exec && host_output;

        let expected_sandbox_name = sandbox_name(run_id);
        let sandbox = sandbox_evidence(&observation.transcript, &calls, &expected_sandbox_name);
        let sandbox_lifecycle = sandbox.complete();

        let core_operations = [
            installed_shell,
            installed_sandbox,
            coder_info,
            coder_create,
            coder_update,
            coder_move,
            coder_read,
            host_exec,
            sandbox.created,
            sandbox.executed,
            sandbox.listed,
            sandbox.stopped,
        ]
        .into_iter()
        .filter(|observed| *observed)
        .count();
        let operation_volume = core_operations >= EXPECTED_CORE_OPERATIONS;
        let function_call_errors = observation.metrics.totals.function_call_errors;
        let response_reports_outputs = observation.response.contains(HOST_STDOUT)
            && observation.response.contains(SANDBOX_STDOUT);

        Ok(ObjectiveEvaluation {
            hard_gates: vec![
                common::gate(
                    "workers_added_and_ready",
                    worker_setup,
                    format!(
                        "shell add={installed_shell}, iii-sandbox add={installed_sandbox}, \
                         shell ready={shell_surface_ready}, coder ready={coder_surface_ready}, \
                         sandbox ready={sandbox_surface_ready}"
                    ),
                ),
                common::gate(
                    "ordered_coder_workflow_completed",
                    coder_workflow,
                    match &final_read {
                        Ok(_) => format!(
                            "info={coder_info}, create={coder_create}, update={coder_update}, \
                             move={coder_move}, read={coder_read}, ordered={coder_ordered}, \
                             successful results={coder_results_succeeded}, exact final \
                             content={final_matches}, draft removed={draft_removed}"
                        ),
                        Err(error) => format!(
                            "coder operations info/create/update/move/read={coder_info}/{coder_create}/\
                             {coder_update}/{coder_move}/{coder_read}; could not read {}: {error}",
                            final_path.display()
                        ),
                    },
                ),
                common::gate(
                    "host_execution_succeeded",
                    host_execution,
                    format!(
                        "exact host execution call={host_exec}, exact successful stdout={host_output}"
                    ),
                ),
                common::gate(
                    "sandbox_lifecycle_completed",
                    sandbox_lifecycle,
                    sandbox.summary(),
                ),
                common::gate(
                    "operation_volume_reached",
                    operation_volume,
                    format!(
                        "observed {core_operations} of {EXPECTED_CORE_OPERATIONS} required core operations"
                    ),
                ),
            ],
            awards: vec![
                common::award(
                    "worker_setup",
                    if worker_setup { 10 } else { 0 },
                    "awarded for adding both registry workers and exposing all three surfaces",
                ),
                common::award(
                    "coder_workflow",
                    if coder_workflow { 20 } else { 0 },
                    "awarded for the ordered inspect/create/update/move/read workflow and exact file",
                ),
                common::award(
                    "host_execution",
                    if host_execution { 10 } else { 0 },
                    "awarded for exact successful host stdout",
                ),
                common::award(
                    "sandbox_lifecycle",
                    if sandbox_lifecycle { 10 } else { 0 },
                    "awarded for creating, executing in, listing, and stopping the isolated sandbox",
                ),
                common::award(
                    "completion_report",
                    if response_reports_outputs { 5 } else { 0 },
                    "awarded when the final response includes both exact outputs",
                ),
                common::award(
                    "execution_quality",
                    execution_quality_award(function_call_errors),
                    format!(
                        "observed {function_call_errors} function-call error(s); recovered errors reduce quality but validated outcomes remain authoritative"
                    ),
                ),
            ],
        })
    })
}

fn execution_quality_award(function_call_errors: u64) -> u8 {
    if function_call_errors == 0 {
        EXECUTION_QUALITY_WEIGHT
    } else {
        0
    }
}

fn is_registry_install(call: &common::ObservedFunctionCall, worker: &str) -> bool {
    call.function_id == "worker::add"
        && call
            .arguments
            .pointer("/source/kind")
            .and_then(Value::as_str)
            == Some("registry")
        && call
            .arguments
            .pointer("/source/name")
            .and_then(Value::as_str)
            == Some(worker)
}

fn is_exact_create(call: &common::ObservedFunctionCall, root: &Path) -> bool {
    call.function_id == "coder::create-file"
        && workspace_path_matches(
            call.arguments
                .pointer("/files/0/path")
                .and_then(Value::as_str),
            root,
            DRAFT_NAME,
        )
        && call
            .arguments
            .pointer("/files/0/content")
            .and_then(Value::as_str)
            == Some(DRAFT_SCRIPT)
        && call
            .arguments
            .get("files")
            .and_then(Value::as_array)
            .is_some_and(|files| files.len() == 1)
}

fn is_exact_update(call: &common::ObservedFunctionCall, root: &Path) -> bool {
    call.function_id == "coder::update-file"
        && workspace_path_matches(
            call.arguments
                .pointer("/files/0/path")
                .and_then(Value::as_str),
            root,
            DRAFT_NAME,
        )
        && call
            .arguments
            .pointer("/files/0/ops")
            .and_then(Value::as_array)
            .is_some_and(|ops| !ops.is_empty())
        && call
            .arguments
            .get("files")
            .and_then(Value::as_array)
            .is_some_and(|files| files.len() == 1)
}

fn is_exact_move(call: &common::ObservedFunctionCall, root: &Path) -> bool {
    call.function_id == "coder::move"
        && workspace_path_matches(
            call.arguments
                .pointer("/files/0/from")
                .and_then(Value::as_str),
            root,
            DRAFT_NAME,
        )
        && workspace_path_matches(
            call.arguments
                .pointer("/files/0/to")
                .and_then(Value::as_str),
            root,
            FINAL_NAME,
        )
        && call
            .arguments
            .get("files")
            .and_then(Value::as_array)
            .is_some_and(|files| files.len() == 1)
}

fn is_exact_read(call: &common::ObservedFunctionCall, root: &Path) -> bool {
    call.function_id == "coder::read-file"
        && workspace_path_matches(
            call.arguments.get("path").and_then(Value::as_str),
            root,
            FINAL_NAME,
        )
        && call.arguments.get("paths").is_none()
}

fn is_exact_host_exec(call: &common::ObservedFunctionCall, root: &Path) -> bool {
    let host_target = call.arguments.get("target").is_none()
        || call
            .arguments
            .pointer("/target/kind")
            .and_then(Value::as_str)
            == Some("host");
    let python = call
        .arguments
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| matches!(command, "python" | "python3"));
    let final_arg = call
        .arguments
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| {
            args.iter()
                .any(|arg| workspace_path_matches(arg.as_str(), root, FINAL_NAME))
        });
    call.function_id == "shell::exec" && host_target && python && final_arg
}

fn workspace_path_matches(value: Option<&str>, root: &Path, expected: &str) -> bool {
    let Some(value) = value else {
        return false;
    };
    let path = Path::new(value);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };

    let Some(resolved) = normalize_workspace_path(&resolved) else {
        return false;
    };
    let Some(expected) = normalize_workspace_path(&root.join(expected)) else {
        return false;
    };
    resolved == expected
}

fn normalize_workspace_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => return None,
            Component::Normal(part) => normalized.push(part),
        }
    }
    Some(normalized)
}

fn calls_are_ordered(calls: &[common::ObservedFunctionCall], required: &[&str]) -> bool {
    let mut next = 0;
    for required_id in required {
        let Some(offset) = calls[next..]
            .iter()
            .position(|call| call.function_id == *required_id)
        else {
            return false;
        };
        next += offset + 1;
    }
    true
}

fn correlated_call_succeeded(
    transcript: &Value,
    invocations: &[common::ObservedFunctionInvocation],
    call_matches: impl Fn(&common::ObservedFunctionCall) -> bool,
    result_matches: impl Fn(&Value) -> bool,
) -> bool {
    invocations
        .iter()
        .filter(|invocation| call_matches(&invocation.call))
        .any(|invocation| {
            common::function_result(transcript, invocation).is_some_and(&result_matches)
        })
}

fn function_results<'a>(transcript: &'a Value, function_id: &'a str) -> Vec<&'a Value> {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("message"))
        .filter(|message| {
            message.get("role").and_then(Value::as_str) == Some("function_result")
                && message.get("function_id").and_then(Value::as_str) == Some(function_id)
                && message.get("is_error").and_then(Value::as_bool) == Some(false)
        })
        .collect()
}

fn batch_result_succeeded(message: &Value) -> bool {
    message
        .pointer("/details/results")
        .and_then(Value::as_array)
        .is_some_and(|results| {
            !results.is_empty()
                && results
                    .iter()
                    .all(|result| result.get("success").and_then(Value::as_bool) == Some(true))
        })
}

fn successful_coder_read(message: &Value) -> bool {
    message.pointer("/details/content").and_then(Value::as_str) == Some(FINAL_SCRIPT)
}

fn successful_output(transcript: &Value, function_id: &str, expected: &str) -> bool {
    function_results(transcript, function_id)
        .into_iter()
        .any(|message| successful_output_result(message, expected))
}

fn successful_output_result(message: &Value, expected: &str) -> bool {
    message
        .pointer("/details/exit_code")
        .and_then(Value::as_i64)
        == Some(0)
        && message
            .pointer("/details/stdout")
            .and_then(Value::as_str)
            .is_some_and(|stdout| stdout.trim() == expected)
        && message
            .pointer("/details/stderr")
            .and_then(Value::as_str)
            .is_some_and(|stderr| stderr.trim().is_empty())
        && message
            .pointer("/details/timed_out")
            .and_then(Value::as_bool)
            != Some(true)
}

#[derive(Debug, Default)]
struct SandboxEvidence {
    sandbox_id: Option<String>,
    create_request: bool,
    created: bool,
    executed: bool,
    listed: bool,
    stopped: bool,
    ordered: bool,
}

impl SandboxEvidence {
    fn complete(&self) -> bool {
        self.create_request
            && self.created
            && self.executed
            && self.listed
            && self.stopped
            && self.ordered
    }

    fn summary(&self) -> String {
        format!(
            "sandbox id={}, exact create request={}, created={}, executed with exact stdout={}, \
             listed active={}, stopped={}, lifecycle ordered={}",
            self.sandbox_id.as_deref().unwrap_or("missing"),
            self.create_request,
            self.created,
            self.executed,
            self.listed,
            self.stopped,
            self.ordered
        )
    }
}

fn sandbox_evidence(
    transcript: &Value,
    calls: &[common::ObservedFunctionCall],
    expected_name: &str,
) -> SandboxEvidence {
    let sandbox_id = function_results(transcript, "sandbox::create")
        .into_iter()
        .find_map(|message| {
            (message.pointer("/details/image").and_then(Value::as_str) == Some("python"))
                .then(|| {
                    message
                        .pointer("/details/sandbox_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .flatten()
        });
    let create_request = calls.iter().any(|call| {
        call.function_id == "sandbox::create"
            && call.arguments.get("image").and_then(Value::as_str) == Some("python")
            && call.arguments.get("name").and_then(Value::as_str) == Some(expected_name)
            && call.arguments.get("cpus").and_then(Value::as_u64) == Some(1)
            && call.arguments.get("memory_mb").and_then(Value::as_u64) == Some(256)
            && call.arguments.get("network").and_then(Value::as_bool) == Some(false)
    });

    let Some(id) = sandbox_id.as_deref() else {
        return SandboxEvidence {
            sandbox_id,
            create_request,
            ..SandboxEvidence::default()
        };
    };
    let exec_call = calls.iter().any(|call| {
        call.function_id == "sandbox::exec"
            && call.arguments.get("sandbox_id").and_then(Value::as_str) == Some(id)
    });
    let executed = exec_call && successful_output(transcript, "sandbox::exec", SANDBOX_STDOUT);
    let list_call = calls.iter().any(|call| call.function_id == "sandbox::list");
    let listed = list_call
        && function_results(transcript, "sandbox::list")
            .into_iter()
            .any(|message| {
                message
                    .pointer("/details/sandboxes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|sandbox| {
                        sandbox.get("sandbox_id").and_then(Value::as_str) == Some(id)
                            && sandbox.get("name").and_then(Value::as_str) == Some(expected_name)
                            && sandbox.get("stopped").and_then(Value::as_bool) == Some(false)
                    })
            });
    let stop_call = calls.iter().any(|call| {
        call.function_id == "sandbox::stop"
            && call.arguments.get("sandbox_id").and_then(Value::as_str) == Some(id)
            && call.arguments.get("wait").and_then(Value::as_bool) == Some(true)
    });
    let stopped = stop_call
        && function_results(transcript, "sandbox::stop")
            .into_iter()
            .any(|message| {
                message
                    .pointer("/details/sandbox_id")
                    .and_then(Value::as_str)
                    == Some(id)
                    && message.pointer("/details/stopped").and_then(Value::as_bool) == Some(true)
            });
    let ordered = sandbox_calls_are_ordered(calls, expected_name, id);

    SandboxEvidence {
        sandbox_id,
        create_request,
        created: true,
        executed,
        listed,
        stopped,
        ordered,
    }
}

fn sandbox_calls_are_ordered(
    calls: &[common::ObservedFunctionCall],
    expected_name: &str,
    sandbox_id: &str,
) -> bool {
    let create = calls.iter().position(|call| {
        call.function_id == "sandbox::create"
            && call.arguments.get("name").and_then(Value::as_str) == Some(expected_name)
    });
    let exec = calls.iter().position(|call| {
        call.function_id == "sandbox::exec"
            && call.arguments.get("sandbox_id").and_then(Value::as_str) == Some(sandbox_id)
    });
    let list = calls
        .iter()
        .position(|call| call.function_id == "sandbox::list");
    let stop = calls.iter().position(|call| {
        call.function_id == "sandbox::stop"
            && call.arguments.get("sandbox_id").and_then(Value::as_str) == Some(sandbox_id)
    });
    matches!(
        (create, exec, list, stop),
        (Some(create), Some(exec), Some(list), Some(stop))
            if create < exec && exec < list && list < stop
    )
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let mut failures = Vec::new();
        if let Err(error) = cleanup_owned_sandbox(context, &sandbox_name(run_id)).await {
            failures.push(format!("sandbox cleanup: {error}"));
        }
        if let Err(error) = remove_workspace(&workspace_root(run_id)) {
            failures.push(format!("workspace cleanup: {error}"));
        }
        if !failures.is_empty() {
            bail!(failures.join("; "));
        }
        Ok(())
    })
}

async fn cleanup_owned_sandbox(context: &E2eContext, expected_name: &str) -> anyhow::Result<()> {
    if !context.function_exists("sandbox::list").await? {
        return Ok(());
    }
    let listed = context.trigger_value("sandbox::list", json!({})).await?;
    for sandbox_id in owned_running_sandbox_ids(&listed, expected_name) {
        let _: Value = context
            .trigger(
                "sandbox::stop",
                json!({ "sandbox_id": sandbox_id, "wait": true }),
            )
            .await?;
    }
    Ok(())
}

fn owned_running_sandbox_ids(listed: &Value, expected_name: &str) -> Vec<String> {
    listed
        .get("sandboxes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|sandbox| sandbox.get("name").and_then(Value::as_str) == Some(expected_name))
        .filter(|sandbox| sandbox.get("stopped").and_then(Value::as_bool) != Some(true))
        .filter_map(|sandbox| sandbox.get("sandbox_id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn remove_workspace(root: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn sandbox_name(run_id: &str) -> String {
    format!("e2e-shell-coder-{run_id}")
}

fn workspace_root(run_id: &str) -> PathBuf {
    let base = std::env::var_os("HARNESS_E2E_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let base = std::fs::canonicalize(&base).unwrap_or(base);
    base.join("scenario-workspaces")
        .join(format!("{ID}-{run_id}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn prompt_describes_all_operations_without_function_ids() {
        let spec = scenario("1234567890abcdef");
        assert!(!spec.prompt.contains("::"));
        assert!(spec.prompt.contains("iii-sandbox"));
        assert!(spec.prompt.contains(DRAFT_NAME));
        assert!(spec.prompt.contains(FINAL_NAME));
        assert!(spec.prompt.contains(HOST_STDOUT));
        assert!(spec.prompt.contains(SANDBOX_STDOUT));
        assert!(spec.prompt.contains("networking disabled"));
        assert!(spec
            .prompt
            .contains("Generic engine function discovery does not satisfy this step"));
        assert!(spec
            .prompt
            .contains("Do not create, edit, move, or read this code file with a general shell"));
    }

    #[test]
    fn recovered_function_errors_are_a_quality_signal() {
        let spec = scenario("1234567890abcdef");
        let execution_quality = spec
            .criteria
            .iter()
            .find(|criterion| criterion.id == "execution_quality")
            .expect("execution quality criterion");

        assert_eq!(spec.threshold, PASS_THRESHOLD);
        assert_eq!(execution_quality.weight, EXECUTION_QUALITY_WEIGHT);
        let total_weight = spec
            .criteria
            .iter()
            .map(|criterion| u16::from(criterion.weight))
            .sum::<u16>();
        let verified_outcome_weight = total_weight - u16::from(EXECUTION_QUALITY_WEIGHT);

        assert_eq!(total_weight, 100);
        assert_eq!(verified_outcome_weight, 55);
        assert!(verified_outcome_weight >= u16::from(PASS_THRESHOLD));
        assert_eq!(execution_quality_award(0), EXECUTION_QUALITY_WEIGHT);
        assert_eq!(execution_quality_award(1), 0);

        let aggregate_score = (100.0 + f64::from(verified_outcome_weight)) / 2.0;
        assert_eq!(aggregate_score, 77.5);
        assert!(aggregate_score >= f64::from(PASS_THRESHOLD));
    }

    #[test]
    fn recognizes_both_registry_installs() {
        let shell = common::ObservedFunctionCall {
            function_id: "worker::add".into(),
            arguments: json!({ "source": { "kind": "registry", "name": "shell" } }),
        };
        let sandbox = common::ObservedFunctionCall {
            function_id: "worker::add".into(),
            arguments: json!({ "source": { "kind": "registry", "name": "iii-sandbox" } }),
        };
        assert!(is_registry_install(&shell, "shell"));
        assert!(is_registry_install(&sandbox, "iii-sandbox"));
        assert!(!is_registry_install(&sandbox, "shell"));
    }

    #[test]
    fn workspace_paths_accept_relative_and_absolute_equivalents() {
        let root = Path::new("/tmp/e2e-shell-coder/workspace");

        assert!(workspace_path_matches(Some(DRAFT_NAME), root, DRAFT_NAME));
        assert!(workspace_path_matches(
            Some("./draft_check.py"),
            root,
            DRAFT_NAME
        ));
        assert!(workspace_path_matches(
            root.join(DRAFT_NAME).to_str(),
            root,
            DRAFT_NAME
        ));
        assert!(workspace_path_matches(
            root.join(FINAL_NAME).to_str(),
            root,
            FINAL_NAME
        ));
    }

    #[test]
    fn workspace_paths_reject_other_targets_and_parent_traversal() {
        let root = Path::new("/tmp/e2e-shell-coder/workspace");

        assert!(!workspace_path_matches(
            Some("other/draft_check.py"),
            root,
            DRAFT_NAME
        ));
        assert!(!workspace_path_matches(
            Some("checks/../checks/check.py"),
            root,
            FINAL_NAME
        ));
        assert!(!workspace_path_matches(
            Some("../workspace/draft_check.py"),
            root,
            DRAFT_NAME
        ));
        assert!(!workspace_path_matches(
            Some("/tmp/e2e-shell-coder/outside/draft_check.py"),
            root,
            DRAFT_NAME
        ));
        assert!(!workspace_path_matches(None, root, DRAFT_NAME));
    }

    #[test]
    fn exact_operations_accept_absolute_workspace_paths() {
        let root = Path::new("/tmp/e2e-shell-coder/workspace");
        let draft = root.join(DRAFT_NAME).to_string_lossy().into_owned();
        let final_path = root.join(FINAL_NAME).to_string_lossy().into_owned();
        let create = observed_call(
            "coder::create-file",
            json!({ "files": [{ "path": draft.clone(), "content": DRAFT_SCRIPT }] }),
        );
        let update = observed_call(
            "coder::update-file",
            json!({ "files": [{ "path": draft.clone(), "ops": [{ "op": "update_lines" }] }] }),
        );
        let move_call = observed_call(
            "coder::move",
            json!({ "files": [{ "from": draft, "to": final_path.clone() }] }),
        );
        let read = observed_call("coder::read-file", json!({ "path": final_path.clone() }));
        let host = observed_call(
            "shell::exec",
            json!({ "command": "python3", "args": [final_path] }),
        );

        assert!(is_exact_create(&create, root));
        assert!(is_exact_update(&update, root));
        assert!(is_exact_move(&move_call, root));
        assert!(is_exact_read(&read, root));
        assert!(is_exact_host_exec(&host, root));
    }

    #[test]
    fn exact_call_requires_its_own_successful_result() {
        let root = Path::new("/tmp/e2e-shell-coder/workspace");
        let transcript = json!({
            "messages": [
                invocation(
                    "call-create",
                    "coder::create-file",
                    json!({ "files": [{
                        "path": root.join(DRAFT_NAME).to_string_lossy(),
                        "content": DRAFT_SCRIPT
                    }] }),
                ),
                correlated_result(
                    "call-other",
                    "coder::create-file",
                    false,
                    json!({ "results": [{ "success": true }] }),
                )
            ]
        });
        let invocations = common::function_invocations(&transcript);

        assert!(invocations
            .iter()
            .any(|invocation| is_exact_create(&invocation.call, root)));
        assert!(!correlated_call_succeeded(
            &transcript,
            &invocations,
            |call| is_exact_create(call, root),
            batch_result_succeeded,
        ));

        let transcript = json!({
            "messages": [
                invocation(
                    "call-create",
                    "coder::create-file",
                    json!({ "files": [{
                        "path": root.join(DRAFT_NAME).to_string_lossy(),
                        "content": DRAFT_SCRIPT
                    }] }),
                ),
                correlated_result(
                    "call-create",
                    "coder::create-file",
                    false,
                    json!({ "results": [{ "success": true }] }),
                )
            ]
        });
        let invocations = common::function_invocations(&transcript);
        assert!(correlated_call_succeeded(
            &transcript,
            &invocations,
            |call| is_exact_create(call, root),
            batch_result_succeeded,
        ));
    }

    #[test]
    fn recognizes_successful_sandbox_lifecycle() {
        let name = "e2e-shell-coder-1234567890ab";
        let id = "sandbox-1";
        let calls = vec![
            common::ObservedFunctionCall {
                function_id: "sandbox::create".into(),
                arguments: json!({
                    "image": "python",
                    "name": name,
                    "cpus": 1,
                    "memory_mb": 256,
                    "network": false
                }),
            },
            common::ObservedFunctionCall {
                function_id: "sandbox::exec".into(),
                arguments: json!({ "sandbox_id": id, "cmd": "python3" }),
            },
            common::ObservedFunctionCall {
                function_id: "sandbox::list".into(),
                arguments: json!({}),
            },
            common::ObservedFunctionCall {
                function_id: "sandbox::stop".into(),
                arguments: json!({ "sandbox_id": id, "wait": true }),
            },
        ];
        let transcript = json!({
            "messages": [
                result("sandbox::create", json!({ "sandbox_id": id, "image": "python" })),
                result("sandbox::exec", json!({
                    "stdout": "sandbox-check:35\n",
                    "stderr": "",
                    "exit_code": 0,
                    "timed_out": false,
                    "success": true
                })),
                result("sandbox::list", json!({ "sandboxes": [{
                    "sandbox_id": id,
                    "name": name,
                    "image": "python",
                    "stopped": false
                }]})),
                result("sandbox::stop", json!({ "sandbox_id": id, "stopped": true }))
            ]
        });
        let evidence = sandbox_evidence(&transcript, &calls, name);
        assert!(evidence.complete(), "{}", evidence.summary());
    }

    #[test]
    fn cleanup_selects_only_owned_running_sandbox() {
        let listed = json!({
            "sandboxes": [
                { "sandbox_id": "owned", "name": "mine", "stopped": false },
                { "sandbox_id": "stopped", "name": "mine", "stopped": true },
                { "sandbox_id": "foreign", "name": "other", "stopped": false }
            ]
        });
        assert_eq!(
            owned_running_sandbox_ids(&listed, "mine"),
            vec!["owned".to_string()]
        );
    }

    #[test]
    fn workspace_root_and_sandbox_name_are_unique_per_run() {
        let first = workspace_root("first");
        let second = workspace_root("second");
        assert!(first.is_absolute());
        assert_ne!(first, second);
        assert_ne!(sandbox_name("first"), sandbox_name("second"));
    }

    fn result(function_id: &str, details: Value) -> Value {
        json!({
            "message": {
                "role": "function_result",
                "function_id": function_id,
                "is_error": false,
                "details": details,
                "content": []
            }
        })
    }

    fn observed_call(function_id: &str, arguments: Value) -> common::ObservedFunctionCall {
        common::ObservedFunctionCall {
            function_id: function_id.into(),
            arguments,
        }
    }

    fn invocation(call_id: &str, function_id: &str, payload: Value) -> Value {
        json!({
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "function_call",
                    "id": call_id,
                    "function_id": "agent_trigger",
                    "arguments": {
                        "function": function_id,
                        "payload": payload
                    }
                }]
            }
        })
    }

    fn correlated_result(
        call_id: &str,
        function_id: &str,
        is_error: bool,
        details: Value,
    ) -> Value {
        json!({
            "message": {
                "role": "function_result",
                "function_call_id": call_id,
                "function_id": function_id,
                "is_error": is_error,
                "details": details,
                "content": []
            }
        })
    }
}
