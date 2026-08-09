const test = require("node:test");
const assert = require("node:assert/strict");

const {
  filterSnapshots,
  metricSeries,
  normalizeBenchmarkData,
  parseMetricName,
  scenarioSummary,
  subjectSummary,
} = require("./dashboard-data.js");

function record(date, value, name, extra = {}) {
  return {
    commit: { id: `commit-${date}`, message: "release" },
    date,
    benches: [
      {
        name,
        unit: name.endsWith("cost_usd") ? "USD" : "percent",
        value,
        extra: JSON.stringify({
          lane: "release",
          release: {
            tag: `harness/v${date}`,
            worker: "harness",
            registry_tag: "latest",
          },
          subject: { id: "glm", model: "glm-5.2", provider: "zai" },
          passed: true,
          ...extra,
        }),
      },
    ],
  };
}

test("parses the stable metric id contract", () => {
  assert.deepEqual(parseMetricName("quality::glm::direct_answer::median_score"), {
    category: "quality",
    subjectId: "glm",
    scenarioId: "direct_answer",
    metricId: "median_score",
  });
  assert.equal(parseMetricName("invalid"), null);
});

test("merges quality and efficiency records for the same release", () => {
  const raw = {
    entries: {
      "Harness E2E Quality": [
        record(1, 90, "quality::glm::direct_answer::median_score"),
        record(1, 100, "quality::glm::suite::scenario_pass_rate"),
      ],
      "Harness E2E Efficiency and Reliability": [
        record(1, 0.42, "efficiency::glm::suite::total_cost_usd"),
      ],
    },
  };

  const data = normalizeBenchmarkData(raw);
  assert.equal(data.snapshots.length, 1);
  const subject = data.snapshots[0].subjects.glm;
  const summary = subjectSummary(subject);
  assert.equal(summary.averageScore, 90);
  assert.equal(summary.scenarioPassRate, 100);
  assert.equal(summary.totalCost, 0.42);
  const scenario = scenarioSummary(subject, "direct_answer");
  assert.equal(scenario.passed, true);
});

test("keeps workflow attempts distinct even when they share a daily release tag", () => {
  const first = record(1, 90, "quality::glm::direct_answer::median_score", {
    execution: { id: "123-1", run_id: "123", attempt: 1 },
    release: { tag: "daily/2026-07-29", registry_tag: "daily" },
  });
  const second = record(2, 92, "quality::glm::direct_answer::median_score", {
    execution: { id: "123-2", run_id: "123", attempt: 2 },
    release: { tag: "daily/2026-07-29", registry_tag: "daily" },
  });

  const data = normalizeBenchmarkData({ entries: { quality: [first, second] } });

  assert.equal(data.snapshots.length, 2);
  assert.deepEqual(
    data.snapshots.map((snapshot) => snapshot.execution.id),
    ["123-1", "123-2"],
  );
});

test("filters release channels and returns ordered scenario series", () => {
  const raw = {
    entries: {
      quality: [
        record(1, 80, "quality::glm::direct_answer::median_score"),
        record(2, 92, "quality::glm::direct_answer::median_score"),
      ],
    },
  };
  const data = normalizeBenchmarkData(raw);
  const snapshots = filterSnapshots(data, {
    channel: "latest",
    subjectId: "glm",
    scenarioId: "direct_answer",
    limit: 1,
  });
  const series = metricSeries(
    snapshots,
    "glm",
    "quality",
    "median_score",
  );

  assert.equal(snapshots.length, 1);
  assert.equal(series.length, 1);
  assert.equal(series[0].scenarioId, "direct_answer");
  assert.equal(series[0].points[0].value, 92);
  assert.equal(
    filterSnapshots(data, {
      subjectId: "glm",
      scenarioId: "missing_scenario",
    }).length,
    0,
  );
});

test("filters snapshots by UTC calendar-day window", () => {
  const day = 24 * 60 * 60 * 1000;
  const raw = {
    entries: {
      quality: [
        record(
          Date.UTC(2026, 6, 1, 6),
          80,
          "quality::glm::direct_answer::median_score",
        ),
        record(
          Date.UTC(2026, 6, 15, 6),
          90,
          "quality::glm::direct_answer::median_score",
        ),
        record(
          Date.UTC(2026, 6, 30, 6),
          95,
          "quality::glm::direct_answer::median_score",
        ),
      ],
    },
  };
  const data = normalizeBenchmarkData(raw);
  const snapshots = filterSnapshots(data, {
    subjectId: "glm",
    days: 16,
  });

  assert.deepEqual(
    snapshots.map((snapshot) => snapshot.date),
    [Date.UTC(2026, 6, 15, 6), Date.UTC(2026, 6, 30, 6)],
  );
  assert.equal(
    Math.floor((snapshots[1].date - snapshots[0].date) / day),
    15,
  );
});
