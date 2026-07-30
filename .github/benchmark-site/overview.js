(function renderHarnessExecutionOverview() {
  "use strict";

  const benchmarkApi = window.HarnessBenchmarkData;
  const executionApi = window.HarnessExecutionData;
  const benchmarkData = benchmarkApi.normalizeBenchmarkData(window.BENCHMARK_DATA);
  const history = executionApi.mergeExecutionHistory(
    window.HARNESS_EXECUTIONS,
    benchmarkData,
  );
  const state = {
    matrixCount: 14,
    page: 1,
    pageSize: 25,
    query: "",
    status: "all",
    event: "all",
    scenarioHistoryRow: null,
    scenarioHistoryMetric: "cost_usd",
  };
  const palette = [
    "#c7ff4a",
    "#7bc7ff",
    "#ff9ee2",
    "#ffd166",
    "#a78bfa",
    "#69e3c2",
    "#ff786f",
  ];
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
    efficiencyBody: document.querySelector("#efficiency-body"),
    efficiencyCost: document.querySelector("#efficiency-cost"),
    efficiencyCostDelta: document.querySelector("#efficiency-cost-delta"),
    efficiencyCostSparkline: document.querySelector("#efficiency-cost-sparkline"),
    efficiencyDuration: document.querySelector("#efficiency-duration"),
    efficiencyDurationDelta: document.querySelector("#efficiency-duration-delta"),
    efficiencyDurationSparkline: document.querySelector(
      "#efficiency-duration-sparkline",
    ),
    efficiencyErrors: document.querySelector("#efficiency-errors"),
    efficiencyErrorsDelta: document.querySelector("#efficiency-errors-delta"),
    efficiencyErrorsSparkline: document.querySelector(
      "#efficiency-errors-sparkline",
    ),
    efficiencyGuardrail: document.querySelector("#efficiency-guardrail"),
    efficiencyRunLabel: document.querySelector("#efficiency-run-label"),
    efficiencyTokens: document.querySelector("#efficiency-tokens"),
    efficiencyTokensDelta: document.querySelector("#efficiency-tokens-delta"),
    efficiencyTokensSparkline: document.querySelector(
      "#efficiency-tokens-sparkline",
    ),
    event: document.querySelector("#event-filter"),
    kpiCost: document.querySelector("#kpi-cost"),
    kpiCoverage: document.querySelector("#kpi-coverage"),
    kpiFailures: document.querySelector("#kpi-failures"),
    kpiPassRate: document.querySelector("#kpi-pass-rate"),
    kpiRuntime: document.querySelector("#kpi-runtime"),
    kpiScore: document.querySelector("#kpi-score"),
    kpiStatus: document.querySelector("#kpi-status"),
    kpiStatusCaption: document.querySelector("#kpi-status-caption"),
    lastUpdate: document.querySelector("#last-update"),
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
    status: document.querySelector("#status-filter"),
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
      incomplete: { label: "Incomplete", short: "–", css: "incomplete" },
      cancelled: { label: "Cancelled", short: "○", css: "cancelled" },
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
    const status = statusMeta(latest.status);
    elements.kpiStatus.textContent = status.label;
    elements.kpiStatus.className = `kpi-value kpi-status-value text-${status.css}`;
    elements.kpiStatusCaption.textContent =
      `${formatDate(latest.completed_at, true)} · run ${latest.run_id || latest.id}` +
      (latest.attempt > 1 ? ` · attempt ${latest.attempt}` : "");
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
      return { label: "No change in comparable cohort", css: "neutral" };
    }
    return {
      label: `${value < 0 ? "↓" : "↑"} ${compactNumber(absolute, 1)}% comparable cohort`,
      css: value < 0 ? "improved" : "regressed",
    };
  }

  function renderEfficiencySparkline(element, metricId, color) {
    const values = history.executions
      .filter((execution) => (execution.scenario_metrics || []).length)
      .slice(0, 14)
      .reverse()
      .map((execution) =>
        (execution.scenario_metrics || []).reduce(
          (total, scenario) =>
            total + (Number(scenario?.averages?.[metricId]) || 0),
          0,
        ),
      );
    if (!values.length) {
      element.replaceChildren();
      return;
    }
    const width = 180;
    const height = 42;
    const minimum = Math.min(...values);
    const maximum = Math.max(...values);
    const range = maximum - minimum || 1;
    const x = (index) =>
      values.length === 1 ? width / 2 : (index / (values.length - 1)) * width;
    const y = (value) => 5 + (1 - (value - minimum) / range) * (height - 10);
    const svg = svgElement("svg", {
      viewBox: `0 0 ${width} ${height}`,
      role: "img",
      "aria-label": `${scenarioMetricDefinitions[metricId].label} operational trend`,
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
        sparkline: elements.efficiencyCostSparkline,
        format: formatCurrency,
        color: palette[0],
      },
      {
        metricId: "tokens",
        value: elements.efficiencyTokens,
        delta: elements.efficiencyTokensDelta,
        sparkline: elements.efficiencyTokensSparkline,
        format: (value) => compactNumber(value, 0),
        color: palette[1],
      },
      {
        metricId: "duration_seconds",
        value: elements.efficiencyDuration,
        delta: elements.efficiencyDurationDelta,
        sparkline: elements.efficiencyDurationSparkline,
        format: formatDuration,
        color: palette[3],
      },
      {
        metricId: "function_call_errors",
        value: elements.efficiencyErrors,
        delta: elements.efficiencyErrorsDelta,
        sparkline: elements.efficiencyErrorsSparkline,
        format: (value) => compactNumber(value, 1),
        color: palette[6],
      },
    ];
    cards.forEach((card) => {
      const metric = overview.metrics[card.metricId];
      card.value.textContent = card.format(metric?.operational);
      const meta = deltaMeta(metric?.delta);
      card.delta.textContent = meta.label;
      card.delta.className = `efficiency-delta delta-${meta.css}`;
      renderEfficiencySparkline(card.sparkline, card.metricId, card.color);
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
            : !scenario || scenario.status === "missing_report"
              ? "incomplete"
              : scenario.passed
                ? "passed"
                : "failed";
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
            stroke: palette[0],
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
        fill: palette[0],
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
          execution.status === "cancelled" ? "cancelled" : cell?.status || "incomplete";
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
      full: "Full report",
      aggregate: "Aggregate",
      unavailable: "No report",
    }[availability] || "Unknown";
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
      row.innerHTML = `
        <td>
          <div class="release-cell">
            <a href="${escapeHtml(detailUrl(execution))}">${escapeHtml(
              formatDate(execution.completed_at, true),
            )}</a>
            <span>run ${escapeHtml(execution.run_id || execution.id)} · attempt ${
              execution.attempt
            } · ${escapeHtml(trigger)}</span>
          </div>
        </td>
        <td><span class="table-status status-${meta.css}">${meta.label}</span></td>
        <td>${
          commit
            ? `<a class="commit-link" href="${escapeHtml(
                safeUrl(
                  `${history.repoUrl.replace(/\/$/, "")}/commit/${encodeURIComponent(commit)}`,
                ),
              )}">${escapeHtml(commit.slice(0, 7))}</a>`
            : "—"
        }</td>
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
      elements.body.append(row);
    });
    if (!page.length) {
      const row = document.createElement("tr");
      row.innerHTML =
        '<td class="table-empty" colspan="12">No executions match these filters.</td>';
      elements.body.append(row);
    }
    elements.count.textContent =
      `${filtered.length} execution${filtered.length === 1 ? "" : "s"}`;
    elements.pageLabel.textContent = `Page ${state.page} of ${pageCount}`;
    elements.previous.disabled = state.page === 1;
    elements.next.disabled = state.page === pageCount;
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
    elements.preview.hidden = !history.preview;
    const lastUpdate = history.lastUpdate || history.executions[0]?.completed_at;
    if (lastUpdate) {
      elements.lastUpdate.dateTime = new Date(lastUpdate).toISOString();
      elements.lastUpdate.textContent = formatDate(lastUpdate, true);
    }
    if (history.repoUrl) {
      const repo = safeUrl(history.repoUrl);
      if (repo) {
        elements.actionsLink.href =
          `${repo.replace(/\/$/, "")}/actions/workflows/harness-e2e-daily.yml`;
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
    render();
    await hydrateExecutionMetrics();
    renderEfficiency();
    renderTable();
  }

  initialize();
})();
