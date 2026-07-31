(function initHarnessExecutionData(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  }
  root.HarnessExecutionData = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function executionDataFactory() {
  "use strict";

  const SCENARIO_METRIC_IDS = [
    "tokens",
    "duration_seconds",
    "cost_usd",
    "function_calls",
    "function_call_errors",
    "sessions",
    "turns",
  ];

  function numberOrNull(value) {
    return typeof value === "number" && Number.isFinite(value) ? value : null;
  }

  function mean(values) {
    const available = values.filter((value) => numberOrNull(value) !== null);
    return available.length
      ? available.reduce((total, value) => total + value, 0) / available.length
      : null;
  }

  function median(values) {
    const available = values
      .filter((value) => numberOrNull(value) !== null)
      .sort((left, right) => left - right);
    if (!available.length) return null;
    const middle = Math.floor(available.length / 2);
    return available.length % 2
      ? available[middle]
      : (available[middle - 1] + available[middle]) / 2;
  }

  function stableJson(value) {
    if (Array.isArray(value)) {
      return `[${value.map(stableJson).join(",")}]`;
    }
    if (value && typeof value === "object") {
      return `{${Object.keys(value)
        .sort()
        .map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`)
        .join(",")}}`;
    }
    return JSON.stringify(value);
  }

  function contractFingerprint(contract) {
    const text = stableJson(contract);
    const bytes = typeof TextEncoder === "function"
      ? new TextEncoder().encode(text)
      : Uint8Array.from(
          unescape(encodeURIComponent(text)),
          (character) => character.charCodeAt(0),
        );
    let value = 2_166_136_261;
    bytes.forEach((byte) => {
      value ^= byte;
      value = Math.imul(value, 16_777_619) >>> 0;
    });
    return `fnv1a32:${value.toString(16).padStart(8, "0")}`;
  }

  function scenarioContract(scenario, scenarioId, runs) {
    return {
      execution_policy:
        scenario?.execution_policy && typeof scenario.execution_policy === "object"
          ? scenario.execution_policy
          : {},
      scenario_id: scenarioId,
      scenario_version: Number(scenario?.scenario_version) || 1,
      threshold: numberOrNull(scenario?.threshold),
    };
  }

  function normalizeScenarioMetric(item) {
    const metric = item && typeof item === "object" ? item : {};
    return {
      ...metric,
      subject_id: String(metric.subject_id || ""),
      scenario_id: String(metric.scenario_id || ""),
      scenario_version: Number(metric.scenario_version) || 1,
      contract_fingerprint: String(metric.contract_fingerprint || ""),
      run_count: Number(metric.run_count) || 0,
      averages:
        metric.averages && typeof metric.averages === "object"
          ? metric.averages
          : {},
      samples:
        metric.samples && typeof metric.samples === "object"
          ? metric.samples
          : {},
    };
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
      scenario_metrics: Array.isArray(execution.scenario_metrics)
        ? execution.scenario_metrics
            .filter((item) => item && typeof item === "object")
            .map(normalizeScenarioMetric)
        : [],
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
      mode: raw.mode === "local" ? "local" : "published",
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

  function executionsWithinDays(executions, days, now = Date.now()) {
    const windowDays = Number(days);
    if (!Number.isFinite(windowDays) || windowDays <= 0) return [...(executions || [])];
    const windowEnd = Number(now);
    const windowStart = windowEnd - windowDays * 24 * 60 * 60 * 1000;
    return (executions || []).filter((execution) => {
      const timestamp = Date.parse(
        execution?.completed_at || execution?.started_at || "",
      );
      return Number.isFinite(timestamp) && timestamp >= windowStart && timestamp <= windowEnd;
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

  function runMetric(run, metricId) {
    const totals = run?.metrics?.totals || {};
    if (metricId === "tokens") {
      const input = numberOrNull(totals.input_tokens);
      const output = numberOrNull(totals.output_tokens);
      return input === null || output === null ? null : input + output;
    }
    if (metricId === "duration_seconds") {
      const wallTime = numberOrNull(run?.wall_time_ms);
      return wallTime === null ? null : wallTime / 1000;
    }
    if (metricId === "cost_usd") {
      return numberOrNull(run?.cost?.total_usd);
    }
    return numberOrNull(totals[metricId]);
  }

  function executionEfficiencyTotalsFromDetail(detail) {
    const tokens = [];
    const functionCalls = [];
    for (const reportEntry of detail?.reports || []) {
      if (!reportEntry?.available) continue;
      for (const scenario of reportEntry?.report?.scenarios || []) {
        for (const run of scenario?.runs || []) {
          const tokenValue = runMetric(run, "tokens");
          const callValue = runMetric(run, "function_calls");
          if (tokenValue !== null) tokens.push(tokenValue);
          if (callValue !== null) functionCalls.push(callValue);
        }
      }
    }
    return {
      total_tokens: tokens.length
        ? tokens.reduce((total, value) => total + value, 0)
        : null,
      function_calls: functionCalls.length
        ? functionCalls.reduce((total, value) => total + value, 0)
        : null,
    };
  }

  function scenarioMetricsFromDetail(detail) {
    const grouped = new Map();
    for (const reportEntry of detail?.reports || []) {
      if (!reportEntry?.available) continue;
      for (const scenario of reportEntry?.report?.scenarios || []) {
        const scenarioId = String(
          scenario?.scenario_id || reportEntry?.scenario_id || "",
        );
        if (!scenarioId) continue;
        const runs = Array.isArray(scenario?.runs)
          ? scenario.runs.filter((run) => run && typeof run === "object")
          : [];
        const subjectId = String(reportEntry?.subject_id || "");
        const key = `${subjectId}::${scenarioId}`;
        if (!grouped.has(key)) {
          grouped.set(key, { subjectId, scenarioId, scenario, runs: [] });
        }
        grouped.get(key).runs.push(...runs);
      }
    }
    return [...grouped.values()]
      .sort(
        (left, right) =>
          left.subjectId.localeCompare(right.subjectId) ||
          left.scenarioId.localeCompare(right.scenarioId),
      )
      .map(({ subjectId, scenarioId, scenario, runs }) => {
        const averages = {};
        const samples = {};
        SCENARIO_METRIC_IDS.forEach((metricId) => {
          const values = runs
            .map((run) => runMetric(run, metricId))
            .filter((value) => value !== null);
          averages[metricId] = mean(values);
          samples[metricId] = values.length;
        });
        const contract = scenarioContract(scenario, scenarioId, runs);
        return {
          subject_id: subjectId,
          scenario_id: scenarioId,
          scenario_version: contract.scenario_version,
          contract_fingerprint: contractFingerprint(contract),
          run_count: runs.length,
          averages,
          samples,
        };
      });
  }

  function scenarioMetricKey(metric) {
    return `${metric?.subject_id || ""}::${metric?.scenario_id || ""}`;
  }

  function scenarioResult(execution, metric) {
    const subjectId = String(metric?.subject_id || "");
    const subject = subjectId
      ? execution?.subjects?.find((item) => String(item.id || "") === subjectId)
      : execution?.subjects?.find((item) =>
          (item.scenarios || []).some(
            (scenario) => scenario.id === metric?.scenario_id,
          ),
        );
    const scenario = subject?.scenarios?.find(
      (item) => item.id === metric?.scenario_id,
    );
    if (!scenario) {
      return {
        passed: false,
        complete: false,
        hardGateFailures: 0,
        technicalFailures: 0,
      };
    }
    const hardGateFailures = Number(scenario.hard_gate_failures) || 0;
    const technicalFailures = Number(scenario.technical_failures) || 0;
    return {
      passed: Boolean(scenario.passed),
      complete: scenario.status !== "missing_report",
      hardGateFailures,
      technicalFailures,
      score: numberOrNull(scenario.median_score),
      passRate: numberOrNull(scenario.pass_rate),
      threshold: numberOrNull(scenario.threshold),
    };
  }

  function percentageDelta(current, baseline) {
    const currentValue = numberOrNull(current);
    const baselineValue = numberOrNull(baseline);
    if (currentValue === null || baselineValue === null || baselineValue === 0) {
      return null;
    }
    return ((currentValue - baselineValue) / Math.abs(baselineValue)) * 100;
  }

  function efficiencyTrend(row) {
    if (row.lifecycle !== "comparable") return row.lifecycle;
    if (!row.outcome.passed || !row.outcome.complete) return "non_comparable";
    if (!row.historyCount) return "collecting";
    if (!row.established) return "collecting";
    const primaryMetrics = [
      "cost_usd",
      "tokens",
      "duration_seconds",
      "function_call_errors",
    ];
    let improvements = 0;
    let regressions = 0;
    primaryMetrics.forEach((metricId) => {
      const current = numberOrNull(row.current?.averages?.[metricId]);
      const baseline = numberOrNull(row.baseline?.[metricId]);
      if (current === null || baseline === null) return;
      if (metricId === "function_call_errors" && baseline === 0) {
        if (current > 0) regressions += 1;
        return;
      }
      const delta = percentageDelta(current, baseline);
      if (delta !== null && delta <= -10) improvements += 1;
      if (delta !== null && delta >= 10) regressions += 1;
    });
    if (improvements && regressions) return "mixed";
    if (regressions) return "regressed";
    if (improvements) return "improving";
    return "stable";
  }

  function buildEfficiencyOverview(
    executions,
    { baselineWindow = 7, minimumHistory = 5 } = {},
  ) {
    const withMetrics = (executions || []).filter(
      (execution) => (execution.scenario_metrics || []).length,
    );
    const latest = withMetrics[0] || null;
    if (!latest) {
      return {
        latest: null,
        rows: [],
        metrics: {},
        counts: {},
        minimumHistory,
      };
    }
    const history = withMetrics.slice(1);
    const currentMetrics = new Map(
      latest.scenario_metrics.map((metric) => [scenarioMetricKey(metric), metric]),
    );
    const latestExpected = new Set(
      (latest.subjects || []).flatMap((subject) =>
        (subject.scenarios || []).map(
          (scenario) => `${subject.id || ""}::${scenario.id || ""}`,
        ),
      ),
    );
    const rows = [];

    currentMetrics.forEach((current, key) => {
      const sameScenario = history
        .map((execution) => ({
          execution,
          metric: (execution.scenario_metrics || []).find(
            (candidate) => scenarioMetricKey(candidate) === key,
          ),
        }))
        .filter((entry) => entry.metric);
      const fingerprint = current.contract_fingerprint;
      const matchingContract = sameScenario.filter(
        (entry) =>
          fingerprint &&
          entry.metric.contract_fingerprint &&
          entry.metric.contract_fingerprint === fingerprint,
      );
      const sameContract = matchingContract
        .filter((entry) => scenarioResult(entry.execution, entry.metric).passed)
        .slice(0, baselineWindow);
      const lifecycle = !sameScenario.length
        ? "new"
        : matchingContract.length
          ? "comparable"
          : "changed";
      const baseline = Object.fromEntries(
        SCENARIO_METRIC_IDS.map((metricId) => [
          metricId,
          median(
            sameContract.map((entry) =>
              numberOrNull(entry.metric?.averages?.[metricId]),
            ),
          ),
        ]),
      );
      const row = {
        key,
        subjectId: current.subject_id,
        scenarioId: current.scenario_id,
        scenarioVersion: current.scenario_version,
        fingerprint,
        lifecycle,
        current,
        baseline,
        historyCount: sameContract.length,
        established: sameContract.length >= minimumHistory,
        outcome: scenarioResult(latest, current),
      };
      row.deltas = Object.fromEntries(
        SCENARIO_METRIC_IDS.map((metricId) => [
          metricId,
          percentageDelta(current.averages?.[metricId], baseline[metricId]),
        ]),
      );
      row.trend = efficiencyTrend(row);
      rows.push(row);
    });

    const latestHistorical = new Map();
    history.forEach((execution) => {
      (execution.scenario_metrics || []).forEach((metric) => {
        const key = scenarioMetricKey(metric);
        if (!currentMetrics.has(key) && !latestHistorical.has(key)) {
          latestHistorical.set(key, { execution, metric });
        }
      });
    });
    latestHistorical.forEach(({ execution, metric }, key) => {
      const expected = latestExpected.has(key);
      rows.push({
        key,
        subjectId: metric.subject_id,
        scenarioId: metric.scenario_id,
        scenarioVersion: metric.scenario_version,
        fingerprint: metric.contract_fingerprint,
        lifecycle: expected ? "non_comparable" : "retired",
        current: null,
        baseline: metric.averages || {},
        deltas: {},
        historyCount: 0,
        established: false,
        outcome: expected
          ? {
              passed: false,
              complete: false,
              hardGateFailures: 0,
              technicalFailures: 0,
            }
          : scenarioResult(execution, metric),
        trend: expected ? "non_comparable" : "retired",
      });
    });

    rows.sort(
      (left, right) =>
        left.scenarioId.localeCompare(right.scenarioId) ||
        left.subjectId.localeCompare(right.subjectId),
    );
    const operationalRows = rows.filter((row) => row.current);
    const comparableRows = rows.filter(
      (row) => row.lifecycle === "comparable" && row.outcome.passed,
    );
    const metrics = Object.fromEntries(
      SCENARIO_METRIC_IDS.map((metricId) => {
        const operational = operationalRows.reduce(
          (total, row) =>
            total + (numberOrNull(row.current?.averages?.[metricId]) || 0),
          0,
        );
        const comparableCurrent = comparableRows.reduce(
          (total, row) =>
            total + (numberOrNull(row.current?.averages?.[metricId]) || 0),
          0,
        );
        const comparableBaseline = comparableRows.reduce(
          (total, row) =>
            total + (numberOrNull(row.baseline?.[metricId]) || 0),
          0,
        );
        return [
          metricId,
          {
            operational,
            comparableCurrent,
            comparableBaseline,
            delta: percentageDelta(comparableCurrent, comparableBaseline),
          },
        ];
      }),
    );
    const counts = {
      active: operationalRows.length,
      comparable: comparableRows.length,
      new: rows.filter((row) => row.lifecycle === "new").length,
      changed: rows.filter((row) => row.lifecycle === "changed").length,
      retired: rows.filter((row) => row.lifecycle === "retired").length,
      nonComparable: rows.filter(
        (row) =>
          row.lifecycle === "non_comparable" ||
          (row.current && !row.outcome.passed),
      ).length,
      established: rows.filter((row) => row.established).length,
    };
    return { latest, rows, metrics, counts, minimumHistory };
  }

  function scenarioMetricSeries(executions, metricId, scenarioId = "all") {
    const series = new Map();
    const orderedExecutions = [...(executions || [])].sort((left, right) => {
      const leftDate = Date.parse(left?.completed_at || left?.started_at || "") || 0;
      const rightDate =
        Date.parse(right?.completed_at || right?.started_at || "") || 0;
      return leftDate - rightDate;
    });

    for (const execution of orderedExecutions) {
      for (const scenario of execution?.scenario_metrics || []) {
        const currentScenarioId = String(scenario?.scenario_id || "");
        if (
          !currentScenarioId ||
          (scenarioId !== "all" && currentScenarioId !== scenarioId)
        ) {
          continue;
        }
        const value = numberOrNull(scenario?.averages?.[metricId]);
        if (value === null) continue;
        if (!series.has(currentScenarioId)) {
          series.set(currentScenarioId, {
            scenarioId: currentScenarioId,
            points: [],
          });
        }
        series.get(currentScenarioId).points.push({
          executionId: execution.id,
          runId: execution.run_id || execution.id,
          attempt: execution.attempt || 1,
          timestamp: execution.completed_at || execution.started_at || "",
          value,
          subjectId: scenario.subject_id || "",
          scenarioVersion: scenario.scenario_version || 1,
          contractFingerprint: scenario.contract_fingerprint || "",
          runSamples: Number(scenario?.samples?.[metricId]) || 0,
          execution,
        });
      }
    }

    return [...series.values()].sort((left, right) =>
      left.scenarioId.localeCompare(right.scenarioId),
    );
  }

  function scenarioMetricRows(executions) {
    const grouped = new Map();
    for (const execution of executions || []) {
      for (const scenario of execution?.scenario_metrics || []) {
        const scenarioId = String(scenario?.scenario_id || "");
        if (!scenarioId) continue;
        if (!grouped.has(scenarioId)) {
          grouped.set(scenarioId, {
            scenarioId,
            executionIds: new Set(),
            runCount: 0,
            values: Object.fromEntries(
              SCENARIO_METRIC_IDS.map((metricId) => [metricId, []]),
            ),
            runSamples: Object.fromEntries(
              SCENARIO_METRIC_IDS.map((metricId) => [metricId, 0]),
            ),
          });
        }
        const row = grouped.get(scenarioId);
        row.executionIds.add(execution.id);
        row.runCount += Number(scenario.run_count) || 0;
        SCENARIO_METRIC_IDS.forEach((metricId) => {
          const value = numberOrNull(scenario?.averages?.[metricId]);
          if (value !== null) row.values[metricId].push(value);
          row.runSamples[metricId] += Number(scenario?.samples?.[metricId]) || 0;
        });
      }
    }
    return [...grouped.values()]
      .sort((left, right) => left.scenarioId.localeCompare(right.scenarioId))
      .map((row) => ({
        scenarioId: row.scenarioId,
        executionCount: row.executionIds.size,
        runCount: row.runCount,
        averages: Object.fromEntries(
          SCENARIO_METRIC_IDS.map((metricId) => [
            metricId,
            mean(row.values[metricId]),
          ]),
        ),
        executionSamples: Object.fromEntries(
          SCENARIO_METRIC_IDS.map((metricId) => [
            metricId,
            row.values[metricId].length,
          ]),
        ),
        runSamples: row.runSamples,
      }));
  }

  function findExecution(history, id) {
    return history?.executions?.find((execution) => execution.id === id) || null;
  }

  return {
    buildEfficiencyOverview,
    contractFingerprint,
    executionEfficiencyTotalsFromDetail,
    executionsWithinDays,
    filterExecutions,
    findExecution,
    legacyExecution,
    matrixCell,
    matrixCellLabel,
    matrixRows,
    mergeExecutionHistory,
    normalizeExecution,
    normalizeStatus,
    scenarioMetricSeries,
    scenarioMetricRows,
    scenarioMetricsFromDetail,
    scenarioContract,
  };
});
