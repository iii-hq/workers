const test = require("node:test");
const assert = require("node:assert/strict");

const {
  buildEfficiencyOverview,
  contractFingerprint,
  executionEfficiencyTotalsFromDetail,
  executionsWithinDays,
  filterExecutions,
  findExecution,
  matrixCell,
  matrixCellLabel,
  matrixRows,
  mergeExecutionHistory,
  normalizeExecution,
  scenarioMetricSeries,
  scenarioMetricRows,
  scenarioMetricsFromDetail,
  scenarioContract,
} = require("./execution-data.js");

function execution(overrides = {}) {
  return {
    id: "123-1",
    run_id: "123",
    attempt: 1,
    status: "passed",
    conclusion: "success",
    event: "schedule",
    completed_at: "2026-07-29T06:10:00Z",
    source: { sha: "a".repeat(40), ref: "main" },
    availability: "full",
    detail_path: "runs/123-1.json",
    totals: { average_score: 92, scenario_pass_rate: 100 },
    subjects: [
      {
        id: "glm",
        model: "glm-5.2",
        provider: "zai",
        scenarios: [
          {
            id: "direct_answer",
            passed: true,
            status: "passed",
            median_score: 92,
            pass_rate: 1,
          },
        ],
      },
    ],
    ...overrides,
  };
}

test("normalizes execution status and availability", () => {
  const normalized = normalizeExecution({
    id: 99,
    status: "failure",
    subjects: [],
  });

  assert.equal(normalized.id, "99");
  assert.equal(normalized.status, "failed");
  assert.equal(normalized.availability, "unavailable");
});

test("merges manifest executions and finds a retained detail", () => {
  const history = mergeExecutionHistory(
    {
      schema_version: 2,
      last_update: "2026-07-29T06:10:00Z",
      executions: [execution()],
    },
    { snapshots: [] },
  );

  assert.equal(history.executions.length, 1);
  assert.equal(findExecution(history, "123-1").detail_path, "runs/123-1.json");
});

test("keeps workflow attempts distinct and newest first", () => {
  const history = mergeExecutionHistory(
    {
      executions: [
        execution({ id: "123-1", attempt: 1 }),
        execution({
          id: "123-2",
          attempt: 2,
          completed_at: "2026-07-29T06:20:00Z",
        }),
      ],
    },
    { snapshots: [] },
  );

  assert.deepEqual(
    history.executions.map((item) => item.id),
    ["123-2", "123-1"],
  );
});

test("filters by status, trigger, run id, commit, and date", () => {
  const entries = [
    execution(),
    execution({
      id: "456-1",
      run_id: "456",
      status: "failed",
      conclusion: "failure",
      event: "workflow_dispatch",
      completed_at: "2026-07-30T07:15:00Z",
      source: { sha: "b".repeat(40), ref: "main" },
    }),
  ];

  assert.deepEqual(
    filterExecutions(entries, { status: "failed" }).map((item) => item.id),
    ["456-1"],
  );
  assert.deepEqual(
    filterExecutions(entries, { event: "schedule" }).map((item) => item.id),
    ["123-1"],
  );
  assert.equal(filterExecutions(entries, { query: "456" }).length, 1);
  assert.equal(filterExecutions(entries, { query: "bbbbbbb" }).length, 1);
  assert.equal(filterExecutions(entries, { query: "2026-07-29" }).length, 1);
});

test("filters execution metrics to a rolling day window", () => {
  const now = Date.parse("2026-07-30T12:00:00Z");
  const entries = [
    execution({ id: "recent", completed_at: "2026-07-29T12:00:00Z" }),
    execution({ id: "edge", completed_at: "2026-06-30T12:00:00Z" }),
    execution({ id: "old", completed_at: "2026-06-30T11:59:59Z" }),
  ];

  assert.deepEqual(
    executionsWithinDays(entries, 30, now).map((item) => item.id),
    ["recent", "edge"],
  );
});

test("builds subject and scenario matrix rows with result cells", () => {
  const entry = execution();
  const rows = matrixRows([entry]);
  const cell = matrixCell(entry, rows[0]);

  assert.deepEqual(rows, [
    {
      key: "glm::direct_answer",
      subjectId: "glm",
      subjectLabel: "zai/glm-5.2",
      scenarioId: "direct_answer",
    },
  ]);
  assert.equal(cell.status, "passed");
  assert.equal(cell.median_score, 92);
  assert.equal(matrixCellLabel(cell, cell.status), "92%");
  assert.equal(matrixCellLabel({ passed: false }, "failed"), "×");
  assert.equal(matrixCellLabel(null, "incomplete"), "–");
  assert.equal(matrixCellLabel(null, "cancelled"), "○");
  assert.equal(
    matrixCellLabel({ passed: true, median_score: null, pass_rate: 0.875 }, "passed"),
    "87.5%",
  );
});

test("derives aggregate legacy entries from benchmark snapshots", () => {
  const history = mergeExecutionHistory(null, {
    lastUpdate: 1,
    snapshots: [
      {
        id: "daily::legacy",
        date: Date.UTC(2026, 6, 28, 6),
        lane: "daily",
        execution: {},
        workflowUrl: "https://github.com/iii-hq/workers/actions/runs/legacy",
        source: { sha: "c".repeat(40), ref: "main" },
        subjects: {
          glm: {
            id: "glm",
            model: "glm-5.2",
            provider: "zai",
            passed: true,
            metrics: {
              quality: {
                direct_answer: {
                  median_score: { value: 90, passed: true, status: "passed" },
                  pass_rate: { value: 100, passed: true, status: "passed" },
                },
                suite: {
                  scenario_pass_rate: { value: 100 },
                  report_coverage: { value: 100 },
                },
              },
              reliability: { suite: {} },
              efficiency: { suite: {} },
            },
          },
        },
      },
    ],
  });

  assert.equal(history.executions[0].availability, "aggregate");
  assert.equal(history.executions[0].totals.average_score, 90);
  assert.equal(history.executions[0].subjects[0].scenarios[0].id, "direct_answer");
});

test("derives per-scenario run averages from a full execution detail", () => {
  const detail = {
    reports: [
      {
        available: true,
        subject_id: "glm",
        scenario_id: "direct_answer",
        report: {
          scenarios: [
            {
              scenario_id: "direct_answer",
              scenario_version: 1,
              threshold: 50,
              execution_policy: {
                max_turns: 2,
                max_total_tokens: 32768,
              },
              runs: [
                {
                  run_id: "first",
                  prompt: "answer for first",
                  wall_time_ms: 10_000,
                  cost: { total_usd: 0.2 },
                  metrics: {
                    totals: {
                      input_tokens: 100,
                      output_tokens: 20,
                      function_calls: 2,
                      function_call_errors: 0,
                      sessions: 1,
                      turns: 4,
                    },
                  },
                },
                {
                  run_id: "second",
                  prompt: "answer for second",
                  wall_time_ms: 20_000,
                  cost: { total_usd: 0.6 },
                  metrics: {
                    totals: {
                      input_tokens: 200,
                      output_tokens: 40,
                      function_calls: 4,
                      function_call_errors: 2,
                      sessions: 3,
                      turns: 8,
                    },
                  },
                },
              ],
            },
          ],
        },
      },
    ],
  };
  const metrics = scenarioMetricsFromDetail(detail);
  const scenario = detail.reports[0].report.scenarios[0];

  assert.deepEqual(executionEfficiencyTotalsFromDetail(detail), {
    total_tokens: 360,
    function_calls: 6,
  });

  assert.deepEqual(metrics, [
    {
      subject_id: "glm",
      scenario_id: "direct_answer",
      scenario_version: 1,
      contract_fingerprint: contractFingerprint(
        scenarioContract(scenario, "direct_answer", scenario.runs),
      ),
      run_count: 2,
      averages: {
        tokens: 180,
        duration_seconds: 15,
        cost_usd: 0.4,
        function_calls: 3,
        function_call_errors: 1,
        sessions: 2,
        turns: 6,
      },
      samples: {
        tokens: 2,
        duration_seconds: 2,
        cost_usd: 2,
        function_calls: 2,
        function_call_errors: 2,
        sessions: 2,
        turns: 2,
      },
    },
  ]);
});

test("averages scenario metrics with equal weight per execution", () => {
  const rows = scenarioMetricRows([
    execution({
      id: "first",
      scenario_metrics: [
        {
          scenario_id: "direct_answer",
          run_count: 3,
          averages: { tokens: 100, cost_usd: 0.2, turns: 4 },
          samples: { tokens: 3, cost_usd: 3, turns: 3 },
        },
      ],
    }),
    execution({
      id: "second",
      scenario_metrics: [
        {
          scenario_id: "direct_answer",
          run_count: 2,
          averages: { tokens: 300, cost_usd: null, turns: 8 },
          samples: { tokens: 2, cost_usd: 0, turns: 2 },
        },
      ],
    }),
  ]);

  assert.equal(rows[0].averages.tokens, 200);
  assert.equal(rows[0].averages.cost_usd, 0.2);
  assert.equal(rows[0].averages.turns, 6);
  assert.equal(rows[0].executionCount, 2);
  assert.equal(rows[0].executionSamples.tokens, 2);
  assert.equal(rows[0].executionSamples.cost_usd, 1);
  assert.equal(rows[0].runCount, 5);
  assert.equal(rows[0].runSamples.tokens, 5);
});

test("keeps scenario metrics as one point per workflow execution", () => {
  const series = scenarioMetricSeries(
    [
      execution({
        id: "newer",
        run_id: "200",
        completed_at: "2026-07-30T12:00:00Z",
        scenario_metrics: [
          {
            scenario_id: "direct_answer",
            averages: { tokens: 300 },
            samples: { tokens: 3 },
          },
        ],
      }),
      execution({
        id: "older",
        run_id: "100",
        completed_at: "2026-07-29T12:00:00Z",
        scenario_metrics: [
          {
            scenario_id: "direct_answer",
            averages: { tokens: 100 },
            samples: { tokens: 2 },
          },
          {
            scenario_id: "security_review",
            averages: { tokens: 200 },
            samples: { tokens: 2 },
          },
        ],
      }),
    ],
    "tokens",
  );

  assert.deepEqual(
    series.map((item) => ({
      scenarioId: item.scenarioId,
      executionIds: item.points.map((point) => point.executionId),
      values: item.points.map((point) => point.value),
    })),
    [
      {
        scenarioId: "direct_answer",
        executionIds: ["older", "newer"],
        values: [100, 300],
      },
      {
        scenarioId: "security_review",
        executionIds: ["older"],
        values: [200],
      },
    ],
  );
  assert.equal(
    scenarioMetricSeries(
      [execution({ scenario_metrics: series.map(() => null) })],
      "tokens",
      "missing",
    ).length,
    0,
  );
});

test("compares efficiency only within the same scenario contract", () => {
  const metric = (
    scenarioId,
    fingerprint,
    tokens,
    cost,
    duration,
    errors = 0,
  ) => ({
    subject_id: "glm",
    scenario_id: scenarioId,
    scenario_version: fingerprint === "v2" ? 2 : 1,
    contract_fingerprint: fingerprint,
    run_count: 3,
    averages: {
      tokens,
      cost_usd: cost,
      duration_seconds: duration,
      function_calls: 2,
      function_call_errors: errors,
      sessions: 1,
      turns: 2,
    },
    samples: {},
  });
  const withScenarios = (id, scenarioMetrics, scenarios) =>
    execution({
      id,
      completed_at:
        id === "latest" ? "2026-07-30T12:00:00Z" : "2026-07-29T12:00:00Z",
      scenario_metrics: scenarioMetrics,
      subjects: [
        {
          id: "glm",
          model: "glm-5.2",
          provider: "zai",
          scenarios: scenarios.map(([scenarioId, passed]) => ({
            id: scenarioId,
            passed,
            status: passed ? "passed" : "hard_gate_failed",
            hard_gate_failures: passed ? 0 : 1,
            technical_failures: 0,
          })),
        },
      ],
    });

  const overview = buildEfficiencyOverview(
    [
      withScenarios(
      "latest",
      [
        metric("stable", "v1", 90, 0.9, 8),
        metric("changed", "v2", 80, 0.8, 7),
        metric("new_case", "v1", 20, 0.2, 2),
        metric("failed", "v1", 30, 0.3, 3, 1),
      ],
      [
        ["stable", true],
        ["changed", true],
        ["new_case", true],
        ["failed", false],
      ],
    ),
      withScenarios(
      "previous",
      [
        metric("stable", "v1", 100, 1, 10),
        metric("changed", "v1", 70, 0.7, 6),
        metric("retired", "v1", 40, 0.4, 4),
        metric("failed", "v1", 40, 0.4, 4),
      ],
      [
        ["stable", true],
        ["changed", true],
        ["retired", true],
        ["failed", true],
      ],
      ),
    ],
    { minimumHistory: 1 },
  );

  assert.equal(overview.rows.find((row) => row.scenarioId === "stable").trend, "improving");
  assert.equal(overview.rows.find((row) => row.scenarioId === "changed").lifecycle, "changed");
  assert.equal(overview.rows.find((row) => row.scenarioId === "new_case").lifecycle, "new");
  assert.equal(overview.rows.find((row) => row.scenarioId === "retired").lifecycle, "retired");
  assert.equal(
    overview.rows.find((row) => row.scenarioId === "failed").trend,
    "non_comparable",
  );
  assert.equal(overview.counts.comparable, 1);
  assert.equal(overview.counts.new, 1);
  assert.equal(overview.counts.changed, 1);
  assert.equal(overview.counts.retired, 1);
  assert.equal(overview.counts.nonComparable, 1);
  assert.equal(overview.metrics.tokens.operational, 220);
  assert.equal(overview.metrics.tokens.comparableCurrent, 90);
  assert.equal(overview.metrics.tokens.comparableBaseline, 100);
  assert.equal(overview.metrics.tokens.delta, -10);
});
