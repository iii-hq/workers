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
    scenarioMetricScenario: "",
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
    scenarioMetricChart: document.querySelector("#scenario-metric-chart"),
    scenarioMetricContext: document.querySelector("#scenario-metric-context"),
    scenarioMetricDescription: document.querySelector(
      "#scenario-metric-description",
    ),
    scenarioMetricScenario: document.querySelector("#scenario-metric-scenario"),
    scenarioMetricTitle: document.querySelector("#scenario-metric-title"),
    search: document.querySelector("#execution-search"),
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

  async function hydrateScenarioMetrics() {
    await Promise.all(
      history.executions.map(async (execution) => {
        if (
          (execution.scenario_metrics || []).length ||
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
          execution.scenario_metrics =
            executionApi.scenarioMetricsFromDetail(detail);
        } catch (_error) {
          execution.scenario_metrics = [];
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

  function metricAxisValue(metricId, value) {
    if (metricId === "cost_usd") {
      return value < 1 ? `$${compactNumber(value, 2)}` : `$${compactNumber(value, 1)}`;
    }
    if (metricId === "duration_seconds") return formatDuration(value);
    return compactNumber(value, value < 10 ? 1 : 0);
  }

  function pointStatus(point, scenarioId) {
    const scenario = (point.execution?.subjects || [])
      .flatMap((subject) =>
        (subject.scenarios || []).map((item) => ({ ...item, subjectId: subject.id })),
      )
      .find((item) => item.id === scenarioId);
    if (!scenario || scenario.status === "missing_report") return "incomplete";
    return scenario.passed ? "pass" : "fail";
  }

  function pointUrl(point, scenarioId) {
    const subject = (point.execution?.subjects || []).find((item) =>
      (item.scenarios || []).some((scenario) => scenario.id === scenarioId),
    );
    return detailUrl(
      point.execution,
      subject ? { subjectId: subject.id, scenarioId } : null,
    );
  }

  function renderScenarioMetricFilter(metricExecutions) {
    const scenarioIds = [
      ...new Set(
        metricExecutions.flatMap((execution) =>
          (execution.scenario_metrics || []).map((scenario) => scenario.scenario_id),
        ),
      ),
    ].filter(Boolean).sort();
    if (!scenarioIds.includes(state.scenarioMetricScenario)) {
      state.scenarioMetricScenario = scenarioIds[0] || "";
    }
    elements.scenarioMetricScenario.replaceChildren(
      ...scenarioIds.map((scenarioId) =>
        new Option(titleCase(scenarioId), scenarioId),
      ),
    );
    elements.scenarioMetricScenario.value = state.scenarioMetricScenario;
    elements.scenarioMetricScenario.disabled = scenarioIds.length === 0;
    return scenarioIds;
  }

  function renderScenarioMetricChart(
    metricId,
    scenarioId,
    metricExecutions,
    color,
  ) {
    const definition = scenarioMetricDefinitions[metricId];
    const item = executionApi.scenarioMetricSeries(
      metricExecutions,
      metricId,
      scenarioId,
    )[0];
    const points = item?.points || [];
    const values = points.map((point) => point.value);
    const card = document.createElement("article");
    card.className = "scenario-metric-card";
    const heading = document.createElement("div");
    heading.className = "scenario-metric-card-heading";
    const label = document.createElement("h3");
    const swatch = document.createElement("i");
    swatch.style.background = color;
    label.append(swatch, document.createTextNode(definition.label));
    const latest = document.createElement("span");
    latest.textContent = points.length
      ? `Latest ${definition.format(points.at(-1).value)}`
      : "No data";
    heading.append(label, latest);
    const description = document.createElement("p");
    description.textContent = definition.description;
    const chart = document.createElement("div");
    chart.className = "scenario-metric-mini-chart";
    card.append(heading, description, chart);

    if (!values.length) {
      chart.innerHTML =
        '<div class="chart-empty">No execution-level points are available.</div>';
      return card;
    }

    const compactViewport = window.matchMedia("(max-width: 560px)").matches;
    const width = compactViewport ? 620 : 520;
    const height = compactViewport ? 290 : 250;
    const margin = compactViewport
      ? { top: 18, right: 18, bottom: 50, left: 54 }
      : { top: 18, right: 18, bottom: 46, left: 54 };
    const innerWidth = width - margin.left - margin.right;
    const innerHeight = height - margin.top - margin.bottom;
    const domainMin = 0;
    const domainMax = Math.max(...values) * 1.14 || 1;
    const xIndex = new Map(
      metricExecutions.map((execution, index) => [execution.id, index]),
    );
    const x = (executionId) => {
      const index = xIndex.get(executionId) || 0;
      if (metricExecutions.length === 1) return margin.left + innerWidth / 2;
      return margin.left + (index / (metricExecutions.length - 1)) * innerWidth;
    };
    const y = (value) =>
      margin.top +
      innerHeight -
      ((value - domainMin) / (domainMax - domainMin || 1)) * innerHeight;

    const svg = svgElement("svg", {
      viewBox: `0 0 ${width} ${height}`,
      role: "img",
      "aria-label": `${definition.label} by workflow execution for ${titleCase(scenarioId)}`,
    });
    const svgDescription = svgElement("desc");
    svgDescription.textContent =
      `Line chart comparing ${definition.label.toLowerCase()} across ${metricExecutions.length} workflow executions.`;
    svg.append(svgDescription);

    for (let index = 0; index <= 3; index += 1) {
      const value = domainMin + ((domainMax - domainMin) * index) / 3;
      const yPosition = y(value);
      svg.append(
        svgElement("line", {
          x1: margin.left,
          x2: width - margin.right,
          y1: yPosition,
          y2: yPosition,
          class: "chart-grid-line",
        }),
      );
      const label = svgElement("text", {
        x: margin.left - 12,
        y: yPosition + 4,
        "text-anchor": "end",
        class: "chart-axis-label",
      });
      label.textContent = metricAxisValue(metricId, value);
      svg.append(label);
    }

    const labelStep = Math.max(1, Math.ceil(metricExecutions.length / 4));
    metricExecutions.forEach((execution, index) => {
      if (index % labelStep !== 0 && index !== metricExecutions.length - 1) return;
      const label = svgElement("text", {
        x: x(execution.id),
        y: height - 20,
        "text-anchor": "middle",
        class: "chart-x-label",
      });
      label.textContent = `…${String(execution.run_id || execution.id).slice(-6)}`;
      svg.append(label);
    });

    let activeTooltip = null;
    function hideTooltip() {
      activeTooltip?.remove();
      activeTooltip = null;
    }

    function showTooltip(point) {
      hideTooltip();
      const pointX = x(point.executionId);
      const pointY = y(point.value);
      const boxWidth = 224;
      const boxHeight = 58;
      const boxX =
        pointX + boxWidth + 16 > width - margin.right
          ? pointX - boxWidth - 14
          : pointX + 14;
      const boxY = Math.max(
        margin.top,
        Math.min(pointY - boxHeight - 12, height - margin.bottom - boxHeight),
      );
      const statusName = pointStatus(point, scenarioId);
      const tooltip = svgElement("g", {
        class: "chart-tooltip",
        "aria-hidden": "true",
      });
      tooltip.append(
        svgElement("line", {
          x1: pointX,
          x2: pointX,
          y1: margin.top,
          y2: height - margin.bottom,
          class: "chart-tooltip-guide",
        }),
        svgElement("circle", {
          cx: pointX,
          cy: pointY,
          r: 7,
          fill: color,
          class: "chart-tooltip-point",
        }),
        svgElement("rect", {
          x: boxX,
          y: boxY,
          width: boxWidth,
          height: boxHeight,
          rx: 8,
          class: "chart-tooltip-box",
        }),
      );
      const heading = svgElement("text", {
        x: boxX + 12,
        y: boxY + 19,
        class: "chart-tooltip-heading",
      });
      heading.textContent =
        `${definition.label} · run ${point.runId}`;
      const value = svgElement("text", {
        x: boxX + 12,
        y: boxY + 40,
        class: "chart-tooltip-value",
      });
      value.textContent = definition.format(point.value);
      const status = svgElement("text", {
        x: boxX + boxWidth - 12,
        y: boxY + 40,
        "text-anchor": "end",
        class: `chart-tooltip-status chart-tooltip-status-${statusName}`,
      });
      status.textContent =
        statusName === "pass"
          ? "Passed"
          : statusName === "fail"
            ? "Attention"
            : "Incomplete";
      tooltip.append(heading, value, status);
      svg.append(tooltip);
      activeTooltip = tooltip;
    }

    const polylinePoints = points
      .map((point) => `${x(point.executionId)},${y(point.value)}`)
      .join(" ");
    const hitPath = svgElement("polyline", {
      points: polylinePoints,
      class: "chart-hit-path",
    });
    hitPath.addEventListener("mousemove", (event) => {
      const bounds = svg.getBoundingClientRect();
      const cursorX = ((event.clientX - bounds.left) / bounds.width) * width;
      const nearest = points.reduce((candidate, point) =>
        Math.abs(x(point.executionId) - cursorX) <
        Math.abs(x(candidate.executionId) - cursorX)
          ? point
          : candidate,
      );
      showTooltip(nearest);
    });
    hitPath.addEventListener("mouseleave", hideTooltip);
    svg.append(hitPath);
    svg.append(
      svgElement("polyline", {
        points: polylinePoints,
        stroke: color,
        class: "chart-path",
      }),
    );
    points.forEach((point) => {
      const statusName = pointStatus(point, scenarioId);
      const circle = svgElement("circle", {
        cx: x(point.executionId),
        cy: y(point.value),
        r: 4,
        fill: color,
        class: `chart-point chart-point-${statusName}`,
        tabindex: 0,
        role: "link",
      });
      const url = pointUrl(point, scenarioId);
      circle.setAttribute(
        "aria-label",
        `${titleCase(scenarioId)}, run ${point.runId}, ${definition.label} ${definition.format(point.value)}, ${statusName}. Open execution.`,
      );
      circle.addEventListener("click", () => window.location.assign(url));
      circle.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          window.location.assign(url);
        }
      });
      circle.addEventListener("mouseenter", () => showTooltip(point));
      circle.addEventListener("mouseleave", hideTooltip);
      circle.addEventListener("focus", () => showTooltip(point));
      circle.addEventListener("blur", hideTooltip);
      const title = svgElement("title");
      title.textContent =
        `${titleCase(scenarioId)} · ${formatDate(point.timestamp, true)} · ${definition.format(point.value)}`;
      circle.append(title);
      svg.append(circle);
    });

    chart.replaceChildren(svg);
    return card;
  }

  function renderScenarioMetrics() {
    const metricExecutions = executionApi.executionsWithinDays(
      history.executions,
      30,
    ).reverse();
    const scenarioIds = renderScenarioMetricFilter(metricExecutions);
    const scenarioId = state.scenarioMetricScenario;
    elements.scenarioMetricTitle.textContent = scenarioId
      ? titleCase(scenarioId)
      : "Scenario metrics";
    elements.scenarioMetricDescription.textContent =
      "Each chart uses its own scale. Hover a point to inspect the workflow attempt or select it to open the complete report.";
    elements.scenarioMetricContext.textContent =
      `${metricExecutions.length} retained execution${
        metricExecutions.length === 1 ? "" : "s"
      } · ${Object.keys(scenarioMetricDefinitions).length} metrics · one point per workflow attempt`;

    if (!scenarioIds.length || !scenarioId) {
      elements.scenarioMetricChart.innerHTML =
        '<div class="chart-empty">No execution-level scenario metrics are available.</div>';
      return;
    }

    elements.scenarioMetricChart.replaceChildren(
      ...Object.keys(scenarioMetricDefinitions).map((metricId, index) =>
        renderScenarioMetricChart(
          metricId,
          scenarioId,
          metricExecutions,
          palette[index % palette.length],
        ),
      ),
    );
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
        '<td class="table-empty" colspan="10">No executions match these filters.</td>';
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
    renderScenarioMetrics();
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
    document.querySelectorAll(".range-button").forEach((button) => {
      button.addEventListener("click", () => {
        state.matrixCount = Number(button.dataset.count);
        document.querySelectorAll(".range-button").forEach((candidate) => {
          candidate.classList.toggle("active", candidate === button);
        });
        renderMatrix();
      });
    });
    elements.scenarioMetricScenario.addEventListener("change", () => {
      state.scenarioMetricScenario = elements.scenarioMetricScenario.value;
      renderScenarioMetrics();
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
    await hydrateScenarioMetrics();
    renderScenarioMetrics();
  }

  initialize();
})();
