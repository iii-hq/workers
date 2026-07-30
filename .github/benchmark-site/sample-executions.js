(function loadHarnessExecutionPreview() {
  "use strict";

  const benchmark = window.HarnessBenchmarkData.normalizeBenchmarkData(
    window.BENCHMARK_DATA,
  );
  const derived = window.HarnessExecutionData.mergeExecutionHistory(null, benchmark);
  const executions = derived.executions.map((execution, index) => {
    const runId = execution.run_id || String(30490000000 - index * 731);
    const id = execution.id || `${runId}-1`;
    return {
      ...execution,
      id,
      run_id: runId,
      attempt: 1,
      workflow_name: "Harness E2E Daily",
      workflow_url: `https://github.com/iii-hq/workers/actions/runs/${runId}`,
      event: index === 3 ? "workflow_dispatch" : "schedule",
      actor: index === 3 ? "iii-team" : "github-actions",
      conclusion:
        execution.status === "passed"
          ? "success"
          : execution.status === "cancelled"
            ? "cancelled"
            : "failure",
      availability: index < 3 ? "full" : "aggregate",
      detail_path: index < 3 ? `runs/${id}.json` : null,
      workflow_duration_seconds:
        (execution.totals?.wall_time_seconds || 0) + 104,
      execution: {
        id,
        run_id: runId,
        attempt: 1,
        event: index === 3 ? "workflow_dispatch" : "schedule",
        actor: index === 3 ? "iii-team" : "github-actions",
      },
    };
  });

  function modelArtifact(subject) {
    return {
      model: subject.model,
      provider: subject.provider,
      context_window: 1000000,
      max_output_tokens: 128000,
      supports_tools: true,
      supports_vision: false,
    };
  }

  function previewRun(execution, subject, scenario, runIndex) {
    const status = scenario.passed
      ? "passed"
      : scenario.hard_gate_failures
        ? "hard_gate_failed"
        : "quality_failed";
    const runId = `${execution.run_id.slice(-4)}-${scenario.id}-${runIndex + 1}`;
    return {
      run_id: runId,
      session_id: `e2e_${runId}`,
      prompt:
        `Run the ${scenario.id} E2E scenario against ${subject.provider}/${subject.model}. ` +
        "Use the registered tools, preserve the requested invariants, and report the evidence.",
      wall_time_ms: Math.round(
        ((scenario.wall_time_seconds || 30) * 1000) /
          Math.max(1, execution.requested_runs || 3),
      ),
      score: scenario.median_score,
      status,
      hard_gates: [
        {
          id: "required_state_present",
          passed: !scenario.hard_gate_failures,
          reason: scenario.hard_gate_failures
            ? "The expected state record was not found."
            : "All expected state records were present.",
        },
      ],
      criteria: [
        {
          id: "task_completion",
          possible: 60,
          awarded: Math.min(60, Math.round((scenario.median_score || 0) * 0.6)),
          reason: "Awarded from the observed task result and persisted evidence.",
        },
        {
          id: "tool_discipline",
          possible: 40,
          awarded: Math.min(40, Math.round((scenario.median_score || 0) * 0.4)),
          reason: "Awarded for bounded tool use and a factual final response.",
        },
      ],
      transcript: {
        messages: [
          {
            entry_id: "message-1",
            origin: "user",
            message: {
              role: "user",
              content: "Complete the benchmark task and record verifiable evidence.",
            },
          },
          {
            entry_id: "message-2",
            origin: "assistant",
            message: {
              role: "assistant",
              content:
                "I will inspect the available functions, execute the scenario, and verify the resulting state.",
            },
          },
          {
            entry_id: "function-1",
            custom: {
              type: "function_call",
              name: "state::set",
              status: scenario.passed ? "completed" : "failed",
            },
          },
        ],
      },
      metrics: {
        complete: true,
        root_session_id: `e2e_${runId}`,
        totals: {
          sessions: scenario.id === "reactive_automation" ? 7 : 1,
          turns: scenario.id === "reactive_automation" ? 122 : 9,
          function_calls: scenario.id === "reactive_automation" ? 151 : 6,
          function_call_errors: scenario.hard_gate_failures || 0,
          input_tokens: scenario.id === "reactive_automation" ? 1136424 : 18420,
          output_tokens: scenario.id === "reactive_automation" ? 22648 : 2180,
          cache_read_tokens: scenario.id === "reactive_automation" ? 1042368 : 12000,
          reasoning_tokens: 0,
          cost_usd:
            (scenario.total_cost_usd || 0) /
            Math.max(1, execution.requested_runs || 3),
        },
        by_session: [
          {
            session_id: `e2e_${runId}`,
            depth: 0,
            turns: 9,
            function_calls: 6,
            function_call_errors: scenario.hard_gate_failures || 0,
            input_tokens: 18420,
            output_tokens: 2180,
            cache_read_tokens: 12000,
            reasoning_tokens: 0,
            cost_usd:
              (scenario.total_cost_usd || 0) /
              Math.max(1, execution.requested_runs || 3),
          },
        ],
        traces: {
          trace_count: 3,
          span_count: 48,
          error_span_count: scenario.hard_gate_failures || 0,
          duration_ms: Math.round(scenario.wall_time_seconds * 1000),
          by_session: [],
        },
      },
      judge_attempts: scenario.passed ? 1 : 2,
      judge_usage: {
        input_tokens: 2900,
        output_tokens: 240,
        cost_usd: 0.014,
      },
      cost: {
        subject_usd: (scenario.total_cost_usd || 0) * 0.8,
        judge_usd: (scenario.total_cost_usd || 0) * 0.2,
        total_usd: scenario.total_cost_usd,
      },
      retry_attempts: Array.from({ length: scenario.retries || 0 }, (_, retryIndex) => ({
        run_id: `${runId}-retry-${retryIndex + 1}`,
        session_id: `retry_${runId}_${retryIndex + 1}`,
        wall_time_ms: 12000,
        status: "subject_error",
        cost: { subject_usd: 0.02, judge_usd: 0, total_usd: 0.02 },
        failures: [{ phase: "execute", message: "Transient provider timeout." }],
      })),
      failures: scenario.passed
        ? []
        : [
            {
              phase: scenario.hard_gate_failures ? "evaluate" : "collect",
              message: scenario.hard_gate_failures
                ? "A required hard gate did not pass."
                : "The score did not meet the scenario threshold.",
            },
          ],
    };
  }

  function previewDetail(execution) {
    const reports = [];
    execution.subjects.forEach((subject) => {
      subject.scenarios.forEach((scenario) => {
        const runs = Array.from(
          { length: execution.requested_runs || 3 },
          (_, index) => previewRun(execution, subject, scenario, index),
        );
        reports.push({
          subject_id: subject.id,
          scenario_id: scenario.id,
          available: true,
          report: {
            subject: modelArtifact(subject),
            judge: {
              model: subject.judge?.model || "claude-sonnet-4-6",
              provider: subject.judge?.provider || "anthropic",
              context_window: 1000000,
              max_output_tokens: 128000,
              supports_tools: true,
              supports_vision: true,
            },
            judge_protocol: "plain-json",
            engine_revision: subject.engine_revision,
            passed: scenario.passed,
            scenarios: [
              {
                scenario_id: scenario.id,
                threshold: scenario.threshold,
                execution_policy: {
                  max_turns: scenario.id === "reactive_automation" ? 64 : 16,
                  max_total_tokens:
                    scenario.id === "reactive_automation" ? 2000000 : 250000,
                  stuck_timeout_seconds: 600,
                },
                aggregate: {
                  runs: runs.length,
                  scored_runs: runs.filter((run) => run.score !== null).length,
                  passed_runs: runs.filter((run) => run.status === "passed").length,
                  required_passes: 2,
                  pass_rate: scenario.pass_rate,
                  median_score: scenario.median_score,
                  hard_gate_failures: scenario.hard_gate_failures,
                  technical_failures: scenario.technical_failures,
                  cost: {
                    subject_usd: (scenario.total_cost_usd || 0) * 0.8,
                    judge_usd: (scenario.total_cost_usd || 0) * 0.2,
                    total_usd: scenario.total_cost_usd,
                  },
                },
                passed: scenario.passed,
                runs,
              },
            ],
          },
        });
      });
    });
    return {
      schema_version: 2,
      execution: execution.execution,
      generated_at: execution.completed_at,
      lane: execution.lane,
      source: execution.source,
      workflow_url: execution.workflow_url,
      release: execution.release,
      requested_runs: execution.requested_runs,
      subjects: execution.subjects,
      reports,
    };
  }

  window.HARNESS_EXECUTIONS = {
    schema_version: 2,
    last_update: executions[0]?.completed_at,
    repo_url: "https://github.com/iii-hq/workers",
    retention: { summaries: 100, details: 30 },
    executions,
  };
  window.HARNESS_EXECUTION_DETAILS = Object.fromEntries(
    executions
      .filter((execution) => execution.availability === "full")
      .map((execution) => [execution.id, previewDetail(execution)]),
  );
})();
