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
    renderMatrix();
    renderTable();
  }

  function initialize() {
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
  }

  initialize();
})();
