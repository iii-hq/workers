(function loadHarnessBenchmarkPreview() {
  "use strict";

  const scenarios = [
    "direct_answer",
    "persistent_state",
    "validation_loop",
    "reactive_automation",
  ];
  const releases = [
    {
      tag: "daily/2026-07-22",
      worker: "main",
      version: "2026-07-22",
      channel: "daily",
      date: Date.UTC(2026, 6, 22, 6),
      sha: "21b25ebc60deeb6059dc08d195e4f7b2b65df629",
      scores: [82, 88, 84, 76],
      passRates: [100, 100, 67, 67],
      costs: [0.08, 0.12, 0.14, 2.48],
      durations: [21, 26, 48, 390],
      hardGates: [0, 0, 0, 1],
      technical: [0, 0, 0, 0],
      retries: [0, 0, 1, 0],
      passed: false,
    },
    {
      tag: "daily/2026-07-23",
      worker: "main",
      version: "2026-07-23",
      channel: "daily",
      date: Date.UTC(2026, 6, 23, 6),
      sha: "46b6b917ceec4e1f01d2b54fd2b2bbd9437a8892",
      scores: [86, 91, 87, 81],
      passRates: [100, 100, 100, 67],
      costs: [0.075, 0.11, 0.13, 2.31],
      durations: [19, 24, 43, 355],
      hardGates: [0, 0, 0, 0],
      technical: [0, 0, 0, 0],
      retries: [0, 0, 0, 1],
      passed: true,
    },
    {
      tag: "daily/2026-07-24",
      worker: "main",
      version: "2026-07-24",
      channel: "daily",
      date: Date.UTC(2026, 6, 24, 6),
      sha: "8867dc0e781bcb5adeb32dc8ef61ca5410a17d21",
      scores: [89, 94, 90, 84],
      passRates: [100, 100, 100, 100],
      costs: [0.072, 0.105, 0.125, 2.18],
      durations: [18, 23, 41, 328],
      hardGates: [0, 0, 0, 0],
      technical: [0, 0, 0, 0],
      retries: [0, 0, 0, 0],
      passed: true,
    },
    {
      tag: "daily/2026-07-25",
      worker: "main",
      version: "2026-07-25",
      channel: "daily",
      date: Date.UTC(2026, 6, 25, 6),
      sha: "d7737956dcc5e13211b456551b1564b7d0aab11f",
      scores: [93, 96, 92, 88],
      passRates: [100, 100, 100, 100],
      costs: [0.069, 0.099, 0.119, 1.97],
      durations: [17, 21, 38, 301],
      hardGates: [0, 0, 0, 0],
      technical: [0, 0, 0, 0],
      retries: [0, 0, 0, 0],
      passed: true,
    },
    {
      tag: "daily/2026-07-26",
      worker: "main",
      version: "2026-07-26",
      channel: "daily",
      date: Date.UTC(2026, 6, 26, 6),
      sha: "d4b23bb742deba1acc143fb246e08c77fd99a061",
      scores: [94, 97, 93, 91],
      passRates: [100, 100, 100, 100],
      costs: [0.068, 0.096, 0.116, 1.88],
      durations: [17, 20, 37, 276],
      hardGates: [0, 0, 0, 0],
      technical: [0, 0, 0, 0],
      retries: [0, 0, 0, 0],
      passed: true,
    },
    {
      tag: "daily/2026-07-27",
      worker: "main",
      version: "2026-07-27",
      channel: "daily",
      date: Date.UTC(2026, 6, 27, 6),
      sha: "ed7f5efffd1541716ef017651e2bd26944171930",
      scores: [96, 98, 95, 93],
      passRates: [100, 100, 100, 67],
      costs: [0.064, 0.092, 0.11, 1.72],
      durations: [15, 19, 34, 248],
      hardGates: [0, 0, 0, 1],
      technical: [0, 0, 0, 0],
      retries: [0, 0, 0, 0],
      passed: false,
    },
    {
      tag: "daily/2026-07-28",
      worker: "main",
      version: "2026-07-28",
      channel: "daily",
      date: Date.UTC(2026, 6, 28, 6),
      sha: "a88137b350ccb0784e49b7c43682a8fba8a39ad1",
      scores: [98, 99, 97, 96],
      passRates: [100, 100, 100, 100],
      costs: [0.061, 0.088, 0.106, 1.61],
      durations: [14, 18, 32, 226],
      hardGates: [0, 0, 0, 0],
      technical: [0, 0, 0, 0],
      retries: [0, 0, 0, 0],
      passed: true,
    },
  ];

  function commit(release) {
    return {
      author: { name: "iii team", username: "iii-hq" },
      committer: { name: "iii team", username: "iii-hq" },
      id: release.sha,
      message: `release: ${release.tag}`,
      timestamp: new Date(release.date).toISOString(),
      url: `https://github.com/iii-hq/workers/commit/${release.sha}`,
    };
  }

  function scenarioPassed(release, index) {
    return (
      release.passRates[index] >= 67 &&
      release.hardGates[index] === 0 &&
      release.technical[index] === 0
    );
  }

  function releasePassed(release) {
    return scenarios.every((_scenario, index) => scenarioPassed(release, index));
  }

  function extra(release, scenario, passed = null) {
    const resolvedPassed = passed ?? releasePassed(release);
    const runId = String(Math.floor(release.date / 1000));
    return JSON.stringify({
      schema_version: 2,
      execution: {
        id: `${runId}-1`,
        run_id: runId,
        attempt: 1,
        event: "schedule",
        actor: "github-actions",
      },
      lane: "release",
      generated_at: new Date(release.date + 12 * 60 * 1000).toISOString(),
      source: {
        sha: release.sha,
        ref: release.tag,
        repository: "iii-hq/workers",
      },
      workflow_url: "https://github.com/iii-hq/workers/actions",
      release: {
        tag: release.tag,
        worker: release.worker,
        version: release.version,
        url: `https://github.com/iii-hq/workers/releases/tag/${release.tag}`,
        registry_tag: release.channel,
      },
      subject: { id: "glm-5-2", model: "glm-5.2", provider: "zai" },
      judge: { model: "glm-5.2", provider: "zai" },
      engine_revision: "c84f918f6f5e92e32ad78e6695d581c9e1995c9b",
      scenario,
      requested_runs: 3,
      passed: resolvedPassed,
      status: resolvedPassed ? "passed" : "failed",
    });
  }

  function metric(name, unit, value, metricExtra) {
    return { name, unit, value, extra: metricExtra };
  }

  function qualityRecord(release) {
    const benches = [];
    scenarios.forEach((scenario, index) => {
      benches.push(
        metric(
          `quality::glm-5-2::${scenario}::median_score`,
          "points",
          release.scores[index],
          extra(
            release,
            scenario,
            scenarioPassed(release, index),
          ),
        ),
        metric(
          `quality::glm-5-2::${scenario}::pass_rate`,
          "percent",
          release.passRates[index],
          extra(
            release,
            scenario,
            scenarioPassed(release, index),
          ),
        ),
      );
    });
    benches.push(
      metric(
        "quality::glm-5-2::suite::scenario_pass_rate",
        "percent",
        (scenarios.filter((_scenario, index) => scenarioPassed(release, index))
          .length /
          scenarios.length) *
          100,
        extra(release, "suite"),
      ),
      metric(
        "quality::glm-5-2::suite::report_coverage",
        "percent",
        100,
        extra(release, "suite"),
      ),
    );
    return {
      commit: commit(release),
      date: release.date,
      tool: "customBiggerIsBetter",
      benches,
    };
  }

  function efficiencyRecord(release) {
    const benches = [];
    scenarios.forEach((scenario, index) => {
      const metricExtra = extra(
        release,
        scenario,
        scenarioPassed(release, index),
      );
      benches.push(
        metric(
          `efficiency::glm-5-2::${scenario}::total_cost_usd`,
          "USD",
          release.costs[index],
          metricExtra,
        ),
        metric(
          `efficiency::glm-5-2::${scenario}::wall_time_seconds`,
          "seconds",
          release.durations[index],
          metricExtra,
        ),
        metric(
          `reliability::glm-5-2::${scenario}::hard_gate_failures`,
          "count",
          release.hardGates[index],
          metricExtra,
        ),
        metric(
          `reliability::glm-5-2::${scenario}::technical_failures`,
          "count",
          release.technical[index],
          metricExtra,
        ),
        metric(
          `reliability::glm-5-2::${scenario}::retry_attempts`,
          "count",
          release.retries[index],
          metricExtra,
        ),
        metric(
          `reliability::glm-5-2::${scenario}::missing_reports`,
          "count",
          0,
          metricExtra,
        ),
      );
    });
    const suiteExtra = extra(release, "suite");
    benches.push(
      metric(
        "efficiency::glm-5-2::suite::total_cost_usd",
        "USD",
        release.costs.reduce((sum, value) => sum + value, 0),
        suiteExtra,
      ),
      metric(
        "efficiency::glm-5-2::suite::wall_time_seconds",
        "seconds",
        release.durations.reduce((sum, value) => sum + value, 0),
        suiteExtra,
      ),
      metric(
        "reliability::glm-5-2::suite::hard_gate_failures",
        "count",
        release.hardGates.reduce((sum, value) => sum + value, 0),
        suiteExtra,
      ),
      metric(
        "reliability::glm-5-2::suite::technical_failures",
        "count",
        release.technical.reduce((sum, value) => sum + value, 0),
        suiteExtra,
      ),
      metric(
        "reliability::glm-5-2::suite::retry_attempts",
        "count",
        release.retries.reduce((sum, value) => sum + value, 0),
        suiteExtra,
      ),
      metric(
        "reliability::glm-5-2::suite::missing_reports",
        "count",
        0,
        suiteExtra,
      ),
    );
    return {
      commit: commit(release),
      date: release.date,
      tool: "customSmallerIsBetter",
      benches,
    };
  }

  window.HARNESS_BENCHMARK_PREVIEW = true;
  window.BENCHMARK_DATA = {
    lastUpdate: releases.at(-1).date,
    repoUrl: "https://github.com/iii-hq/workers",
    entries: {
      "Harness E2E Quality": releases.map(qualityRecord),
      "Harness E2E Efficiency and Reliability": releases.map(efficiencyRecord),
    },
  };
})();
