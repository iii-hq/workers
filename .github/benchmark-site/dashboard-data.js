(function initHarnessBenchmarkData(root, factory) {
  const api = factory(root);
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  }
  root.HarnessBenchmarkData = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function dashboardDataFactory(root) {
  "use strict";

  const METRIC_SEPARATOR = "::";

  function parseExtra(extra) {
    if (!extra || typeof extra !== "string") return {};
    try {
      const value = JSON.parse(extra);
      return value && typeof value === "object" ? value : {};
    } catch (_error) {
      return {};
    }
  }

  function parseMetricName(name) {
    if (typeof name !== "string") return null;
    const parts = name.split(METRIC_SEPARATOR);
    if (parts.length !== 4 || parts.some((part) => !part)) return null;
    const [category, subjectId, scenarioId, metricId] = parts;
    return { category, subjectId, scenarioId, metricId };
  }

  function pointIdentity(record, extra) {
    const lane = extra.lane || "unknown";
    const commitId = record.commit?.id || extra.source?.sha || "unknown";
    const releaseTag = extra.release?.tag || "";
    const executionId = extra.execution?.id || "";
    return `${lane}${METRIC_SEPARATOR}${executionId || releaseTag || commitId}`;
  }

  function emptyMetricTree() {
    return {
      quality: {},
      reliability: {},
      efficiency: {},
    };
  }

  function ensureScenario(tree, category, scenarioId) {
    if (!tree[category]) tree[category] = {};
    if (!tree[category][scenarioId]) tree[category][scenarioId] = {};
    return tree[category][scenarioId];
  }

  function normalizeBenchmarkData(raw) {
    const data = raw && typeof raw === "object" ? raw : {};
    const snapshots = new Map();
    const entries = data.entries && typeof data.entries === "object" ? data.entries : {};

    for (const records of Object.values(entries)) {
      if (!Array.isArray(records)) continue;
      for (const record of records) {
        if (!record || !Array.isArray(record.benches)) continue;
        for (const bench of record.benches) {
          const parsed = parseMetricName(bench?.name);
          if (!parsed || typeof bench.value !== "number" || !Number.isFinite(bench.value)) {
            continue;
          }
          const extra = parseExtra(bench.extra);
          const key = pointIdentity(record, extra);
          if (!snapshots.has(key)) {
            snapshots.set(key, {
              id: key,
              date: Number(record.date) || 0,
              lane: extra.lane || "unknown",
              commit: record.commit || {},
              release: extra.release || {},
              source: extra.source || {},
              execution: extra.execution || {},
              workflowUrl: extra.workflow_url || "",
              generatedAt: extra.generated_at || "",
              subjects: {},
            });
          }
          const snapshot = snapshots.get(key);
          snapshot.date = Math.max(snapshot.date, Number(record.date) || 0);
          if (!snapshot.subjects[parsed.subjectId]) {
            snapshot.subjects[parsed.subjectId] = {
              id: parsed.subjectId,
              model: extra.subject?.model || parsed.subjectId,
              provider: extra.subject?.provider || "",
              judge: extra.judge || {},
              engineRevision: extra.engine_revision || "",
              passed: extra.passed,
              status: extra.status || "unknown",
              requestedRuns: extra.requested_runs,
              metrics: emptyMetricTree(),
              metadata: extra,
            };
          }
          const subject = snapshot.subjects[parsed.subjectId];
          subject.metadata = { ...subject.metadata, ...extra };
          if (parsed.scenarioId === "suite") {
            subject.passed = extra.passed;
            subject.status = extra.status || subject.status;
          }
          const scenario = ensureScenario(
            subject.metrics,
            parsed.category,
            parsed.scenarioId,
          );
          scenario[parsed.metricId] = {
            value: bench.value,
            unit: bench.unit || "",
            range: bench.range || "",
            threshold:
              typeof extra.threshold === "number" ? extra.threshold : null,
            passed:
              typeof extra.passed === "boolean" ? extra.passed : null,
            status: extra.status || "unknown",
          };
        }
      }
    }

    const normalizedSnapshots = [...snapshots.values()].sort(
      (left, right) => left.date - right.date,
    );
    return {
      repoUrl: data.repoUrl || "",
      lastUpdate: Number(data.lastUpdate) || 0,
      preview: Boolean(root.HARNESS_BENCHMARK_PREVIEW),
      snapshots: normalizedSnapshots,
      lanes: [...new Set(normalizedSnapshots.map((snapshot) => snapshot.lane))],
      channels: [
        ...new Set(
          normalizedSnapshots.map(
            (snapshot) => snapshot.release?.registry_tag || "manual",
          ),
        ),
      ],
      subjects: [
        ...new Set(
          normalizedSnapshots.flatMap((snapshot) => Object.keys(snapshot.subjects)),
        ),
      ],
    };
  }

  function getMetric(subject, category, scenarioId, metricId) {
    return subject?.metrics?.[category]?.[scenarioId]?.[metricId] || null;
  }

  function metricValue(subject, category, scenarioId, metricId) {
    return getMetric(subject, category, scenarioId, metricId)?.value ?? null;
  }

  function listScenarios(subject) {
    const scenarios = new Set();
    for (const category of Object.values(subject?.metrics || {})) {
      for (const scenarioId of Object.keys(category || {})) {
        if (scenarioId !== "suite") scenarios.add(scenarioId);
      }
    }
    return [...scenarios].sort();
  }

  function filterSnapshots(data, filters = {}) {
    let snapshots = [...(data?.snapshots || [])];
    if (filters.lane && filters.lane !== "all") {
      snapshots = snapshots.filter((snapshot) => snapshot.lane === filters.lane);
    }
    if (filters.channel && filters.channel !== "all") {
      snapshots = snapshots.filter(
        (snapshot) =>
          (snapshot.release?.registry_tag || "manual") === filters.channel,
      );
    }
    if (filters.subjectId) {
      snapshots = snapshots.filter((snapshot) =>
        Boolean(snapshot.subjects[filters.subjectId]),
      );
    }
    if (
      filters.subjectId &&
      filters.scenarioId &&
      filters.scenarioId !== "all"
    ) {
      snapshots = snapshots.filter((snapshot) =>
        listScenarios(snapshot.subjects[filters.subjectId]).includes(
          filters.scenarioId,
        ),
      );
    }
    const days = Number(filters.days);
    if (Number.isFinite(days) && days > 0 && snapshots.length > 0) {
      const millisecondsPerDay = 24 * 60 * 60 * 1000;
      const latestDay = Math.floor(
        snapshots.at(-1).date / millisecondsPerDay,
      );
      const firstIncludedDay = latestDay - days + 1;
      snapshots = snapshots.filter(
        (snapshot) =>
          Math.floor(snapshot.date / millisecondsPerDay) >= firstIncludedDay,
      );
    }
    const limit = Number(filters.limit);
    if (Number.isFinite(limit) && limit > 0 && snapshots.length > limit) {
      snapshots = snapshots.slice(-limit);
    }
    return snapshots;
  }

  function subjectSummary(subject) {
    if (!subject) return null;
    const scenarios = listScenarios(subject);
    const scores = scenarios
      .map((scenarioId) =>
        metricValue(subject, "quality", scenarioId, "median_score"),
      )
      .filter((value) => value !== null);
    const averageScore =
      scores.length > 0
        ? scores.reduce((total, value) => total + value, 0) / scores.length
        : null;
    return {
      passed: Boolean(subject.passed),
      status: subject.status,
      averageScore,
      scenarioPassRate: metricValue(
        subject,
        "quality",
        "suite",
        "scenario_pass_rate",
      ),
      reportCoverage: metricValue(
        subject,
        "quality",
        "suite",
        "report_coverage",
      ),
      totalCost: metricValue(
        subject,
        "efficiency",
        "suite",
        "total_cost_usd",
      ),
      wallTime: metricValue(
        subject,
        "efficiency",
        "suite",
        "wall_time_seconds",
      ),
      hardGateFailures:
        metricValue(
          subject,
          "reliability",
          "suite",
          "hard_gate_failures",
        ) ?? 0,
      technicalFailures:
        metricValue(
          subject,
          "reliability",
          "suite",
          "technical_failures",
        ) ?? 0,
      missingReports:
        metricValue(subject, "reliability", "suite", "missing_reports") ?? 0,
      retries:
        metricValue(subject, "reliability", "suite", "retry_attempts") ?? 0,
      scenarios,
    };
  }

  function scenarioSummary(subject, scenarioId) {
    if (!subject) return null;
    const scoreMetric = getMetric(
      subject,
      "quality",
      scenarioId,
      "median_score",
    );
    const passRateMetric = getMetric(
      subject,
      "quality",
      scenarioId,
      "pass_rate",
    );
    return {
      id: scenarioId,
      medianScore: scoreMetric?.value ?? null,
      threshold: scoreMetric?.threshold ?? passRateMetric?.threshold ?? null,
      passed: scoreMetric?.passed ?? passRateMetric?.passed ?? false,
      status: scoreMetric?.status ?? passRateMetric?.status ?? "unknown",
      passRate: passRateMetric?.value ?? null,
      totalCost: metricValue(
        subject,
        "efficiency",
        scenarioId,
        "total_cost_usd",
      ),
      wallTime: metricValue(
        subject,
        "efficiency",
        scenarioId,
        "wall_time_seconds",
      ),
      hardGateFailures:
        metricValue(
          subject,
          "reliability",
          scenarioId,
          "hard_gate_failures",
        ) ?? 0,
      technicalFailures:
        metricValue(
          subject,
          "reliability",
          scenarioId,
          "technical_failures",
        ) ?? 0,
      retries:
        metricValue(
          subject,
          "reliability",
          scenarioId,
          "retry_attempts",
        ) ?? 0,
      missingReports:
        metricValue(
          subject,
          "reliability",
          scenarioId,
          "missing_reports",
        ) ?? 0,
    };
  }

  function metricSeries(snapshots, subjectId, category, metricId) {
    const series = new Map();
    for (const snapshot of snapshots || []) {
      const subject = snapshot.subjects?.[subjectId];
      if (!subject) continue;
      for (const scenarioId of listScenarios(subject)) {
        const metric = getMetric(subject, category, scenarioId, metricId);
        if (!metric) continue;
        if (!series.has(scenarioId)) series.set(scenarioId, []);
        series.get(scenarioId).push({
          snapshotId: snapshot.id,
          date: snapshot.date,
          label:
            snapshot.release?.tag ||
            snapshot.commit?.id?.slice(0, 7) ||
            "unknown",
          value: metric.value,
          unit: metric.unit,
          threshold: metric.threshold,
          snapshot,
        });
      }
    }
    return [...series.entries()].map(([scenarioId, points]) => ({
      scenarioId,
      points,
    }));
  }

  return {
    filterSnapshots,
    getMetric,
    listScenarios,
    metricSeries,
    metricValue,
    normalizeBenchmarkData,
    parseExtra,
    parseMetricName,
    scenarioSummary,
    subjectSummary,
  };
});
