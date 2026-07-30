(function initHarnessExecutionData(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  }
  root.HarnessExecutionData = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function executionDataFactory() {
  "use strict";

  function numberOrNull(value) {
    return typeof value === "number" && Number.isFinite(value) ? value : null;
  }

  function metricValue(subject, category, scenarioId, metricId) {
    return subject?.metrics?.[category]?.[scenarioId]?.[metricId]?.value ?? null;
  }

  function listLegacyScenarios(subject) {
    const scenarios = new Set();
    for (const category of Object.values(subject?.metrics || {})) {
      for (const scenarioId of Object.keys(category || {})) {
        if (scenarioId !== "suite") scenarios.add(scenarioId);
      }
    }
    return [...scenarios].sort();
  }

  function normalizeStatus(value) {
    if (["passed", "failed", "incomplete", "cancelled"].includes(value)) return value;
    if (value === "pass" || value === "success") return "passed";
    if (value === "fail" || value === "failure") return "failed";
    return "incomplete";
  }

  function normalizeExecution(entry) {
    const execution = entry && typeof entry === "object" ? entry : {};
    const subjects = Array.isArray(execution.subjects)
      ? execution.subjects.filter((subject) => subject && typeof subject === "object")
      : [];
    return {
      ...execution,
      id: String(execution.id || ""),
      run_id: String(execution.run_id || ""),
      attempt: Number(execution.attempt) || 1,
      status: normalizeStatus(execution.status),
      conclusion: String(execution.conclusion || ""),
      event: String(execution.event || ""),
      actor: String(execution.actor || ""),
      workflow_url: String(execution.workflow_url || ""),
      started_at: String(execution.started_at || execution.generated_at || ""),
      completed_at: String(execution.completed_at || execution.generated_at || ""),
      availability: ["full", "aggregate", "unavailable"].includes(execution.availability)
        ? execution.availability
        : subjects.length
          ? "aggregate"
          : "unavailable",
      detail_path: execution.detail_path || null,
      source: execution.source && typeof execution.source === "object"
        ? execution.source
        : {},
      release: execution.release && typeof execution.release === "object"
        ? execution.release
        : {},
      subjects,
      totals: execution.totals && typeof execution.totals === "object"
        ? execution.totals
        : {},
    };
  }

  function legacySubject(subject) {
    const scenarios = listLegacyScenarios(subject).map((scenarioId) => {
      const score = subject.metrics?.quality?.[scenarioId]?.median_score;
      const passRate = subject.metrics?.quality?.[scenarioId]?.pass_rate;
      return {
        id: scenarioId,
        status: score?.status || passRate?.status || "unknown",
        passed: score?.passed ?? passRate?.passed ?? false,
        threshold: score?.threshold ?? passRate?.threshold ?? null,
        median_score: score?.value ?? null,
        pass_rate: passRate?.value === null || passRate?.value === undefined
          ? null
          : passRate.value / 100,
        hard_gate_failures:
          metricValue(subject, "reliability", scenarioId, "hard_gate_failures") ?? 0,
        technical_failures:
          metricValue(subject, "reliability", scenarioId, "technical_failures") ?? 0,
        retries: metricValue(subject, "reliability", scenarioId, "retry_attempts") ?? 0,
        total_cost_usd:
          metricValue(subject, "efficiency", scenarioId, "total_cost_usd"),
        wall_time_seconds:
          metricValue(subject, "efficiency", scenarioId, "wall_time_seconds"),
      };
    });
    return {
      id: subject.id,
      model: subject.model,
      provider: subject.provider,
      judge: subject.judge || {},
      engine_revision: subject.engineRevision || "",
      passed: Boolean(subject.passed),
      expected_reports: scenarios.length,
      received_reports: scenarios.filter((scenario) => scenario.status !== "missing_report")
        .length,
      scenario_pass_rate:
        (metricValue(subject, "quality", "suite", "scenario_pass_rate") ?? 0) / 100,
      report_coverage:
        (metricValue(subject, "quality", "suite", "report_coverage") ?? 0) / 100,
      hard_gate_failures:
        metricValue(subject, "reliability", "suite", "hard_gate_failures") ?? 0,
      technical_failures:
        metricValue(subject, "reliability", "suite", "technical_failures") ?? 0,
      retry_attempts:
        metricValue(subject, "reliability", "suite", "retry_attempts") ?? 0,
      total_cost_usd:
        metricValue(subject, "efficiency", "suite", "total_cost_usd"),
      wall_time_seconds:
        metricValue(subject, "efficiency", "suite", "wall_time_seconds"),
      scenarios,
    };
  }

  function legacyExecution(snapshot) {
    const subjects = Object.values(snapshot.subjects || {}).map(legacySubject);
    const scenarios = subjects.flatMap((subject) => subject.scenarios);
    const scores = scenarios
      .map((scenario) => numberOrNull(scenario.median_score))
      .filter((value) => value !== null);
    const expected = subjects.reduce(
      (total, subject) => total + Number(subject.expected_reports || 0),
      0,
    );
    const received = subjects.reduce(
      (total, subject) => total + Number(subject.received_reports || 0),
      0,
    );
    const complete = expected > 0 && expected === received;
    const passed = complete && subjects.every((subject) => subject.passed);
    const executionId = snapshot.execution?.id || snapshot.id;
    const runId = snapshot.execution?.run_id || "";
    return normalizeExecution({
      id: executionId,
      run_id: runId,
      attempt: snapshot.execution?.attempt || 1,
      workflow_url: snapshot.workflowUrl,
      started_at: snapshot.generatedAt || new Date(snapshot.date).toISOString(),
      completed_at: snapshot.generatedAt || new Date(snapshot.date).toISOString(),
      event: snapshot.execution?.event || "legacy",
      actor: snapshot.execution?.actor || "",
      conclusion: passed ? "success" : "failure",
      status: complete ? (passed ? "passed" : "failed") : "incomplete",
      availability: "aggregate",
      detail_path: null,
      generated_at: snapshot.generatedAt,
      lane: snapshot.lane,
      source: snapshot.source,
      release: snapshot.release,
      subjects,
      totals: {
        expected_reports: expected,
        received_reports: received,
        report_coverage: expected ? (received / expected) * 100 : 0,
        passed_scenarios: scenarios.filter((scenario) => scenario.passed).length,
        scenario_pass_rate: expected
          ? (scenarios.filter((scenario) => scenario.passed).length / expected) * 100
          : 0,
        average_score: scores.length
          ? scores.reduce((total, score) => total + score, 0) / scores.length
          : null,
        total_cost_usd: subjects.every(
          (subject) => numberOrNull(subject.total_cost_usd) !== null,
        )
          ? subjects.reduce((total, subject) => total + subject.total_cost_usd, 0)
          : null,
        wall_time_seconds: subjects.every(
          (subject) => numberOrNull(subject.wall_time_seconds) !== null,
        )
          ? subjects.reduce((total, subject) => total + subject.wall_time_seconds, 0)
          : null,
        hard_gate_failures: subjects.reduce(
          (total, subject) => total + Number(subject.hard_gate_failures || 0),
          0,
        ),
        technical_failures: subjects.reduce(
          (total, subject) => total + Number(subject.technical_failures || 0),
          0,
        ),
        missing_reports: Math.max(0, expected - received),
        retries: subjects.reduce(
          (total, subject) => total + Number(subject.retry_attempts || 0),
          0,
        ),
      },
    });
  }

  function mergeExecutionHistory(manifest, benchmarkData) {
    const raw = manifest && typeof manifest === "object" ? manifest : {};
    const byId = new Map(
      (Array.isArray(raw.executions) ? raw.executions : [])
        .map(normalizeExecution)
        .filter((entry) => entry.id)
        .map((entry) => [entry.id, entry]),
    );
    for (const snapshot of benchmarkData?.snapshots || []) {
      const id = snapshot.execution?.id || snapshot.id;
      if (!byId.has(id)) byId.set(id, legacyExecution(snapshot));
    }
    const executions = [...byId.values()].sort(
      (left, right) =>
        Date.parse(right.completed_at || right.started_at || 0) -
        Date.parse(left.completed_at || left.started_at || 0),
    );
    return {
      schemaVersion: Number(raw.schema_version) || 1,
      lastUpdate: raw.last_update || benchmarkData?.lastUpdate || "",
      repoUrl: raw.repo_url || benchmarkData?.repoUrl || "",
      preview: Boolean(globalThis.HARNESS_BENCHMARK_PREVIEW),
      retention: raw.retention || { summaries: 100, details: 30 },
      executions,
    };
  }

  function filterExecutions(executions, filters = {}) {
    const query = String(filters.query || "").trim().toLowerCase();
    return (executions || []).filter((execution) => {
      if (filters.status && filters.status !== "all" && execution.status !== filters.status) {
        return false;
      }
      if (filters.event && filters.event !== "all" && execution.event !== filters.event) {
        return false;
      }
      if (!query) return true;
      const haystack = [
        execution.id,
        execution.run_id,
        execution.source?.sha,
        execution.source?.ref,
        execution.completed_at,
        execution.started_at,
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  }

  function matrixRows(executions) {
    const rows = new Map();
    for (const execution of executions || []) {
      for (const subject of execution.subjects || []) {
        for (const scenario of subject.scenarios || []) {
          const key = `${subject.id}::${scenario.id}`;
          if (!rows.has(key)) {
            rows.set(key, {
              key,
              subjectId: subject.id,
              subjectLabel: `${subject.provider || ""}/${subject.model || subject.id}`.replace(
                /^\//,
                "",
              ),
              scenarioId: scenario.id,
            });
          }
        }
      }
    }
    return [...rows.values()].sort(
      (left, right) =>
        left.subjectLabel.localeCompare(right.subjectLabel) ||
        left.scenarioId.localeCompare(right.scenarioId),
    );
  }

  function matrixCell(execution, row) {
    const subject = execution?.subjects?.find((item) => item.id === row.subjectId);
    const scenario = subject?.scenarios?.find((item) => item.id === row.scenarioId);
    if (!scenario) return null;
    const status =
      scenario.status === "missing_report"
        ? "incomplete"
        : scenario.passed
          ? "passed"
          : "failed";
    return { ...scenario, status };
  }

  function matrixCellLabel(cell, status) {
    if (status === "failed") return "×";
    if (status === "cancelled") return "○";
    if (status !== "passed") return "–";

    const score = numberOrNull(cell?.median_score);
    const passRate = numberOrNull(cell?.pass_rate);
    const percentage = score ?? (passRate === null ? null : passRate * 100);
    if (percentage === null) return "—";

    const rounded = Math.round(percentage * 10) / 10;
    return `${rounded.toLocaleString("en-US", {
      maximumFractionDigits: 1,
    })}%`;
  }

  function findExecution(history, id) {
    return history?.executions?.find((execution) => execution.id === id) || null;
  }

  return {
    filterExecutions,
    findExecution,
    legacyExecution,
    matrixCell,
    matrixCellLabel,
    matrixRows,
    mergeExecutionHistory,
    normalizeExecution,
    normalizeStatus,
  };
});
