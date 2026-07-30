(function renderHarnessBenchmarkDashboard() {
  "use strict";

  const api = window.HarnessBenchmarkData;
  const data = api.normalizeBenchmarkData(window.BENCHMARK_DATA);
  const state = {
    subjectId: data.subjects[0] || "",
    scenarioId: "all",
    windowDays: 30,
    trendMetric: "score",
  };
  const palette = ["#c7ff4a", "#7bc7ff", "#ff9ee2", "#ffd166", "#a78bfa", "#69e3c2"];
  const metricDefinitions = {
    score: {
      title: "Quality score",
      category: "quality",
      metricId: "median_score",
      unit: "points",
      fixedDomain: [0, 100],
    },
    pass_rate: {
      title: "Scenario pass rate",
      category: "quality",
      metricId: "pass_rate",
      unit: "%",
      fixedDomain: [0, 100],
      target: 100,
    },
    cost: {
      title: "Model cost",
      category: "efficiency",
      metricId: "total_cost_usd",
      unit: "USD",
    },
    duration: {
      title: "Scenario runtime",
      category: "efficiency",
      metricId: "wall_time_seconds",
      unit: "seconds",
    },
  };

  const elements = {
    actionsLink: document.querySelector("#actions-link"),
    content: document.querySelector("#dashboard-content"),
    coverageNote: document.querySelector("#coverage-note"),
    dataHealth: document.querySelector("#data-health"),
    empty: document.querySelector("#empty-state"),
    failureCallout: document.querySelector("#failure-callout"),
    failureDetail: document.querySelector("#failure-detail"),
    failureLink: document.querySelector("#failure-link"),
    failureTitle: document.querySelector("#failure-title"),
    historyBody: document.querySelector("#history-body"),
    lastUpdate: document.querySelector("#last-update"),
    latestHeading: document.querySelector("#latest-heading"),
    meta: document.querySelector("#release-meta"),
    previewBadge: document.querySelector("#preview-badge"),
    releaseCount: document.querySelector("#release-count"),
    releaseLink: document.querySelector("#release-link"),
    releaseStatus: document.querySelector("#release-status"),
    scenario: document.querySelector("#scenario-filter"),
    scenarioGrid: document.querySelector("#scenario-grid"),
    subject: document.querySelector("#subject-filter"),
    subjectContext: document.querySelector("#subject-context"),
    trendChart: document.querySelector("#trend-chart"),
    trendHeading: document.querySelector("#trend-heading"),
    trendLegend: document.querySelector("#trend-legend"),
    window: document.querySelector("#window-filter"),
  };

  function titleCase(value) {
    return String(value || "")
      .replaceAll("_", " ")
      .replaceAll("-", " ")
      .replace(/\b\w/g, (letter) => letter.toUpperCase());
  }

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

  function executionUrl(snapshot, scenarioId = "") {
    if (!snapshot?.id) return "";
    const id = snapshot.execution?.id || snapshot.id;
    const base = `./execution.html?id=${encodeURIComponent(id)}`;
    if (!scenarioId) return base;
    const subjectId = state.subjectId.replace(/[^a-zA-Z0-9_-]/g, "-");
    const scenario = scenarioId.replace(/[^a-zA-Z0-9_-]/g, "-");
    return `${base}#scenario-${subjectId}-${scenario}`;
  }

  function compactNumber(value, digits = 1) {
    if (value === null || value === undefined || Number.isNaN(value)) return "—";
    return new Intl.NumberFormat("en-US", {
      maximumFractionDigits: digits,
      minimumFractionDigits: 0,
    }).format(value);
  }

  function formatPercent(value) {
    return value === null || value === undefined
      ? "—"
      : `${compactNumber(value, 1)}%`;
  }

  function formatCurrency(value) {
    return value === null || value === undefined
      ? "—"
      : new Intl.NumberFormat("en-US", {
          style: "currency",
          currency: "USD",
          minimumFractionDigits: value < 1 ? 3 : 2,
          maximumFractionDigits: value < 1 ? 3 : 2,
        }).format(value);
  }

  function formatDuration(seconds) {
    if (seconds === null || seconds === undefined) return "—";
    if (seconds < 60) return `${compactNumber(seconds, 0)}s`;
    const minutes = Math.floor(seconds / 60);
    const remainder = Math.round(seconds % 60);
    return `${minutes}m ${String(remainder).padStart(2, "0")}s`;
  }

  function formatDate(timestamp, options = {}) {
    if (!timestamp) return "Unknown date";
    return new Intl.DateTimeFormat("en-US", {
      month: "short",
      day: "numeric",
      year: options.year ? "numeric" : undefined,
      hour: options.time ? "numeric" : undefined,
      minute: options.time ? "2-digit" : undefined,
    }).format(new Date(timestamp));
  }

  function setOptions(select, values, selected, label) {
    select.replaceChildren();
    values.forEach((value) => {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = label(value);
      option.selected = value === selected;
      select.append(option);
    });
  }

  function latestSubject(snapshots) {
    const latest = snapshots.at(-1);
    return { latest, subject: latest?.subjects?.[state.subjectId] || null };
  }

  function previousSubject(snapshots) {
    const previous = snapshots.at(-2);
    return { previous, subject: previous?.subjects?.[state.subjectId] || null };
  }

  function availableScenarios() {
    return [
      ...new Set(
        data.snapshots.flatMap((snapshot) =>
          api.listScenarios(snapshot.subjects[state.subjectId]),
        ),
      ),
    ].sort();
  }

  function refreshScenarioOptions() {
    const scenarios = availableScenarios();
    if (state.scenarioId !== "all" && !scenarios.includes(state.scenarioId)) {
      state.scenarioId = "all";
    }
    setOptions(
      elements.scenario,
      ["all", ...scenarios],
      state.scenarioId,
      (scenarioId) =>
        scenarioId === "all" ? "All scenarios" : titleCase(scenarioId),
    );
  }

  function statusKind(summary) {
    if (!summary || summary.reportCoverage < 100) return "incomplete";
    return summary.passed ? "pass" : "fail";
  }

  function applyStatus(element, summary, compact = false) {
    const kind = statusKind(summary);
    const text =
      kind === "pass"
        ? "Passed"
        : kind === "incomplete"
          ? "Incomplete"
          : compact
            ? "Attention"
            : "Needs attention";
    element.className = `${compact ? "table-status" : "status-pill"} status-${kind}`;
    element.textContent = text;
  }

  function deltaText(current, previous, options = {}) {
    if (
      current === null ||
      current === undefined ||
      previous === null ||
      previous === undefined
    ) {
      return { text: "No previous daily run", className: "delta-neutral" };
    }
    const delta = current - previous;
    if (Math.abs(delta) < (options.tolerance || 0.0001)) {
      return { text: "No change from yesterday", className: "delta-neutral" };
    }
    const lowerIsBetter = Boolean(options.lowerIsBetter);
    const improved = lowerIsBetter ? delta < 0 : delta > 0;
    const formatter = options.formatter || ((value) => compactNumber(value, 1));
    return {
      text: `${delta > 0 ? "↑" : "↓"} ${formatter(Math.abs(delta))} vs previous run`,
      className: improved ? "delta-good" : "delta-bad",
    };
  }

  function updateDelta(selector, delta) {
    const element = document.querySelector(selector);
    element.textContent = delta.text;
    element.className = `kpi-delta ${delta.className}`;
  }

  function metaChip(label, value) {
    const chip = document.createElement("span");
    chip.className = "meta-chip";
    const strong = document.createElement("strong");
    strong.textContent = label;
    chip.append(strong, document.createTextNode(value || "unknown"));
    return chip;
  }

  function failureReason(summary) {
    const reasons = [];
    if (summary.hardGateFailures > 0) {
      reasons.push(
        `${summary.hardGateFailures} hard gate${summary.hardGateFailures === 1 ? "" : "s"}`,
      );
    }
    if (summary.technicalFailures > 0) {
      reasons.push(
        `${summary.technicalFailures} technical failure${summary.technicalFailures === 1 ? "" : "s"}`,
      );
    }
    if (summary.missingReports > 0) {
      reasons.push(
        `${summary.missingReports} missing report${summary.missingReports === 1 ? "" : "s"}`,
      );
    }
    if (!summary.passed && reasons.length === 0) {
      reasons.push("quality threshold");
    }
    return reasons.join(", ");
  }

  function renderFailureCallout(latest, subject) {
    const failures = api
      .listScenarios(subject)
      .map((scenarioId) => ({
        scenarioId,
        summary: api.scenarioSummary(subject, scenarioId),
      }))
      .filter(
        ({ summary }) => summary && scenarioStatus(summary) !== "pass",
      );

    elements.failureCallout.hidden = failures.length === 0;
    if (failures.length === 0) return;

    elements.failureTitle.textContent =
      `${failures.length} scenario${failures.length === 1 ? "" : "s"} need attention`;
    elements.failureDetail.textContent = failures
      .map(
        ({ scenarioId, summary }) =>
          `${titleCase(scenarioId)}: ${failureReason(summary)}`,
      )
      .join(" · ");
    const detailUrl = executionUrl(latest);
    elements.failureLink.href = detailUrl || "#";
    elements.failureLink.hidden = !detailUrl;
  }

  function renderLatest(snapshots) {
    const { latest, subject } = latestSubject(snapshots);
    const { subject: priorSubject } = previousSubject(snapshots);
    const summary = api.subjectSummary(subject);
    const prior = api.subjectSummary(priorSubject);
    if (!latest || !summary) return;

    elements.latestHeading.textContent = formatDate(latest.date, { year: true });
    applyStatus(elements.releaseStatus, summary);
    const releaseUrl = executionUrl(latest);
    elements.releaseLink.href = releaseUrl || "#";
    elements.releaseLink.hidden = !releaseUrl;
    elements.meta.replaceChildren(
      metaChip("cadence", "daily"),
      metaChip("commit", latest.commit?.id?.slice(0, 9) || "unknown"),
      metaChip("subject", `${subject.provider}/${subject.model}`),
      metaChip(
        "judge",
        subject.judge?.provider && subject.judge?.model
          ? `${subject.judge.provider}/${subject.judge.model}`
          : "unknown",
      ),
      metaChip("runs", String(subject.requestedRuns || "—")),
      metaChip(
        "engine",
        subject.engineRevision ? subject.engineRevision.slice(0, 9) : "unresolved",
      ),
    );
    renderFailureCallout(latest, subject);

    document.querySelector("#kpi-pass-rate").textContent = formatPercent(
      summary.scenarioPassRate,
    );
    document.querySelector("#kpi-score").textContent =
      summary.averageScore === null ? "—" : compactNumber(summary.averageScore, 1);
    document.querySelector("#kpi-cost").textContent = formatCurrency(summary.totalCost);
    document.querySelector("#kpi-duration").textContent = formatDuration(summary.wallTime);
    const reliabilityEvents =
      summary.hardGateFailures +
      summary.technicalFailures +
      summary.missingReports;
    document.querySelector("#kpi-reliability").textContent = compactNumber(
      reliabilityEvents,
      0,
    );
    document.querySelector("#kpi-reliability-caption").textContent =
      reliabilityEvents === 0
        ? `${summary.retries} retr${summary.retries === 1 ? "y" : "ies"} · no blocking events`
        : `${summary.hardGateFailures} hard gate · ${summary.technicalFailures} technical · ${summary.missingReports} missing`;

    updateDelta(
      "#kpi-pass-delta",
      deltaText(summary.scenarioPassRate, prior?.scenarioPassRate, {
        formatter: (value) => `${compactNumber(value, 1)} pts`,
      }),
    );
    updateDelta(
      "#kpi-score-delta",
      deltaText(summary.averageScore, prior?.averageScore, {
        formatter: (value) => `${compactNumber(value, 1)} pts`,
      }),
    );
    updateDelta(
      "#kpi-cost-delta",
      deltaText(summary.totalCost, prior?.totalCost, {
        lowerIsBetter: true,
        formatter: formatCurrency,
      }),
    );
    updateDelta(
      "#kpi-duration-delta",
      deltaText(summary.wallTime, prior?.wallTime, {
        lowerIsBetter: true,
        formatter: formatDuration,
      }),
    );

    elements.coverageNote.textContent = `${formatPercent(summary.reportCoverage)} report coverage`;
  }

  function chartValue(value, definition) {
    if (definition.unit === "USD") return formatCurrency(value);
    if (definition.unit === "seconds") return formatDuration(value);
    if (definition.unit === "%") return `${compactNumber(value, 1)}%`;
    return `${compactNumber(value, 1)} ${definition.unit}`;
  }

  function svgElement(name, attributes = {}) {
    const element = document.createElementNS("http://www.w3.org/2000/svg", name);
    for (const [key, value] of Object.entries(attributes)) {
      element.setAttribute(key, String(value));
    }
    return element;
  }

  function renderTrend(snapshots) {
    const definition = metricDefinitions[state.trendMetric];
    elements.trendHeading.textContent = `${definition.title} over time`;
    const series = api.metricSeries(
      snapshots,
      state.subjectId,
      definition.category,
      definition.metricId,
    ).filter(
      (item) =>
        state.scenarioId === "all" || item.scenarioId === state.scenarioId,
    );
    elements.trendLegend.replaceChildren();
    series.forEach((item, index) => {
      const legend = document.createElement("span");
      legend.className = "legend-item";
      const swatch = document.createElement("span");
      swatch.className = "legend-swatch";
      swatch.style.background = palette[index % palette.length];
      legend.append(swatch, document.createTextNode(titleCase(item.scenarioId)));
      elements.trendLegend.append(legend);
    });

    const values = series.flatMap((item) => item.points.map((point) => point.value));
    if (values.length === 0) {
      elements.trendChart.innerHTML =
        '<div class="chart-empty">No comparable points for this metric.</div>';
      return;
    }

    const compactViewport = window.matchMedia("(max-width: 560px)").matches;
    const width = compactViewport ? 600 : 960;
    const height = compactViewport ? 300 : 330;
    const margin = compactViewport
      ? { top: 22, right: 16, bottom: 44, left: 44 }
      : { top: 22, right: 28, bottom: 48, left: 56 };
    const innerWidth = width - margin.left - margin.right;
    const innerHeight = height - margin.top - margin.bottom;
    const [domainMin, domainMax] = definition.fixedDomain || [
      0,
      Math.max(...values) * 1.14 || 1,
    ];
    const dateBySnapshot = new Map(
      snapshots.map((snapshot) => [snapshot.id, Number(snapshot.date) || 0]),
    );
    const dates = [...dateBySnapshot.values()];
    const firstDate = Math.min(...dates);
    const lastDate = Math.max(...dates);
    const x = (snapshotId) => {
      const date = dateBySnapshot.get(snapshotId) || firstDate;
      if (firstDate === lastDate) return margin.left + innerWidth / 2;
      return margin.left + ((date - firstDate) / (lastDate - firstDate)) * innerWidth;
    };
    const y = (value) =>
      margin.top +
      innerHeight -
      ((value - domainMin) / (domainMax - domainMin || 1)) * innerHeight;

    const svg = svgElement("svg", {
      viewBox: `0 0 ${width} ${height}`,
      role: "img",
      "aria-label": `Daily ${definition.title.toLowerCase()} by scenario`,
    });
    const description = svgElement("desc");
    description.textContent = `Line chart showing ${definition.title.toLowerCase()} for ${series.length} scenarios across ${snapshots.length} daily runs.`;
    svg.append(description);

    for (let index = 0; index <= 4; index += 1) {
      const value = domainMin + ((domainMax - domainMin) * index) / 4;
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
      label.textContent =
        definition.unit === "USD"
          ? `$${compactNumber(value, 1)}`
          : definition.unit === "seconds"
            ? formatDuration(value)
            : compactNumber(value, 0);
      svg.append(label);
    }

    const targets = [
      ...new Set([
        ...(definition.target === undefined ? [] : [definition.target]),
        ...(state.trendMetric === "score"
          ? series.flatMap((item) =>
              item.points
                .map((point) => point.threshold)
                .filter((value) => typeof value === "number"),
            )
          : []),
      ]),
    ].sort((left, right) => left - right);
    targets.forEach((target) => {
      if (target < domainMin || target > domainMax) return;
      const targetY = y(target);
      svg.append(
        svgElement("line", {
          x1: margin.left,
          x2: width - margin.right,
          y1: targetY,
          y2: targetY,
          class: "chart-target",
        }),
      );
      const targetLabel = svgElement("text", {
        x: width - margin.right,
        y: targetY - 7,
        "text-anchor": "end",
        class: "chart-target-label",
      });
      targetLabel.textContent = `target ${target}`;
      svg.append(targetLabel);
    });

    const labelStep = Math.max(1, Math.ceil(snapshots.length / 6));
    snapshots.forEach((snapshot, index) => {
      if (index % labelStep !== 0 && index !== snapshots.length - 1) return;
      const label = svgElement("text", {
        x: x(snapshot.id),
        y: height - 18,
        "text-anchor": "middle",
        class: "chart-x-label",
      });
      label.textContent = formatDate(snapshot.date);
      svg.append(label);
    });

    let activeTooltip = null;
    function hideTooltip() {
      activeTooltip?.remove();
      activeTooltip = null;
    }

    function chartPointStatus(item, point) {
      const pointSummary = api.scenarioSummary(
        point.snapshot?.subjects?.[state.subjectId],
        item.scenarioId,
      );
      return pointSummary ? scenarioStatus(pointSummary) : "incomplete";
    }

    function showTooltip(item, point, color) {
      hideTooltip();
      const pointX = x(point.snapshotId);
      const pointY = y(point.value);
      const boxWidth = 214;
      const boxHeight = 50;
      const boxX =
        pointX + boxWidth + 16 > width - margin.right
          ? pointX - boxWidth - 14
          : pointX + 14;
      const boxY = Math.max(
        margin.top,
        Math.min(pointY - boxHeight - 12, height - margin.bottom - boxHeight),
      );
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
        y: boxY + 18,
        class: "chart-tooltip-heading",
      });
      heading.textContent =
        `${titleCase(item.scenarioId)} · ${formatDate(point.date)}`;
      const value = svgElement("text", {
        x: boxX + 12,
        y: boxY + 37,
        class: "chart-tooltip-value",
      });
      value.textContent = chartValue(point.value, definition);
      const pointStatus = chartPointStatus(item, point);
      const status = svgElement("text", {
        x: boxX + boxWidth - 12,
        y: boxY + 37,
        "text-anchor": "end",
        class: `chart-tooltip-status chart-tooltip-status-${pointStatus}`,
      });
      status.textContent =
        pointStatus === "pass"
          ? "Passed"
          : pointStatus === "fail"
            ? "Attention"
            : "Incomplete";
      tooltip.append(heading, value, status);
      svg.append(tooltip);
      activeTooltip = tooltip;
    }

    series.forEach((item, seriesIndex) => {
      const color = palette[seriesIndex % palette.length];
      const points = item.points
        .map((point) => `${x(point.snapshotId)},${y(point.value)}`)
        .join(" ");
      const hitPath = svgElement("polyline", {
        points,
        class: "chart-hit-path",
      });
      hitPath.addEventListener("mousemove", (event) => {
        const bounds = svg.getBoundingClientRect();
        const cursorX = ((event.clientX - bounds.left) / bounds.width) * width;
        const nearest = item.points.reduce((candidate, point) =>
          Math.abs(x(point.snapshotId) - cursorX) <
          Math.abs(x(candidate.snapshotId) - cursorX)
            ? point
            : candidate,
        );
        showTooltip(item, nearest, color);
      });
      hitPath.addEventListener("mouseleave", hideTooltip);
      svg.append(hitPath);
      svg.append(
        svgElement("polyline", {
          points,
          stroke: color,
          class: "chart-path",
        }),
      );
      item.points.forEach((point) => {
        const pointStatus = chartPointStatus(item, point);
        const circle = svgElement("circle", {
          cx: x(point.snapshotId),
          cy: y(point.value),
          r: 4,
          fill: color,
          class: `chart-point chart-point-${pointStatus}`,
          tabindex: 0,
        });
        const pointUrl = executionUrl(point.snapshot, item.scenarioId);
        if (pointUrl) {
          circle.setAttribute("role", "link");
          circle.setAttribute(
            "aria-label",
            `${titleCase(item.scenarioId)}, ${formatDate(point.date, { year: true })}, ${chartValue(point.value, definition)}, ${pointStatus}. Open workflow.`,
          );
          circle.addEventListener("click", () => {
            window.open(pointUrl, "_blank", "noopener");
          });
          circle.addEventListener("keydown", (event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              window.open(pointUrl, "_blank", "noopener");
            }
          });
        }
        circle.addEventListener("mouseenter", () => {
          showTooltip(item, point, color);
        });
        circle.addEventListener("mouseleave", hideTooltip);
        circle.addEventListener("focus", () => {
          showTooltip(item, point, color);
        });
        circle.addEventListener("blur", hideTooltip);
        const title = svgElement("title");
        title.textContent = `${titleCase(item.scenarioId)} · ${formatDate(point.date, { year: true })} · ${chartValue(point.value, definition)} · ${pointStatus}`;
        circle.append(title);
        svg.append(circle);
      });
    });
    elements.trendChart.replaceChildren(svg);
  }

  function scenarioStatus(summary) {
    if (summary.missingReports > 0) return "incomplete";
    return summary.passed ? "pass" : "fail";
  }

  function renderScenarios(snapshots) {
    const { latest, subject } = latestSubject(snapshots);
    const { subject: priorSubject } = previousSubject(snapshots);
    elements.scenarioGrid.replaceChildren();
    api.listScenarios(subject).forEach((scenarioId, index) => {
      const summary = api.scenarioSummary(subject, scenarioId);
      if (!summary) return;
      const prior = api.scenarioSummary(priorSubject, scenarioId);
      const card = document.createElement("article");
      card.className = `scenario-card scenario-card-${scenarioStatus(summary)}`;
      const status = scenarioStatus(summary);
      const scoreDelta = deltaText(summary.medianScore, prior?.medianScore, {
        formatter: (value) => `${compactNumber(value, 1)} pts`,
      });
      const blockingEvents =
        summary.hardGateFailures +
        summary.technicalFailures +
        summary.missingReports;
      const workflowUrl = executionUrl(latest, scenarioId);
      card.innerHTML = `
        <div class="scenario-card-head">
          <div class="scenario-name">
            <span class="scenario-index">${String(index + 1).padStart(2, "0")}</span>
            <h3>${escapeHtml(titleCase(scenarioId))}</h3>
          </div>
          <span class="table-status status-${status}">
            ${status === "pass" ? "Passed" : status === "incomplete" ? "Incomplete" : "Attention"}
          </span>
        </div>
        <div class="scenario-stats">
          <div class="scenario-stat">
            <span>Median score${summary.threshold === null ? "" : ` · target ${compactNumber(summary.threshold, 0)}`}</span>
            <strong>${summary.medianScore === null ? "—" : compactNumber(summary.medianScore, 1)}</strong>
          </div>
          <div class="scenario-stat">
            <span>Pass rate</span>
            <strong>${formatPercent(summary.passRate)}</strong>
          </div>
          <div class="scenario-stat">
            <span>Cost</span>
            <strong>${formatCurrency(summary.totalCost)}</strong>
          </div>
          <div class="scenario-stat">
            <span>Runtime</span>
            <strong>${formatDuration(summary.wallTime)}</strong>
          </div>
        </div>
        <div class="scenario-foot">
          <span class="${scoreDelta.className}">${escapeHtml(scoreDelta.text)}</span>
          <span>${blockingEvents === 0 ? "No blocking failures" : escapeHtml(failureReason(summary))}</span>
          ${status === "pass" || !workflowUrl ? "" : `<a href="${escapeHtml(workflowUrl)}">Inspect failures ↗</a>`}
        </div>
      `;
      elements.scenarioGrid.append(card);
    });
  }

  function renderHistory(snapshots) {
    elements.historyBody.replaceChildren();
    [...snapshots].reverse().forEach((snapshot) => {
      const subject = snapshot.subjects[state.subjectId];
      const summary = api.subjectSummary(subject);
      if (!summary) return;
      const row = document.createElement("tr");
      const runName = formatDate(snapshot.date, { year: true });
      const runUrl = executionUrl(snapshot) || "#";
      const commitUrl = safeUrl(snapshot.commit?.url) || "#";
      const status = document.createElement("span");
      applyStatus(status, summary, true);
      row.innerHTML = `
        <td>
          <div class="release-cell">
            <a href="${escapeHtml(runUrl)}">${escapeHtml(runName)}</a>
            <span>daily run · ${escapeHtml(snapshot.commit?.id?.slice(0, 7) || "unknown")}</span>
          </div>
        </td>
        <td class="status-cell"></td>
        <td>${formatPercent(summary.scenarioPassRate)}</td>
        <td>${summary.averageScore === null ? "—" : compactNumber(summary.averageScore, 1)}</td>
        <td>${formatCurrency(summary.totalCost)}</td>
        <td>${formatDuration(summary.wallTime)}</td>
        <td><a class="commit-link" href="${escapeHtml(commitUrl)}">${escapeHtml(snapshot.commit?.id?.slice(0, 7) || "unknown")}</a></td>
      `;
      row.querySelector(".status-cell").append(status);
      elements.historyBody.append(row);
    });
  }

  function render() {
    const snapshots = api.filterSnapshots(data, {
      subjectId: state.subjectId,
      days: state.windowDays,
    });
    const hasData = snapshots.length > 0;
    elements.empty.hidden = hasData;
    elements.content.hidden = !hasData;
    elements.releaseCount.textContent =
      `${snapshots.length} daily run${snapshots.length === 1 ? "" : "s"}`;
    const latest = snapshots.at(-1);
    const subject = latest?.subjects?.[state.subjectId];
    elements.subjectContext.textContent = subject
      ? `${subject.provider}/${subject.model}`
      : "No subject";
    if (!hasData) return;
    renderTrend(snapshots);
    renderLatest(snapshots);
    renderScenarios(snapshots);
    renderHistory(snapshots);
  }

  function initialize() {
    elements.previewBadge.hidden = !data.preview;
    if (data.lastUpdate) {
      const iso = new Date(data.lastUpdate).toISOString();
      elements.lastUpdate.dateTime = iso;
      elements.lastUpdate.textContent = formatDate(data.lastUpdate, {
        year: true,
        time: true,
      });
    }
    if (data.repoUrl) {
      const repositoryUrl = safeUrl(data.repoUrl);
      if (repositoryUrl) {
        elements.actionsLink.href =
          `${repositoryUrl.replace(/\/$/, "")}/actions/workflows/` +
          "harness-e2e-daily.yml";
      }
    }
    setOptions(elements.subject, data.subjects, state.subjectId, (subjectId) => {
      const snapshot = [...data.snapshots]
        .reverse()
        .find((item) => item.subjects[subjectId]);
      const subject = snapshot?.subjects?.[subjectId];
      return subject ? `${subject.provider}/${subject.model}` : subjectId;
    });
    refreshScenarioOptions();
    elements.window.value = String(state.windowDays);
    elements.subject.addEventListener("change", () => {
      state.subjectId = elements.subject.value;
      refreshScenarioOptions();
      render();
    });
    elements.scenario.addEventListener("change", () => {
      state.scenarioId = elements.scenario.value;
      render();
    });
    elements.window.addEventListener("change", () => {
      state.windowDays = Number(elements.window.value);
      render();
    });
    document.querySelectorAll(".metric-tab").forEach((button) => {
      button.addEventListener("click", () => {
        state.trendMetric = button.dataset.metric;
        document.querySelectorAll(".metric-tab").forEach((candidate) => {
          candidate.classList.toggle("active", candidate === button);
        });
        render();
      });
    });
    render();
  }

  initialize();
})();
