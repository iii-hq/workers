(function renderHarnessExecutionComparison() {
  "use strict";

  const api = window.HarnessExecutionData;
  const manifest = window.HARNESS_EXECUTIONS || { executions: [] };
  const history = {
    executions: (manifest.executions || []).map(api.normalizeExecution),
  };
  const parameters = new URLSearchParams(window.location.search);
  const left = api.findExecution(history, parameters.get("left") || "");
  const right = api.findExecution(history, parameters.get("right") || "");
  const elements = {
    content: document.querySelector("#compare-content"),
    empty: document.querySelector("#compare-empty"),
    metrics: document.querySelector("#compare-metrics"),
    scenarios: document.querySelector("#compare-scenarios"),
    selection: document.querySelector("#compare-selection"),
    warnings: document.querySelector("#compare-warnings"),
  };

  function escapeHtml(value) {
    return String(value ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#039;");
  }

  function number(value, digits = 1) {
    return typeof value === "number" && Number.isFinite(value)
      ? new Intl.NumberFormat("en-US", { maximumFractionDigits: digits }).format(value)
      : "—";
  }

  function percent(value) {
    return typeof value === "number" ? `${number(value, 1)}%` : "—";
  }

  function currency(value) {
    return typeof value === "number"
      ? new Intl.NumberFormat("en-US", {
          style: "currency",
          currency: "USD",
          minimumFractionDigits: value < 1 ? 3 : 2,
          maximumFractionDigits: value < 1 ? 3 : 2,
        }).format(value)
      : "—";
  }

  function duration(value) {
    if (typeof value !== "number") return "—";
    if (Math.abs(value) < 60) return `${number(value, 1)}s`;
    const sign = value < 0 ? "−" : "";
    const absolute = Math.abs(value);
    return `${sign}${Math.floor(absolute / 60)}m ${String(Math.round(absolute % 60)).padStart(2, "0")}s`;
  }

  function date(value) {
    return value && !Number.isNaN(Date.parse(value))
      ? new Intl.DateTimeFormat("en-US", {
          month: "short",
          day: "numeric",
          year: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        }).format(new Date(value))
      : "Unknown date";
  }

  function status(value) {
    return {
      passed: "Passed",
      quality_advisory: "Quality advisory",
      hard_gate_failed: "Hard gate failed",
      technical_failed: "Technical failure",
      infra_failed: "Infrastructure failure",
      incomplete: "Incomplete",
      cancelled: "Cancelled",
    }[value] || "Unknown";
  }

  function subjectSummary(execution) {
    const subjects = (execution.subjects || []).map(
      (subject) => `${subject.provider || "unknown"}/${subject.model || "unknown"}`,
    );
    return subjects.length === 1 ? subjects[0] : `${subjects.length} subjects`;
  }

  function selectionCard(execution, side) {
    const label = execution.label || date(execution.completed_at);
    return `
      <article class="compare-selection-card">
        <span>Execution ${side}</span>
        <h2>${escapeHtml(label)}</h2>
        <div class="compare-selection-meta">
          <small>${escapeHtml(date(execution.completed_at))}</small>
          <small>${escapeHtml(subjectSummary(execution))}</small>
          <small>${escapeHtml(status(execution.status))}</small>
          <small>${number(execution.requested_runs, 0)} run${execution.requested_runs === 1 ? "" : "s"}</small>
        </div>
        <a class="text-link" href="./execution.html?id=${encodeURIComponent(execution.id)}">Open diagnostic detail →</a>
      </article>`;
  }

  function signed(value, formatter) {
    if (typeof value !== "number") return "—";
    if (value === 0) return formatter(0);
    return `${value > 0 ? "+" : ""}${formatter(value)}`;
  }

  function renderMetric(definition, comparison) {
    const values = definition.values(comparison);
    let deltaClass = "";
    if (typeof values.delta === "number" && values.delta !== 0) {
      const improved = definition.lowerIsBetter ? values.delta < 0 : values.delta > 0;
      deltaClass = improved ? "compare-delta-improved" : "compare-delta-regressed";
    }
    return `
      <article class="compare-metric-card">
        <span>${escapeHtml(definition.label)}</span>
        <div class="compare-metric-values">
          <strong>${definition.format(values.left)}</strong>
          <small>→ ${definition.format(values.right)}</small>
        </div>
        <div class="compare-delta ${deltaClass}">${signed(values.delta, definition.format)} B−A</div>
      </article>`;
  }

  function blockingFailures(execution) {
    return ["hard_gate_failures", "technical_failures", "missing_reports"].reduce(
      (total, field) => total + Number(execution.totals?.[field] || 0),
      0,
    );
  }

  function sideScenario(value) {
    if (!value) return '<span class="text-incomplete">Not run</span>';
    const score = typeof value.score === "number" ? ` · score ${number(value.score, 1)}` : "";
    const passRate =
      typeof value.passRate === "number" ? ` · ${number(value.passRate * 100, 1)}% pass` : "";
    return `<span class="table-status status-${value.status === "passed" ? "pass" : value.status === "quality_advisory" ? "advisory" : "fail"}">${escapeHtml(status(value.status))}</span>${score}${passRate}`;
  }

  if (!left || !right || left.id === right.id) {
    elements.empty.hidden = false;
    return;
  }

  const comparison = api.compareExecutions(left, right);
  elements.content.hidden = false;
  elements.selection.innerHTML =
    selectionCard(comparison.left, "A") + selectionCard(comparison.right, "B");
  elements.warnings.innerHTML = comparison.warnings
    .map((warning) => `<li>${escapeHtml(warning)}</li>`)
    .join("");
  elements.warnings.hidden = comparison.warnings.length === 0;

  const metricDefinitions = [
    { label: "Pass rate", format: percent, lowerIsBetter: false, values: (item) => item.totals.scenario_pass_rate },
    { label: "Quality score", format: number, lowerIsBetter: false, values: (item) => item.totals.average_score },
    {
      label: "Blocking failures",
      format: (value) => number(value, 0),
      lowerIsBetter: true,
      values: (item) => ({
        left: blockingFailures(item.left),
        right: blockingFailures(item.right),
        delta: blockingFailures(item.right) - blockingFailures(item.left),
      }),
    },
    { label: "Tokens", format: (value) => number(value, 0), lowerIsBetter: true, values: (item) => item.totals.total_tokens },
    { label: "Function calls", format: (value) => number(value, 0), lowerIsBetter: true, values: (item) => item.totals.function_calls },
    { label: "Cost", format: currency, lowerIsBetter: true, values: (item) => item.totals.total_cost_usd },
    { label: "Runtime", format: duration, lowerIsBetter: true, values: (item) => item.totals.wall_time_seconds },
  ];
  elements.metrics.innerHTML = metricDefinitions
    .map((definition) => renderMetric(definition, comparison))
    .join("");

  elements.scenarios.innerHTML = comparison.scenarios
    .map((row) => `
      <tr>
        <th scope="row">
          <div class="compare-scenario-name">
            <span>${escapeHtml(row.subjectLabel)}</span>
            <strong>${escapeHtml(row.scenarioId.replaceAll("_", " "))}</strong>
          </div>
        </th>
        <td>${sideScenario(row.left)}</td>
        <td>${sideScenario(row.right)}</td>
        <td>${signed(row.deltas.score, (value) => number(value, 1))}</td>
        <td>${signed(row.deltas.tokens, (value) => number(value, 0))}</td>
        <td>${signed(row.deltas.cost_usd, currency)}</td>
        <td>${signed(row.deltas.duration_seconds, duration)}</td>
        <td><span class="comparison-contract comparison-contract-${escapeHtml(row.contract)}">${escapeHtml(row.contract)}</span></td>
      </tr>`)
    .join("");
})();
