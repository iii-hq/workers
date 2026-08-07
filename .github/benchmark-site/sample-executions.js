(function loadHarnessExecutionPreview() {
  "use strict";

  const executionApi = window.HarnessExecutionData;
  const benchmark = window.HarnessBenchmarkData.normalizeBenchmarkData(
    window.BENCHMARK_DATA,
  );
  const derived = executionApi.mergeExecutionHistory(null, benchmark);
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
        execution.status === "passed" || execution.status === "quality_advisory"
          ? "success"
          : execution.status === "cancelled"
            ? "cancelled"
            : "failure",
      // Preview data intentionally uses aggregates only. The published Pages
      // contract never includes prompts, transcripts, responses, or tool payloads.
      availability: "aggregate",
      detail_path: null,
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

  window.HARNESS_EXECUTIONS = {
    schema_version: 3,
    last_update: executions[0]?.completed_at,
    repo_url: "https://github.com/iii-hq/workers",
    retention: { summaries: 100, details: 30 },
    executions,
  };
  window.HARNESS_EXECUTION_DETAILS = {};
})();
