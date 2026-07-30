const test = require("node:test");
const assert = require("node:assert/strict");

const {
  filterExecutions,
  findExecution,
  matrixCell,
  matrixCellLabel,
  matrixRows,
  mergeExecutionHistory,
  normalizeExecution,
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
