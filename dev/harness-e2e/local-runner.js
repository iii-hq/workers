(function defineHarnessLocalRunner(global) {
  "use strict";

  const elements = {
    advanced: document.querySelector("#local-advanced"),
    cancel: document.querySelector("#local-run-cancel"),
    catalogIndicator: document.querySelector("#local-catalog-indicator"),
    catalogRefresh: document.querySelector("#local-catalog-refresh"),
    catalogStatus: document.querySelector("#local-catalog-status"),
    connectionUrl: document.querySelector("#local-connection-url"),
    form: document.querySelector("#local-run-form"),
    judge: document.querySelector("#local-judge"),
    runError: document.querySelector("#local-run-error"),
    runLog: document.querySelector("#local-run-log"),
    runLogShell: document.querySelector("#local-run-log-shell"),
    runStatus: document.querySelector("#local-run-status"),
    scenarioAll: document.querySelector("#local-scenario-all"),
    scenarioNone: document.querySelector("#local-scenario-none"),
    scenarioOptions: document.querySelector("#local-scenario-options"),
    scenarioPicker: document.querySelector("#local-scenario-picker"),
    scenarioSummary: document.querySelector("#local-scenario-summary"),
    subject: document.querySelector("#local-subject"),
    submit: document.querySelector("#local-run-submit"),
  };

  let pollTimer = null;
  let catalogReady = false;
  let catalogLoading = false;
  let jobActive = false;
  let defaults = {};
  let initialized = false;

  function formField(name) {
    return elements.form.elements.namedItem(name);
  }

  function applyDefaults(nextDefaults) {
    defaults = { ...defaults, ...(nextDefaults || {}) };
    Object.entries(nextDefaults || {}).forEach(([name, value]) => {
      const field = formField(name);
      if (field && !field.value && value !== null && value !== undefined) {
        field.value = String(value);
      }
    });
    const url = formField("url")?.value || nextDefaults?.url || "";
    if (url) elements.connectionUrl.textContent = url;
  }

  function setControls(active) {
    jobActive = active;
    for (const field of elements.form.elements) {
      if (field !== elements.cancel) field.disabled = active;
    }
    elements.subject.disabled = active || !catalogReady;
    elements.judge.disabled = active || !catalogReady;
    elements.submit.disabled = active || !catalogReady;
    elements.catalogRefresh.disabled = active || catalogLoading;
    elements.scenarioPicker.classList.toggle(
      "local-picker-disabled",
      active || !catalogReady,
    );
    elements.scenarioPicker.setAttribute(
      "aria-disabled",
      String(active || !catalogReady),
    );
  }

  function renderJob(response) {
    applyDefaults(response?.defaults);
    const job = response?.job;
    const active = ["running", "cancelling"].includes(job?.status);
    setControls(active);
    elements.cancel.hidden = !active;
    elements.runError.hidden = !job?.error;
    elements.runError.textContent = job?.error || "";
    elements.runLogShell.hidden = !job?.log;
    elements.runLog.textContent = job?.log || "";
    if (job?.log && active) elements.runLogShell.open = true;
    elements.runStatus.textContent = !job
      ? "Ready"
      : {
          running: "Running…",
          cancelling: "Cancelling…",
          cancelled: "Cancelled",
          completed: "Results saved",
          failed: "Runner failed",
        }[job.status] || job.status;
    if (active) {
      clearTimeout(pollTimer);
      pollTimer = setTimeout(refreshJob, 1_000);
    } else if (job?.status === "completed" && job.id) {
      const reloadKey = "harness-e2e-local-last-reload";
      if (sessionStorage.getItem(reloadKey) !== job.id) {
        sessionStorage.setItem(reloadKey, job.id);
        global.location.reload();
      }
    }
  }

  async function api(path, options = {}) {
    const response = await fetch(path, {
      cache: "no-store",
      headers: { "Content-Type": "application/json" },
      ...options,
    });
    const payload = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(payload.error || `Request failed (${response.status})`);
    }
    return payload;
  }

  async function refreshJob() {
    try {
      const response = await api("./api/local/run");
      renderJob(response);
      return response;
    } catch (error) {
      elements.runError.hidden = false;
      elements.runError.textContent = error.message;
      elements.runStatus.textContent = "Unavailable";
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
      (defaults.model && defaults.provider
        ? modelKey({ model: defaults.model, provider: defaults.provider })
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
    return [...elements.scenarioOptions.querySelectorAll("input[type=checkbox]")];
  }

  function updateScenarioSummary() {
    const inputs = scenarioInputs();
    const selected = inputs.filter((input) => input.checked).length;
    if (!inputs.length) {
      elements.scenarioSummary.textContent = catalogLoading
        ? "Loading scenarios…"
        : "Catalog unavailable";
      elements.submit.disabled = true;
      return;
    }
    elements.scenarioSummary.textContent =
      selected === inputs.length
        ? `All ${inputs.length} scenarios`
        : `${selected} of ${inputs.length} scenarios`;
    elements.submit.disabled = jobActive || !catalogReady || selected === 0;
  }

  function fillScenarios(scenarios) {
    const previous = new Set(
      scenarioInputs().filter((input) => input.checked).map((input) => input.value),
    );
    const selectAll = previous.size === 0;
    elements.scenarioOptions.replaceChildren();
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
      elements.scenarioOptions.append(label);
    });
    updateScenarioSummary();
  }

  async function refreshCatalog() {
    const url = formField("url")?.value || defaults.url || "";
    elements.connectionUrl.textContent = url;
    elements.catalogStatus.textContent = "Discovering the running Harness…";
    elements.catalogIndicator.className = "local-connection-dot";
    catalogLoading = true;
    catalogReady = false;
    setControls(jobActive);
    try {
      const query = new URLSearchParams({ url });
      const catalog = await api(`./api/local/catalog?${query}`);
      fillModelSelect(elements.subject, catalog.models);
      fillModelSelect(elements.judge, catalog.models, { includeAutomatic: true });
      fillScenarios(catalog.scenarios);
      catalogReady = true;
      elements.catalogIndicator.className = "local-connection-dot connected";
      elements.catalogStatus.textContent =
        `${catalog.models.length} registered model${catalog.models.length === 1 ? "" : "s"} · ${catalog.scenarios.length} scenarios`;
      elements.runError.hidden = true;
    } catch (error) {
      elements.catalogIndicator.className = "local-connection-dot failed";
      elements.catalogStatus.textContent = "Could not read the Harness catalog";
      elements.runError.hidden = false;
      elements.runError.textContent = error.message;
      elements.scenarioSummary.textContent = "Catalog unavailable";
      elements.advanced.open = true;
    } finally {
      catalogLoading = false;
      setControls(jobActive);
      updateScenarioSummary();
    }
  }

  function initialize() {
    if (initialized) return;
    initialized = true;
    elements.form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const values = new FormData(elements.form);
      const subject = selectedModel(elements.subject);
      const judge = selectedModel(elements.judge);
      const scenarios = scenarioInputs()
        .filter((input) => input.checked)
        .map((input) => input.value);
      try {
        if (!subject) throw new Error("Select a registered subject model.");
        if (!scenarios.length) throw new Error("Select at least one scenario.");
        localStorage.setItem("harness-e2e-local-subject", modelKey(subject));
        elements.runError.hidden = true;
        renderJob(
          await api("./api/local/run", {
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
        elements.runError.hidden = false;
        elements.runError.textContent = error.message;
        elements.runStatus.textContent = "Could not start";
      }
    });
    elements.cancel.addEventListener("click", async () => {
      try {
        renderJob(
          await api("./api/local/run/cancel", {
            method: "POST",
            body: "{}",
          }),
        );
      } catch (error) {
        elements.runError.hidden = false;
        elements.runError.textContent = error.message;
      }
    });
    elements.catalogRefresh.addEventListener("click", refreshCatalog);
    elements.scenarioAll.addEventListener("click", () => {
      scenarioInputs().forEach((input) => {
        input.checked = true;
      });
      updateScenarioSummary();
    });
    elements.scenarioNone.addEventListener("click", () => {
      scenarioInputs().forEach((input) => {
        input.checked = false;
      });
      updateScenarioSummary();
    });
    elements.scenarioOptions.addEventListener("change", updateScenarioSummary);
    elements.scenarioPicker.addEventListener("toggle", () => {
      if (!catalogReady && elements.scenarioPicker.open) {
        elements.scenarioPicker.open = false;
      }
    });
    refreshJob().then(refreshCatalog);
  }

  global.HarnessLocalRunner = { initialize };
})(window);
