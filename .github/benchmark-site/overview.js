(function renderHarnessExecutionOverview() {
  "use strict";

  const benchmarkApi = window.HarnessBenchmarkData;
  const executionApi = window.HarnessExecutionData;
  const benchmarkData = benchmarkApi.normalizeBenchmarkData(window.BENCHMARK_DATA);
  const history = executionApi.mergeExecutionHistory(
    window.HARNESS_EXECUTIONS,
    benchmarkData,
  );
  const isLocal = history.mode === "local";
  const state = {
    matrixCount: 14,
    page: 1,
    pageSize: 25,
    query: "",
    status: "all",
    event: "all",
    scenarioHistoryRow: null,
    scenarioHistoryMetric: "cost_usd",
    comparison: [],
  };
  const efficiencyTrendColors = {
    improved: "var(--success)",
    regressed: "var(--danger)",
    neutral: "var(--text-muted)",
  };
  const chartAccentColor = "var(--accent)";
  const scenarioHistoryMetricIds = [
    "cost_usd",
    "tokens",
    "duration_seconds",
    "function_calls",
    "function_call_errors",
  ];
  const scenarioMetricDefinitions = {
    tokens: {
      label: "Tokens",
      description:
        "Input plus output tokens. Cache reads remain part of input usage and are not counted twice.",
      format: (value) => compactNumber(value, value < 100 ? 1 : 0),
    },
    duration_seconds: {
      label: "Time",
      description: "Wall-clock runtime for one scenario execution.",
      format: formatDuration,
    },
    cost_usd: {
      label: "Cost",
      description: "Combined subject and judge cost for one scenario execution.",
      format: formatCurrency,
    },
    function_calls: {
      label: "Function calls",
      description: "iii function calls made during one scenario execution.",
      format: (value) => compactNumber(value, 1),
    },
    function_call_errors: {
      label: "Function errors",
      description: "iii function results marked as errors during one scenario execution.",
      format: (value) => compactNumber(value, 1),
    },
    sessions: {
      label: "Sessions",
      description: "Root and descendant sessions observed during one scenario execution.",
      format: (value) => compactNumber(value, 1),
    },
    turns: {
      label: "Turns",
      description: "Agent turns observed during one scenario execution.",
      format: (value) => compactNumber(value, 1),
    },
  };

  const elements = {
    actionsLink: document.querySelector("#actions-link"),
    body: document.querySelector("#execution-body"),
    content: document.querySelector("#overview-content"),
    count: document.querySelector("#execution-count"),
    empty: document.querySelector("#empty-state"),
    emptyDescription: document.querySelector("#empty-description"),
    emptyTitle: document.querySelector("#empty-title"),
    efficiencyBody: document.querySelector("#efficiency-body"),
    efficiencyCost: document.querySelector("#efficiency-cost"),
    efficiencyCostDelta: document.querySelector("#efficiency-cost-delta"),
    efficiencyCostBaseline: document.querySelector("#efficiency-cost-baseline"),
    efficiencyCostSparkline: document.querySelector("#efficiency-cost-sparkline"),
    efficiencyDuration: document.querySelector("#efficiency-duration"),
    efficiencyDurationDelta: document.querySelector("#efficiency-duration-delta"),
    efficiencyDurationBaseline: document.querySelector(
      "#efficiency-duration-baseline",
    ),
    efficiencyDurationSparkline: document.querySelector(
      "#efficiency-duration-sparkline",
    ),
    efficiencyErrors: document.querySelector("#efficiency-errors"),
    efficiencyErrorsDelta: document.querySelector("#efficiency-errors-delta"),
    efficiencyErrorsBaseline: document.querySelector(
      "#efficiency-errors-baseline",
    ),
    efficiencyErrorsSparkline: document.querySelector(
      "#efficiency-errors-sparkline",
    ),
    efficiencyGuardrail: document.querySelector("#efficiency-guardrail"),
    efficiencyRunLabel: document.querySelector("#efficiency-run-label"),
    efficiencyTokens: document.querySelector("#efficiency-tokens"),
    efficiencyTokensDelta: document.querySelector("#efficiency-tokens-delta"),
    efficiencyTokensBaseline: document.querySelector(
      "#efficiency-tokens-baseline",
    ),
    efficiencyTokensSparkline: document.querySelector(
      "#efficiency-tokens-sparkline",
    ),
    event: document.querySelector("#event-filter"),
    footerSummary: document.querySelector("#dashboard-footer-summary"),
    commitHeading: document.querySelector("#execution-commit-heading"),
    kpiCost: document.querySelector("#kpi-cost"),
    kpiCoverage: document.querySelector("#kpi-coverage"),
    kpiFailures: document.querySelector("#kpi-failures"),
    kpiPassRate: document.querySelector("#kpi-pass-rate"),
    kpiRuntime: document.querySelector("#kpi-runtime"),
    kpiScore: document.querySelector("#kpi-score"),
    efficiencyStatus: document.querySelector("#efficiency-status"),
    efficiencyStatusCaption: document.querySelector("#efficiency-status-caption"),
    lastUpdate: document.querySelector("#last-update"),
    localRunner: document.querySelector("#local-runner"),
    localForm: document.querySelector("#local-run-form"),
    localSubmit: document.querySelector("#local-run-submit"),
    localCancel: document.querySelector("#local-run-cancel"),
    localRunStatus: document.querySelector("#local-run-status"),
    localRunError: document.querySelector("#local-run-error"),
    localRunLogShell: document.querySelector("#local-run-log-shell"),
    localRunLog: document.querySelector("#local-run-log"),
    localCatalogIndicator: document.querySelector("#local-catalog-indicator"),
    localCatalogStatus: document.querySelector("#local-catalog-status"),
    localConnectionUrl: document.querySelector("#local-connection-url"),
    localCatalogRefresh: document.querySelector("#local-catalog-refresh"),
    localSubject: document.querySelector("#local-subject"),
    localJudge: document.querySelector("#local-judge"),
    localScenarioPicker: document.querySelector("#local-scenario-picker"),
    localScenarioSummary: document.querySelector("#local-scenario-summary"),
    localScenarioOptions: document.querySelector("#local-scenario-options"),
    localScenarioAll: document.querySelector("#local-scenario-all"),
    localScenarioNone: document.querySelector("#local-scenario-none"),
    localAdvanced: document.querySelector("#local-advanced"),
    matrix: document.querySelector("#health-matrix"),
    next: document.querySelector("#next-page"),
    pageLabel: document.querySelector("#page-label"),
    preview: document.querySelector("#preview-badge"),
    previous: document.querySelector("#previous-page"),
    search: document.querySelector("#execution-search"),
    scenarioHistoryBody: document.querySelector("#scenario-history-body"),
    scenarioHistoryChart: document.querySelector("#scenario-history-chart"),
    scenarioHistoryClose: document.querySelector("#scenario-history-close"),
    scenarioHistoryContext: document.querySelector("#scenario-history-context"),
    scenarioHistoryDescription: document.querySelector(
      "#scenario-history-description",
    ),
    scenarioHistoryDialog: document.querySelector("#scenario-history-dialog"),
    scenarioHistoryTitle: document.querySelector("#scenario-history-title"),
    syncLabel: document.querySelector("#sync-label"),
    status: document.querySelector("#status-filter"),
    comparisonBar: document.querySelector("#comparison-bar"),
    comparisonCount: document.querySelector("#comparison-count"),
    comparisonLink: document.querySelector("#comparison-link"),
  };

  function escapeHtml(value) {
    return String(value ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#039;");
  }

  function safeUrl(value) {
    if (!value) return "";
    try {
      const url = new URL(value, window.location.href);
      return ["http:", "https:"].includes(url.protocol) ? url.href : "";
    } catch (_error) {
      return "";
    }
  }

  function detailUrl(execution, row = null) {
    const query = `./execution.html?id=${encodeURIComponent(execution.id)}`;
    return row ? `${query}#${scenarioAnchor(row.subjectId, row.scenarioId)}` : query;
  }

  function scenarioAnchor(subjectId, scenarioId) {
    return `scenario-${subjectId}-${scenarioId}`.replace(/[^a-zA-Z0-9_-]/g, "-");
  }

  function titleCase(value) {
    return String(value || "")
      .replaceAll("_", " ")
      .replaceAll("-", " ")
      .replace(/\b\w/g, (letter) => letter.toUpperCase());
  }

  function compactNumber(value, digits = 1) {
    return typeof value !== "number" || !Number.isFinite(value)
      ? "—"
      : new Intl.NumberFormat("en-US", {
          maximumFractionDigits: digits,
          minimumFractionDigits: 0,
        }).format(value);
  }

  function formatPercent(value) {
    return typeof value === "number" ? `${compactNumber(value, 1)}%` : "—";
  }

  function formatCurrency(value) {
    return typeof value !== "number"
      ? "—"
      : new Intl.NumberFormat("en-US", {
          style: "currency",
          currency: "USD",
          minimumFractionDigits: value < 1 ? 3 : 2,
          maximumFractionDigits: value < 1 ? 3 : 2,
        }).format(value);
  }

  function formatDuration(seconds) {
    if (typeof seconds !== "number") return "—";
    if (seconds < 60) return `${compactNumber(seconds, 0)}s`;
    const minutes = Math.floor(seconds / 60);
    const remainder = Math.round(seconds % 60);
    return `${minutes}m ${String(remainder).padStart(2, "0")}s`;
  }
  function formatScenarioHistoryValue(metricId, value) {
    if (metricId === "cost_usd" && typeof value === "number") {
      return new Intl.NumberFormat("en-US", {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 4,
        maximumFractionDigits: 4,
      }).format(value);
    }
    if (
      metricId === "duration_seconds" &&
      typeof value === "number" &&
      value < 60
    ) {
      return `${compactNumber(value, 1)}s`;
    }
    return scenarioMetricDefinitions[metricId].format(value);
  }

  function formatDate(timestamp, withTime = false) {
    if (!timestamp || Number.isNaN(Date.parse(timestamp))) return "Unknown date";
    return new Intl.DateTimeFormat("en-US", {
      month: "short",
      day: "numeric",
      year: "numeric",
      hour: withTime ? "2-digit" : undefined,
      minute: withTime ? "2-digit" : undefined,
    }).format(new Date(timestamp));
  }

  function statusMeta(status) {
    return {
      passed: { label: "Passed", short: "", css: "pass" },
      failed: { label: "Failed", short: "×", css: "fail" },
      quality_advisory: { label: "Quality advisory", short: "!", css: "advisory" },
      hard_gate_failed: { label: "Hard gate failed", short: "×", css: "fail" },
      technical_failed: { label: "Technical failure", short: "×", css: "fail" },
      infra_failed: { label: "Infrastructure failure", short: "×", css: "fail" },
      incomplete: { label: "Incomplete", short: "–", css: "incomplete" },
      cancelled: { label: "Cancelled", short: "○", css: "cancelled" },
      running: { label: "Running", short: "•", css: "running" },
    }[status] || { label: "Unknown", short: "?", css: "incomplete" };
  }

  function failureCount(execution) {
    const totals = execution.totals || {};
    return (
      Number(totals.hard_gate_failures || 0) +
      Number(totals.technical_failures || 0) +
      Number(totals.missing_reports || 0)
    );
  }

  function renderKpis() {
    const latest = history.executions[0];
    if (!latest) return;
    elements.kpiPassRate.textContent = formatPercent(
      latest.totals?.scenario_pass_rate,
    );
    elements.kpiCoverage.textContent =
      `${formatPercent(latest.totals?.report_coverage)} report coverage`;
    elements.kpiScore.textContent = compactNumber(latest.totals?.average_score, 1);
    elements.kpiFailures.textContent = compactNumber(failureCount(latest), 0);
    elements.kpiCost.textContent = formatCurrency(latest.totals?.total_cost_usd);
    elements.kpiRuntime.textContent =
      `${formatDuration(latest.totals?.wall_time_seconds)} model runtime`;
  }

  function deltaMeta(value) {
    if (typeof value !== "number" || !Number.isFinite(value)) {
      return { label: "Collecting comparable baseline", css: "neutral" };
    }
    const absolute = Math.abs(value);
    if (absolute < 0.05) {
      return { label: "No change vs baseline median", css: "neutral" };
    }
    return {
      label: `${value < 0 ? "↓" : "↑"} ${compactNumber(absolute, 1)}% vs baseline median`,
      css: value < 0 ? "improved" : "regressed",
    };
  }

  function renderEfficiencySparkline(element, metricId, color, cohortRows, baseline) {
    const points = executionApi.cohortMetricSparkline(
      history.executions,
      cohortRows,
      metricId,
      14,
    );
    if (!points.length) {
      element.replaceChildren();
      return;
    }
    const definition = scenarioMetricDefinitions[metricId];
    const values = points.map((point) => point.value);
    const baselineValue = typeof baseline === "number" ? baseline : null;
    const scaleValues =
      baselineValue === null ? values : [...values, baselineValue];
    const width = 180;
    const height = 42;
    const minimum = Math.min(...scaleValues);
    const maximum = Math.max(...scaleValues);
    const range = maximum - minimum || 1;
    const x = (index) =>
      values.length === 1 ? width / 2 : (index / (values.length - 1)) * width;
    const y = (value) => 5 + (1 - (value - minimum) / range) * (height - 10);
    const svg = svgElement("svg", {
      viewBox: `0 0 ${width} ${height}`,
      role: "img",
      "aria-label": `${definition.label} comparable cohort trend`,
    });
    const areaPoints = [
      `0,${height}`,
      ...values.map((value, index) => `${x(index)},${y(value)}`),
      `${width},${height}`,
    ].join(" ");
    svg.append(
      svgElement("polygon", {
        points: areaPoints,
        fill: color,
        class: "efficiency-sparkline-area",
      }),
    );
    if (baselineValue !== null) {
      const baselineLine = svgElement("line", {
        x1: 0,
        x2: width,
        y1: y(baselineValue),
        y2: y(baselineValue),
        class: "efficiency-sparkline-baseline",
      });
      const baselineTitle = svgElement("title", {});
      baselineTitle.textContent =
        `Baseline ${definition.format(baselineValue)} — median of up to 7 prior comparable runs`;
      baselineLine.append(baselineTitle);
      svg.append(baselineLine);
    }
    svg.append(
      svgElement("polyline", {
        points: values
          .map((value, index) => `${x(index)},${y(value)}`)
          .join(" "),
        stroke: color,
        class: "efficiency-sparkline-line",
      }),
      svgElement("circle", {
        cx: x(values.length - 1),
        cy: y(values.at(-1)),
        r: 3,
        fill: color,
      }),
    );
    const slot = width / points.length;
    points.forEach((point, index) => {
      const hit = svgElement("rect", {
        x: index * slot,
        y: 0,
        width: slot,
        height,
        class: "efficiency-sparkline-hit",
      });
      const parts = [
        `Run ${point.executionId}`,
        formatDate(point.timestamp),
        definition.format(point.value),
      ];
      if (baselineValue !== null && baselineValue !== 0) {
        const deltaPct = ((point.value - baselineValue) / Math.abs(baselineValue)) * 100;
        parts.push(
          `${deltaPct < 0 ? "↓" : "↑"} ${compactNumber(Math.abs(deltaPct), 1)}% vs baseline`,
        );
      }
      const title = svgElement("title", {});
      title.textContent = parts.join(" · ");
      hit.append(title);
      svg.append(hit);
    });
    element.replaceChildren(svg);
  }

  function efficiencyCell(row, metricId, formatter) {
    const current = row.current?.averages?.[metricId];
    if (typeof current !== "number") {
      const previous = row.baseline?.[metricId];
      return typeof previous === "number"
        ? `<span class="efficiency-cell-muted">${escapeHtml(
            formatter(previous),
          )}<small>last observed</small></span>`
        : "—";
    }
    const delta =
      row.lifecycle === "comparable" && row.outcome.passed
        ? row.deltas?.[metricId]
        : null;
    const meta = deltaMeta(delta);
    const deltaLabel =
      typeof delta === "number"
        ? `<small class="efficiency-cell-delta delta-${meta.css}">${escapeHtml(
            `${delta < 0 ? "↓" : delta > 0 ? "↑" : ""}${compactNumber(
              Math.abs(delta),
              1,
            )}%`,
          )}</small>`
        : "";
    return `<span>${escapeHtml(formatter(current))}${deltaLabel}</span>`;
  }

  function efficiencyTrendMeta(row) {
    const values = {
      improving: ["Improving", "improved"],
      stable: ["Stable", "stable"],
      regressed: ["Regressed", "regressed"],
      mixed: ["Mixed", "mixed"],
      collecting: [
        `Baseline ${row.historyCount}/${row.established ? row.historyCount : 5}`,
        "collecting",
      ],
      new: ["New", "new"],
      changed: [`Changed · v${row.scenarioVersion}`, "changed"],
      retired: ["Removed", "retired"],
      non_comparable: ["Non-comparable", "non-comparable"],
    };
    const [label, css] = values[row.trend] || ["Unknown", "collecting"];
    return { label, css };
  }

  function renderEfficiency() {
    const latest = history.executions[0];
    if (latest) {
      const status = statusMeta(latest.status);
      elements.efficiencyStatus.textContent = status.label;
      elements.efficiencyStatus.className =
        `efficiency-result-value text-${status.css}`;
      elements.efficiencyStatusCaption.textContent =
        `${formatDate(latest.completed_at, true)} · run ${latest.run_id || latest.id}` +
        (latest.attempt > 1 ? ` · attempt ${latest.attempt}` : "");
    }
    const overview = executionApi.buildEfficiencyOverview(history.executions);
    if (!overview.latest) {
      elements.efficiencyBody.innerHTML =
        '<tr><td colspan="7" class="table-empty">Waiting for complete efficiency reports.</td></tr>';
      return;
    }
    elements.efficiencyRunLabel.textContent =
      `Run ${overview.latest.run_id || overview.latest.id} · ${formatDate(
        overview.latest.completed_at,
        true,
      )}`;
    const cards = [
      {
        metricId: "cost_usd",
        value: elements.efficiencyCost,
        delta: elements.efficiencyCostDelta,
        baseline: elements.efficiencyCostBaseline,
        sparkline: elements.efficiencyCostSparkline,
        format: formatCurrency,
      },
      {
        metricId: "tokens",
        value: elements.efficiencyTokens,
        delta: elements.efficiencyTokensDelta,
        baseline: elements.efficiencyTokensBaseline,
        sparkline: elements.efficiencyTokensSparkline,
        format: (value) => compactNumber(value, 0),
      },
      {
        metricId: "duration_seconds",
        value: elements.efficiencyDuration,
        delta: elements.efficiencyDurationDelta,
        baseline: elements.efficiencyDurationBaseline,
        sparkline: elements.efficiencyDurationSparkline,
        format: formatDuration,
      },
      {
        metricId: "function_call_errors",
        value: elements.efficiencyErrors,
        delta: elements.efficiencyErrorsDelta,
        baseline: elements.efficiencyErrorsBaseline,
        sparkline: elements.efficiencyErrorsSparkline,
        format: (value) => compactNumber(value, 1),
      },
    ];
    const cohortRows = overview.rows.filter(
      (row) => row.lifecycle === "comparable" && row.outcome.passed,
    );
    cards.forEach((card) => {
      const metric = overview.metrics[card.metricId];
      // Value, delta, and sparkline must all read the same population; fall
      // back to the full-suite total only while no cohort exists yet.
      card.value.textContent = card.format(
        cohortRows.length ? metric?.comparableCurrent : metric?.operational,
      );
      const meta = deltaMeta(metric?.delta);
      card.delta.textContent = meta.label;
      card.delta.className = `efficiency-delta delta-${meta.css}`;
      const baselineValue =
        cohortRows.length && typeof metric?.comparableBaseline === "number"
          ? metric.comparableBaseline
          : null;
      card.baseline.textContent =
        baselineValue === null ? "" : `baseline ${card.format(baselineValue)}`;
      renderEfficiencySparkline(
        card.sparkline,
        card.metricId,
        efficiencyTrendColors[meta.css] || efficiencyTrendColors.neutral,
        cohortRows,
        cohortRows.length ? metric?.comparableBaseline : null,
      );
    });

    const passed = Number(overview.latest.totals?.passed_scenarios) || 0;
    const expected = Number(overview.latest.totals?.expected_reports) || 0;
    const countParts = [
      `${passed}/${expected} scenarios passed`,
      `${overview.counts.comparable} comparable`,
    ];
    if (overview.counts.new) countParts.push(`${overview.counts.new} new`);
    if (overview.counts.changed) countParts.push(`${overview.counts.changed} changed`);
    if (overview.counts.retired) countParts.push(`${overview.counts.retired} removed`);
    if (overview.counts.nonComparable) {
      countParts.push(`${overview.counts.nonComparable} non-comparable`);
    }
    const guardrailAlert =
      overview.counts.nonComparable ||
      overview.counts.changed ||
      passed < expected;
    elements.efficiencyGuardrail.className =
      `efficiency-guardrail${guardrailAlert ? " efficiency-guardrail-alert" : ""}`;
    elements.efficiencyGuardrail.innerHTML = `
      <span class="guardrail-status">${
        guardrailAlert ? "Outcome attention" : "Outcome guardrail passed"
      }</span>
      <span>${escapeHtml(countParts.join(" · "))}</span>
      <small>Lower efficiency totals are positive only for the comparable cohort.</small>
    `;

    elements.efficiencyBody.replaceChildren();
    overview.rows.forEach((row) => {
      const trend = efficiencyTrendMeta(row);
      const tableRow = document.createElement("tr");
      tableRow.className = `efficiency-row efficiency-row-${trend.css}`;
      tableRow.innerHTML = `
        <th scope="row">
          <button class="scenario-history-button" type="button">
            <span>${escapeHtml(titleCase(row.scenarioId))}</span>
            <small>${escapeHtml(row.subjectId || "default subject")} · v${escapeHtml(
            row.scenarioVersion,
          )}</small></button>
        </th>
        <td>${efficiencyCell(row, "cost_usd", formatCurrency)}</td>
        <td>${efficiencyCell(row, "tokens", (value) =>
          compactNumber(value, 0),
        )}</td>
        <td>${efficiencyCell(row, "duration_seconds", formatDuration)}</td>
        <td>${efficiencyCell(row, "function_calls", (value) =>
          compactNumber(value, 1),
        )}</td>
        <td>${efficiencyCell(row, "function_call_errors", (value) =>
          compactNumber(value, 1),
        )}</td>
        <td><span class="efficiency-trend trend-${trend.css}">${escapeHtml(
          trend.label,
        )}</span></td>
      `;
      tableRow
        .querySelector(".scenario-history-button")
        .addEventListener("click", () => openScenarioHistory(row));
      elements.efficiencyBody.append(tableRow);
    });
  }

  async function hydrateExecutionMetrics() {
    await Promise.all(
      history.executions.map(async (execution) => {
        const hasScenarioMetrics = (execution.scenario_metrics || []).length > 0;
        const hasEfficiencyTotals =
          typeof execution.totals?.total_tokens === "number" &&
          typeof execution.totals?.function_calls === "number";
        if (
          (hasScenarioMetrics && hasEfficiencyTotals) ||
          execution.availability !== "full" ||
          typeof execution.detail_path !== "string" ||
          execution.detail_path.includes("..") ||
          !execution.detail_path.startsWith("runs/")
        ) {
          return;
        }
        try {
          const preview = window.HARNESS_EXECUTION_DETAILS?.[execution.id];
          let detail = preview;
          if (!detail) {
            const url = new URL(execution.detail_path, window.location.href);
            const runsRoot = new URL("./runs/", window.location.href);
            if (
              url.origin !== runsRoot.origin ||
              !url.pathname.startsWith(runsRoot.pathname)
            ) {
              return;
            }
            const response = await fetch(url, { cache: "no-store" });
            if (!response.ok) return;
            detail = await response.json();
          }
          if (!hasScenarioMetrics) {
            execution.scenario_metrics =
              executionApi.scenarioMetricsFromDetail(detail);
          }
          execution.totals = {
            ...execution.totals,
            ...executionApi.executionEfficiencyTotalsFromDetail(detail),
          };
        } catch (_error) {
          if (!hasScenarioMetrics) execution.scenario_metrics = [];
        }
      }),
    );
  }

  function svgElement(name, attributes = {}) {
    const element = document.createElementNS("http://www.w3.org/2000/svg", name);
    Object.entries(attributes).forEach(([key, value]) => {
      element.setAttribute(key, String(value));
    });
    return element;
  }

  function scenarioHistoryEntries(row) {
    return history.executions
      .map((execution) => {
        const metric = (execution.scenario_metrics || []).find(
          (candidate) =>
            String(candidate.subject_id || "") === row.subjectId &&
            candidate.scenario_id === row.scenarioId,
        );
        if (!metric) return null;
        const subject = row.subjectId
          ? (execution.subjects || []).find(
              (candidate) => String(candidate.id || "") === row.subjectId,
            )
          : (execution.subjects || []).find((candidate) =>
              (candidate.scenarios || []).some(
                (scenario) => scenario.id === row.scenarioId,
              ),
            );
        const scenario = (subject?.scenarios || []).find(
          (candidate) => candidate.id === row.scenarioId,
        );
        const status =
          execution.status === "cancelled"
            ? "cancelled"
            : !scenario
              ? "incomplete"
              : executionApi.normalizeScenarioStatus(scenario);
        return { execution, metric, scenario, status };
      })
      .filter(Boolean)
      .reverse();
  }

  function renderScenarioHistoryTooltip(svg, point, entry, definition, metricId, bounds) {
    svg.querySelector(".scenario-history-tooltip")?.remove();
    const width = 212;
    const height = 70;
    const x = Math.min(Math.max(point.x - width / 2, 8), bounds.width - width - 8);
    const y = point.y > 92 ? point.y - height - 15 : point.y + 15;
    const meta = statusMeta(entry.status);
    const group = svgElement("g", { class: "scenario-history-tooltip" });
    group.append(
      svgElement("rect", {
        x,
        y,
        width,
        height,
        rx: 8,
        class: "chart-tooltip-box",
      }),
    );
    const heading = svgElement("text", {
      x: x + 12,
      y: y + 18,
      class: "chart-tooltip-heading",
    });
    heading.textContent = `${formatDate(entry.execution.completed_at, true)} · run ${
      entry.execution.run_id || entry.execution.id
    }`;
    const value = svgElement("text", {
      x: x + 12,
      y: y + 40,
      class: "chart-tooltip-value",
    });
    value.textContent = formatScenarioHistoryValue(metricId, point.value);
    const outcome = svgElement("text", {
      x: x + 12,
      y: y + 58,
      class: `chart-tooltip-status chart-tooltip-status-${meta.css}`,
    });
    outcome.textContent = `${meta.label} · v${entry.metric.scenario_version || 1}`;
    group.append(heading, value, outcome);
    svg.append(group);
  }

  function renderScenarioHistoryChart(entries, metricId, row) {
    const definition = scenarioMetricDefinitions[metricId];
    const points = entries
      .map((entry) => ({
        ...entry,
        value: entry.metric?.averages?.[metricId],
      }))
      .filter(
        (entry) =>
          typeof entry.value === "number" && Number.isFinite(entry.value),
      );
    if (!points.length) {
      elements.scenarioHistoryChart.innerHTML =
        '<div class="chart-empty">No values were collected for this metric.</div>';
      return;
    }

    const width = 960;
    const height = 330;
    const margin = { top: 28, right: 24, bottom: 48, left: 74 };
    const plotWidth = width - margin.left - margin.right;
    const plotHeight = height - margin.top - margin.bottom;
    const baseline = row.baseline?.[metricId];
    const domain = points.map((point) => point.value);
    if (typeof baseline === "number") domain.push(baseline);
    let minimum = Math.min(...domain);
    let maximum = Math.max(...domain);
    const padding =
      maximum === minimum
        ? Math.max(Math.abs(maximum) * 0.15, 1)
        : (maximum - minimum) * 0.15;
    minimum = Math.max(0, minimum - padding);
    maximum += padding;
    if (maximum === minimum) maximum = minimum + 1;
    const x = (index) =>
      margin.left +
      (points.length === 1 ? plotWidth / 2 : (index / (points.length - 1)) * plotWidth);
    const y = (value) =>
      margin.top + (1 - (value - minimum) / (maximum - minimum)) * plotHeight;
    const svg = svgElement("svg", {
      viewBox: `0 0 ${width} ${height}`,
      role: "img",
      "aria-label": `${definition.label} history for ${titleCase(row.scenarioId)}`,
    });

    for (let index = 0; index <= 4; index += 1) {
      const value = minimum + ((maximum - minimum) * index) / 4;
      const pointY = y(value);
      svg.append(
        svgElement("line", {
          x1: margin.left,
          y1: pointY,
          x2: width - margin.right,
          y2: pointY,
          class: "chart-grid-line",
        }),
      );
      const label = svgElement("text", {
        x: margin.left - 12,
        y: pointY + 4,
        "text-anchor": "end",
        class: "chart-axis-label",
      });
      label.textContent = formatScenarioHistoryValue(metricId, value);
      svg.append(label);
    }

    if (typeof baseline === "number") {
      const baselineY = y(baseline);
      svg.append(
        svgElement("line", {
          x1: margin.left,
          y1: baselineY,
          x2: width - margin.right,
          y2: baselineY,
          class: "chart-target",
        }),
      );
      const baselineLabel = svgElement("text", {
        x: width - margin.right,
        y: baselineY - 7,
        "text-anchor": "end",
        class: "chart-target-label",
      });
      baselineLabel.textContent = `Comparable baseline ${formatScenarioHistoryValue(
        metricId,
        baseline,
      )}`;
      svg.append(baselineLabel);
    }

    let segment = [];
    const appendSegment = () => {
      if (segment.length > 1) {
        svg.append(
          svgElement("polyline", {
            points: segment.map((point) => `${point.x},${point.y}`).join(" "),
            stroke: chartAccentColor,
            class: "chart-path",
          }),
        );
      }
      segment = [];
    };
    points.forEach((entry, index) => {
      const point = { x: x(index), y: y(entry.value), value: entry.value };
      const previous = points[index - 1];
      if (
        previous &&
        previous.metric.contract_fingerprint !== entry.metric.contract_fingerprint
      ) {
        appendSegment();
        const boundaryX = (x(index - 1) + x(index)) / 2;
        svg.append(
          svgElement("line", {
            x1: boundaryX,
            y1: margin.top,
            x2: boundaryX,
            y2: height - margin.bottom,
            class: "chart-contract-boundary",
          }),
        );
        const label = svgElement("text", {
          x: boundaryX + 5,
          y: margin.top + 10,
          class: "chart-contract-label",
        });
        label.textContent = `v${entry.metric.scenario_version || 1}`;
        svg.append(label);
      }
      segment.push(point);
      if (index === points.length - 1) appendSegment();

      const link = svgElement("a", {
        href: detailUrl(entry.execution, row),
        "aria-label": `${definition.label} ${definition.format(entry.value)}, ${
          statusMeta(entry.status).label
        }, ${formatDate(entry.execution.completed_at, true)}`,
      });
      const circle = svgElement("circle", {
        cx: point.x,
        cy: point.y,
        r: 4.5,
        fill: chartAccentColor,
        class: `chart-point chart-point-${statusMeta(entry.status).css}`,
      });
      link.append(circle);
      const showTooltip = () =>
        renderScenarioHistoryTooltip(
          svg,
          point,
          entry,
          definition,
          metricId,
          { width, height },
        );
      link.addEventListener("mouseenter", showTooltip);
      link.addEventListener("focus", showTooltip);
      link.addEventListener("mouseleave", () =>
        svg.querySelector(".scenario-history-tooltip")?.remove(),
      );
      link.addEventListener("blur", () =>
        svg.querySelector(".scenario-history-tooltip")?.remove(),
      );
      svg.append(link);
    });

    const labelIndexes = [
      ...new Set([0, Math.floor((points.length - 1) / 2), points.length - 1]),
    ];
    labelIndexes.forEach((index) => {
      const label = svgElement("text", {
        x: x(index),
        y: height - 16,
        "text-anchor":
          index === 0
            ? "start"
            : index === points.length - 1
              ? "end"
              : "middle",
        class: "chart-x-label",
      });
      label.textContent = formatDate(points[index].execution.completed_at, true);
      svg.append(label);
    });
    elements.scenarioHistoryChart.replaceChildren(svg);
  }

  function renderScenarioHistoryTable(entries, metricId, row) {
    const definition = scenarioMetricDefinitions[metricId];
    let previousComparable = null;
    const prepared = entries.map((entry, index) => {
      const value = entry.metric?.averages?.[metricId];
      const comparable =
        entry.status === "passed" &&
        previousComparable &&
        previousComparable.metric.contract_fingerprint ===
          entry.metric.contract_fingerprint;
      const delta = comparable
        ? ((value - previousComparable.value) / Math.abs(previousComparable.value)) * 100
        : null;
      const previous = entries[index - 1];
      const contractChanged = Boolean(
        previous &&
          previous.metric.contract_fingerprint !== entry.metric.contract_fingerprint,
      );
      if (entry.status === "passed" && typeof value === "number" && value !== 0) {
        previousComparable = { metric: entry.metric, value };
      }
      return { ...entry, value, delta, contractChanged };
    });
    elements.scenarioHistoryBody.replaceChildren();
    prepared.reverse().forEach((entry) => {
      const meta = statusMeta(entry.status);
      const tableRow = document.createElement("tr");
      const delta =
        typeof entry.delta === "number" && Number.isFinite(entry.delta)
          ? `${entry.delta < 0 ? "↓" : entry.delta > 0 ? "↑" : ""}${compactNumber(
              Math.abs(entry.delta),
              1,
            )}%`
          : "—";
      tableRow.innerHTML = `
        <td><div class="release-cell"><a href="${escapeHtml(
          detailUrl(entry.execution, row),
        )}">${escapeHtml(formatDate(entry.execution.completed_at, true))}</a><span>run ${escapeHtml(
          entry.execution.run_id || entry.execution.id,
        )}</span></div></td>
        <td>${escapeHtml(definition.format(entry.value))}</td>
        <td class="${entry.delta < 0 ? "text-pass" : entry.delta > 0 ? "text-fail" : ""}">${escapeHtml(
          delta,
        )}</td>
        <td><span class="table-status status-${meta.css}">${escapeHtml(meta.label)}</span></td>
        <td><span class="scenario-history-contract">v${escapeHtml(
          entry.metric.scenario_version || 1,
        )}${entry.contractChanged ? " · changed" : ""}</span></td>
      `;
      elements.scenarioHistoryBody.append(tableRow);
    });
  }

  function renderScenarioHistory() {
    const row = state.scenarioHistoryRow;
    if (!row) return;
    const entries = scenarioHistoryEntries(row);
    const metricId = state.scenarioHistoryMetric;
    const definition = scenarioMetricDefinitions[metricId];
    elements.scenarioHistoryTitle.textContent = titleCase(row.scenarioId);
    elements.scenarioHistoryContext.textContent = `${row.subjectId || "default subject"} · ${
      entries.length
    } execution${entries.length === 1 ? "" : "s"} · ${
      row.lifecycle === "retired"
        ? "removed from current suite"
        : `current contract v${row.scenarioVersion}`
    }`;
    elements.scenarioHistoryDescription.textContent =
      `${definition.description} Lines break when the scenario contract changes.`;
    document.querySelectorAll("[data-history-metric]").forEach((button) => {
      const active = button.dataset.historyMetric === metricId;
      button.classList.toggle("active", active);
      button.setAttribute("aria-selected", String(active));
      button.tabIndex = active ? 0 : -1;
    });
    renderScenarioHistoryChart(entries, metricId, row);
    renderScenarioHistoryTable(entries, metricId, row);
  }

  function openScenarioHistory(row) {
    state.scenarioHistoryRow = row;
    state.scenarioHistoryMetric = "cost_usd";
    renderScenarioHistory();
    if (!elements.scenarioHistoryDialog.open) {
      elements.scenarioHistoryDialog.showModal();
    }
  }

  function matrixTooltip(execution, row, cell) {
    const status = statusMeta(cell?.status || execution.status);
    if (!cell) {
      return `${formatDate(execution.completed_at, true)} · ${status.label} · no scenario report`;
    }
    const blocking =
      Number(cell.hard_gate_failures || 0) +
      Number(cell.technical_failures || 0);
    return [
      `${titleCase(row.scenarioId)} · ${formatDate(execution.completed_at, true)}`,
      `${status.label} · score ${compactNumber(cell.median_score, 1)} · pass ${formatPercent(
        typeof cell.pass_rate === "number" ? cell.pass_rate * 100 : null,
      )}`,
      `${formatCurrency(cell.total_cost_usd)} · ${formatDuration(
        cell.wall_time_seconds,
      )} · ${blocking} blocking event${blocking === 1 ? "" : "s"}`,
    ].join("\n");
  }

  function renderMatrix() {
    const executions = history.executions
      .slice(0, state.matrixCount)
      .reverse();
    const rows = executionApi.matrixRows(executions);
    if (!executions.length || !rows.length) {
      elements.matrix.innerHTML =
        '<div class="matrix-empty">No scenario reports are available for this range.</div>';
      return;
    }
    const table = document.createElement("table");
    table.className = "health-matrix";
    const thead = document.createElement("thead");
    const headerRow = document.createElement("tr");
    headerRow.innerHTML = '<th scope="col">Subject / scenario</th>';
    executions.forEach((execution) => {
      const header = document.createElement("th");
      header.scope = "col";
      const status = statusMeta(execution.status);
      header.innerHTML = `
        <a href="${escapeHtml(detailUrl(execution))}" aria-label="${escapeHtml(
          `${formatDate(execution.completed_at, true)}, ${status.label}, run ${execution.run_id}`,
        )}">
          <span>${escapeHtml(
            new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric" }).format(
              new Date(execution.completed_at || execution.started_at),
            ),
          )}</span>
          <i class="matrix-run-status status-${status.css}" aria-hidden="true"></i>
        </a>
      `;
      headerRow.append(header);
    });
    thead.append(headerRow);
    table.append(thead);

    const tbody = document.createElement("tbody");
    rows.forEach((row) => {
      const tableRow = document.createElement("tr");
      const label = document.createElement("th");
      label.scope = "row";
      label.innerHTML = `
        <span>${escapeHtml(row.subjectLabel)}</span>
        <strong>${escapeHtml(titleCase(row.scenarioId))}</strong>
      `;
      tableRow.append(label);
      executions.forEach((execution) => {
        const cell = executionApi.matrixCell(execution, row);
        const cellStatus =
          cell?.status ||
          (["cancelled", "running", "infra_failed"].includes(execution.status)
            ? execution.status
            : "incomplete");
        const meta = statusMeta(cellStatus);
        const cellLabel = executionApi.matrixCellLabel(cell, cellStatus);
        const data = document.createElement("td");
        const tooltip = matrixTooltip(execution, row, cell);
        data.innerHTML = `
          <a
            class="matrix-cell matrix-${escapeHtml(cellStatus)}"
            href="${escapeHtml(detailUrl(execution, row))}"
            aria-label="${escapeHtml(tooltip.replaceAll("\n", ". "))}"
            data-tooltip="${escapeHtml(tooltip)}"
          >
            <span aria-hidden="true">${escapeHtml(cellLabel || meta.short)}</span>
          </a>
        `;
        tableRow.append(data);
      });
      tbody.append(tableRow);
    });
    table.append(tbody);
    elements.matrix.replaceChildren(table);
  }

  function dataLabel(availability) {
    return {
      full: "Diagnostic detail",
      aggregate: "Aggregate",
      unavailable: "No report",
    }[availability] || "Unknown";
  }

  function renderComparisonBar() {
    if (!isLocal) {
      elements.comparisonBar.hidden = true;
      return;
    }
    elements.comparisonBar.hidden = false;
    const count = state.comparison.length;
    elements.comparisonCount.textContent =
      count === 0
        ? "Select two executions"
        : count === 1
          ? "1 of 2 executions selected"
          : "2 executions selected";
    const ready = count === 2;
    elements.comparisonLink.setAttribute("aria-disabled", String(!ready));
    elements.comparisonLink.href = ready
      ? `./compare.html?left=${encodeURIComponent(state.comparison[0])}&right=${encodeURIComponent(state.comparison[1])}`
      : "./compare.html";
  }

  function toggleComparison(executionId, checked) {
    state.comparison = state.comparison.filter((id) => id !== executionId);
    if (checked) {
      if (state.comparison.length === 2) state.comparison.shift();
      state.comparison.push(executionId);
    }
    renderTable();
  }

  function renderTable() {
    const filtered = executionApi.filterExecutions(history.executions, state);
    const pageCount = Math.max(1, Math.ceil(filtered.length / state.pageSize));
    state.page = Math.min(state.page, pageCount);
    const start = (state.page - 1) * state.pageSize;
    const page = filtered.slice(start, start + state.pageSize);
    elements.body.replaceChildren();
    page.forEach((execution) => {
      const row = document.createElement("tr");
      const meta = statusMeta(execution.status);
      const commit = execution.source?.sha || "";
      const subjectLabels = (execution.subjects || []).map(
        (subject) => `${subject.provider}/${subject.model}`,
      );
      const failures = failureCount(execution);
      const trigger =
        execution.event === "workflow_dispatch" ? "manual" : execution.event || "unknown";
      const primaryLabel = execution.label || formatDate(execution.completed_at, true);
      const secondaryLabel = execution.label
        ? `${formatDate(execution.completed_at, true)} · run ${execution.run_id || execution.id}`
        : `run ${execution.run_id || execution.id}`;
      const comparisonControl = isLocal
        ? `<label class="execution-compare-control">
            <input type="checkbox" data-compare-id="${escapeHtml(execution.id)}" ${
              state.comparison.includes(execution.id) ? "checked" : ""
            }>
            <span class="visually-hidden">Select ${escapeHtml(primaryLabel)} for comparison</span>
          </label>`
        : "";
      const commitCell = isLocal
        ? ""
        : `<td>${
            commit
              ? `<a class="commit-link" href="${escapeHtml(
                  safeUrl(
                    `${history.repoUrl.replace(/\/$/, "")}/commit/${encodeURIComponent(commit)}`,
                  ),
                )}">${escapeHtml(commit.slice(0, 7))}</a>`
              : "—"
          }</td>`;
      row.innerHTML = `
        <td>
          <div class="execution-identity-cell">
            ${comparisonControl}
            <div class="release-cell">
            <a href="${escapeHtml(detailUrl(execution))}">${escapeHtml(
              primaryLabel,
            )}</a>
            <span>${escapeHtml(secondaryLabel)} · attempt ${execution.attempt} · ${escapeHtml(trigger)}</span>
            </div>
          </div>
        </td>
        <td><span class="table-status status-${meta.css}">${meta.label}</span></td>
        ${commitCell}
        <td title="${escapeHtml(subjectLabels.join(", "))}">${escapeHtml(
          subjectLabels.length === 1
            ? subjectLabels[0]
            : subjectLabels.length
              ? `${subjectLabels.length} subjects`
              : "—",
        )}</td>
        <td>${formatPercent(execution.totals?.scenario_pass_rate)}</td>
        <td>${compactNumber(execution.totals?.average_score, 1)}</td>
        <td class="${failures ? "text-fail" : ""}">${compactNumber(failures, 0)}</td>
        <td>${compactNumber(execution.totals?.total_tokens, 0)}</td>
        <td>${compactNumber(execution.totals?.function_calls, 0)}</td>
        <td>${formatCurrency(execution.totals?.total_cost_usd)}</td>
        <td>${formatDuration(execution.totals?.wall_time_seconds)}</td>
        <td><span class="data-badge data-${escapeHtml(
          execution.availability,
        )}">${escapeHtml(dataLabel(execution.availability))}</span></td>
      `;
      row.querySelector("[data-compare-id]")?.addEventListener("change", (event) => {
        toggleComparison(execution.id, event.currentTarget.checked);
      });
      elements.body.append(row);
    });
    if (!page.length) {
      const row = document.createElement("tr");
      row.innerHTML =
        `<td class="table-empty" colspan="${isLocal ? 11 : 12}">No executions match these filters.</td>`;
      elements.body.append(row);
    }
    elements.count.textContent =
      `${filtered.length} execution${filtered.length === 1 ? "" : "s"}`;
    elements.pageLabel.textContent = `Page ${state.page} of ${pageCount}`;
    elements.previous.disabled = state.page === 1;
    elements.next.disabled = state.page === pageCount;
    renderComparisonBar();
  }

  function render() {
    const hasData = history.executions.length > 0;
    elements.empty.hidden = hasData;
    elements.content.hidden = !hasData;
    if (!hasData) return;
    renderKpis();
    renderEfficiency();
    renderMatrix();
    renderTable();
  }

  async function initialize() {
    elements.preview.hidden = !(history.preview || isLocal);
    elements.preview.textContent = isLocal ? "Local data" : "Preview data";
    if (isLocal) {
      elements.syncLabel.textContent = "Last completed";
      elements.emptyTitle.textContent = "No local executions yet";
      elements.emptyDescription.textContent =
        "Run an E2E experiment above to create the first local execution.";
      elements.actionsLink.textContent = "View repository ↗";
      elements.footerSummary.textContent = "Harness E2E · local execution history";
      elements.localRunner.hidden = false;
      elements.commitHeading.hidden = true;
      elements.search.placeholder = "Search label, run, or date";
    }
    const lastUpdate = history.lastUpdate || history.executions[0]?.completed_at;
    if (lastUpdate) {
      elements.lastUpdate.dateTime = new Date(lastUpdate).toISOString();
      elements.lastUpdate.textContent = formatDate(lastUpdate, true);
    }
    if (history.repoUrl) {
      const repo = safeUrl(history.repoUrl);
      if (repo) {
        elements.actionsLink.href = isLocal
          ? repo
          : `${repo.replace(/\/$/, "")}/actions/workflows/harness-e2e-daily.yml`;
      }
    }
    elements.scenarioHistoryClose.addEventListener("click", () => {
      elements.scenarioHistoryDialog.close();
    });
    elements.scenarioHistoryDialog.addEventListener("click", (event) => {
      if (event.target === elements.scenarioHistoryDialog) {
        elements.scenarioHistoryDialog.close();
      }
    });
    elements.scenarioHistoryDialog.addEventListener("close", () => {
      state.scenarioHistoryRow = null;
    });
    document.querySelectorAll("[data-history-metric]").forEach((button) => {
      button.addEventListener("click", () => {
        const metricId = button.dataset.historyMetric;
        if (!scenarioHistoryMetricIds.includes(metricId)) return;
        state.scenarioHistoryMetric = metricId;
        renderScenarioHistory();
      });
    });
    document.querySelectorAll(".range-button").forEach((button) => {
      button.addEventListener("click", () => {
        state.matrixCount = Number(button.dataset.count);
        document.querySelectorAll(".range-button").forEach((candidate) => {
          candidate.classList.toggle("active", candidate === button);
        });
        renderMatrix();
      });
    });
    elements.search.addEventListener("input", () => {
      state.query = elements.search.value;
      state.page = 1;
      renderTable();
    });
    elements.status.addEventListener("change", () => {
      state.status = elements.status.value;
      state.page = 1;
      renderTable();
    });
    elements.event.addEventListener("change", () => {
      state.event = elements.event.value;
      state.page = 1;
      renderTable();
    });
    elements.previous.addEventListener("click", () => {
      state.page = Math.max(1, state.page - 1);
      renderTable();
    });
    elements.next.addEventListener("click", () => {
      state.page += 1;
      renderTable();
    });
    if (isLocal) initializeLocalRunner();
    render();
    await hydrateExecutionMetrics();
    renderEfficiency();
    renderTable();
  }

  let localPollTimer = null;
  let localCatalogReady = false;
  let localCatalogLoading = false;
  let localJobActive = false;
  let localDefaults = {};

  function localFormField(name) {
    return elements.localForm.elements.namedItem(name);
  }

  function applyLocalDefaults(defaults) {
    localDefaults = { ...localDefaults, ...(defaults || {}) };
    Object.entries(defaults || {}).forEach(([name, value]) => {
      const field = localFormField(name);
      if (field && !field.value && value !== null && value !== undefined) {
        field.value = String(value);
      }
    });
    const url = localFormField("url")?.value || defaults?.url || "";
    if (url) elements.localConnectionUrl.textContent = url;
  }

  function setLocalControls(active) {
    localJobActive = active;
    for (const field of elements.localForm.elements) {
      if (field !== elements.localCancel) field.disabled = active;
    }
    elements.localSubject.disabled = active || !localCatalogReady;
    elements.localJudge.disabled = active || !localCatalogReady;
    elements.localSubmit.disabled = active || !localCatalogReady;
    elements.localCatalogRefresh.disabled = active || localCatalogLoading;
    elements.localScenarioPicker.classList.toggle(
      "local-picker-disabled",
      active || !localCatalogReady,
    );
    elements.localScenarioPicker.setAttribute(
      "aria-disabled",
      String(active || !localCatalogReady),
    );
  }

  function renderLocalJob(response) {
    applyLocalDefaults(response?.defaults);
    const job = response?.job;
    const active = ["running", "cancelling"].includes(job?.status);
    setLocalControls(active);
    elements.localCancel.hidden = !active;
    elements.localRunError.hidden = !job?.error;
    elements.localRunError.textContent = job?.error || "";
    elements.localRunLogShell.hidden = !job?.log;
    elements.localRunLog.textContent = job?.log || "";
    if (job?.log && active) elements.localRunLogShell.open = true;
    elements.localRunStatus.textContent = !job
      ? "Ready"
      : {
          running: "Running…",
          cancelling: "Cancelling…",
          cancelled: "Cancelled",
          completed: "Results saved",
          failed: "Runner failed",
        }[job.status] || job.status;
    if (active) {
      clearTimeout(localPollTimer);
      localPollTimer = setTimeout(refreshLocalJob, 1_000);
    } else if (job?.status === "completed" && job.id) {
      const reloadKey = "harness-e2e-local-last-reload";
      if (sessionStorage.getItem(reloadKey) !== job.id) {
        sessionStorage.setItem(reloadKey, job.id);
        window.location.reload();
      }
    }
  }

  async function localApi(path, options = {}) {
    const response = await fetch(path, {
      cache: "no-store",
      headers: { "Content-Type": "application/json" },
      ...options,
    });
    const payload = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(payload.error || `Request failed (${response.status})`);
    return payload;
  }

  async function refreshLocalJob() {
    try {
      const response = await localApi("./api/local/run");
      renderLocalJob(response);
      return response;
    } catch (error) {
      elements.localRunError.hidden = false;
      elements.localRunError.textContent = error.message;
      elements.localRunStatus.textContent = "Unavailable";
      return null;
    }
  }

  function modelKey(model) {
    return `${model.provider}\n${model.model}`;
  }

  function selectedModel(select) {
    const option = select.selectedOptions[0];
    return option?.dataset.model && option?.dataset.provider
      ? { model: option.dataset.model, provider: option.dataset.provider }
      : null;
  }

  function fillModelSelect(select, models, { includeAutomatic = false } = {}) {
    const selected = selectedModel(select);
    const preferredKey =
      (selected && modelKey(selected)) ||
      localStorage.getItem("harness-e2e-local-subject") ||
      (localDefaults.model && localDefaults.provider
        ? modelKey({ model: localDefaults.model, provider: localDefaults.provider })
        : "");
    select.replaceChildren();
    if (includeAutomatic) {
      const automatic = document.createElement("option");
      automatic.value = "";
      automatic.textContent = "Use subject model when required";
      select.append(automatic);
    }
    models.forEach((model, index) => {
      const option = document.createElement("option");
      option.value = `model-${index}`;
      option.dataset.model = model.model;
      option.dataset.provider = model.provider;
      option.textContent = `${model.provider} / ${model.model}`;
      option.selected = !includeAutomatic && modelKey(model) === preferredKey;
      select.append(option);
    });
    if (!includeAutomatic && select.selectedIndex < 0 && select.options.length) {
      select.selectedIndex = 0;
    }
  }

  function scenarioInputs() {
    return [...elements.localScenarioOptions.querySelectorAll("input[type=checkbox]")];
  }

  function updateScenarioSummary() {
    const inputs = scenarioInputs();
    const selected = inputs.filter((input) => input.checked).length;
    if (!inputs.length) {
      elements.localScenarioSummary.textContent = localCatalogLoading
        ? "Loading scenarios…"
        : "Catalog unavailable";
      elements.localSubmit.disabled = true;
      return;
    }
    elements.localScenarioSummary.textContent =
      selected === inputs.length
        ? `All ${inputs.length} scenarios`
        : `${selected} of ${inputs.length} scenarios`;
    elements.localSubmit.disabled =
      localJobActive || !localCatalogReady || selected === 0;
  }

  function fillScenarios(scenarios) {
    const previous = new Set(
      scenarioInputs().filter((input) => input.checked).map((input) => input.value),
    );
    const selectAll = previous.size === 0;
    elements.localScenarioOptions.replaceChildren();
    scenarios.forEach((scenarioId, index) => {
      const label = document.createElement("label");
      label.className = "local-scenario-option";
      const input = document.createElement("input");
      input.type = "checkbox";
      input.name = "scenario";
      input.value = scenarioId;
      input.id = `local-scenario-${index}`;
      input.checked = selectAll || previous.has(scenarioId);
      const text = document.createElement("span");
      text.textContent = scenarioId.replaceAll("_", " ");
      text.title = scenarioId;
      label.append(input, text);
      elements.localScenarioOptions.append(label);
    });
    updateScenarioSummary();
  }

  async function refreshLocalCatalog() {
    const url = localFormField("url")?.value || localDefaults.url || "";
    elements.localConnectionUrl.textContent = url;
    elements.localCatalogStatus.textContent = "Discovering the running Harness…";
    elements.localCatalogIndicator.className = "local-connection-dot";
    localCatalogLoading = true;
    localCatalogReady = false;
    setLocalControls(localJobActive);
    try {
      const query = new URLSearchParams({ url });
      const catalog = await localApi(`./api/local/catalog?${query}`);
      fillModelSelect(elements.localSubject, catalog.models);
      fillModelSelect(elements.localJudge, catalog.models, { includeAutomatic: true });
      fillScenarios(catalog.scenarios);
      localCatalogReady = true;
      elements.localCatalogIndicator.className = "local-connection-dot connected";
      elements.localCatalogStatus.textContent =
        `${catalog.models.length} registered model${catalog.models.length === 1 ? "" : "s"} · ${catalog.scenarios.length} scenarios`;
      elements.localRunError.hidden = true;
    } catch (error) {
      elements.localCatalogIndicator.className = "local-connection-dot failed";
      elements.localCatalogStatus.textContent = "Could not read the Harness catalog";
      elements.localRunError.hidden = false;
      elements.localRunError.textContent = error.message;
      elements.localScenarioSummary.textContent = "Catalog unavailable";
      elements.localAdvanced.open = true;
    } finally {
      localCatalogLoading = false;
      setLocalControls(localJobActive);
      updateScenarioSummary();
    }
  }

  function initializeLocalRunner() {
    elements.localForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      const values = new FormData(elements.localForm);
      const subject = selectedModel(elements.localSubject);
      const judge = selectedModel(elements.localJudge);
      const scenarios = scenarioInputs()
        .filter((input) => input.checked)
        .map((input) => input.value);
      try {
        if (!subject) throw new Error("Select a registered subject model.");
        if (!scenarios.length) throw new Error("Select at least one scenario.");
        localStorage.setItem("harness-e2e-local-subject", modelKey(subject));
        elements.localRunError.hidden = true;
        renderLocalJob(
          await localApi("./api/local/run", {
            method: "POST",
            body: JSON.stringify({
              label: values.get("label"),
              url: values.get("url"),
              model: subject.model,
              provider: subject.provider,
              judge_model: judge?.model || "",
              judge_provider: judge?.provider || "",
              scenarios,
              runs: Number(values.get("runs")),
              technical_retries: Number(values.get("technical_retries")),
            }),
          }),
        );
      } catch (error) {
        elements.localRunError.hidden = false;
        elements.localRunError.textContent = error.message;
        elements.localRunStatus.textContent = "Could not start";
      }
    });
    elements.localCancel.addEventListener("click", async () => {
      try {
        renderLocalJob(
          await localApi("./api/local/run/cancel", {
            method: "POST",
            body: "{}",
          }),
        );
      } catch (error) {
        elements.localRunError.hidden = false;
        elements.localRunError.textContent = error.message;
      }
    });
    elements.localCatalogRefresh.addEventListener("click", () => {
      refreshLocalCatalog();
    });
    elements.localScenarioAll.addEventListener("click", () => {
      scenarioInputs().forEach((input) => {
        input.checked = true;
      });
      updateScenarioSummary();
    });
    elements.localScenarioNone.addEventListener("click", () => {
      scenarioInputs().forEach((input) => {
        input.checked = false;
      });
      updateScenarioSummary();
    });
    elements.localScenarioOptions.addEventListener("change", updateScenarioSummary);
    elements.localScenarioPicker.addEventListener("toggle", () => {
      if (!localCatalogReady && elements.localScenarioPicker.open) {
        elements.localScenarioPicker.open = false;
      }
    });
    refreshLocalJob().then(() => refreshLocalCatalog());
  }

  initialize();
})();
