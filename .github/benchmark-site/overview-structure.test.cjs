const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const root = __dirname;
const index = fs.readFileSync(path.join(root, "index.html"), "utf8");
const execution = fs.readFileSync(path.join(root, "execution.html"), "utf8");
const executionScript = fs.readFileSync(path.join(root, "execution.js"), "utf8");
const sampleExecutions = fs.readFileSync(
  path.join(root, "sample-executions.js"),
  "utf8",
);
const overview = fs.readFileSync(path.join(root, "overview.js"), "utf8");
const localRunner = fs.readFileSync(path.join(root, "local-runner.js"), "utf8");
const styles = fs.readFileSync(path.join(root, "styles.css"), "utf8");
const publisher = fs.readFileSync(
  path.join(root, "..", "scripts", "publish_harness_e2e_dashboard.py"),
  "utf8",
);

test("places efficiency overview first and removes the operational health panel", () => {
  const latest = index.indexOf('class="panel latest-health"');
  const matrix = index.indexOf('class="panel health-panel"');
  const efficiency = index.indexOf('class="panel efficiency-overview"');
  const executions = index.indexOf('class="panel executions-panel"');

  assert.equal(latest, -1);
  assert.ok(efficiency >= 0);
  assert.ok(efficiency < matrix);
  assert.ok(efficiency < executions);
  assert.doesNotMatch(index, /Operational health|latest-daily-execution/i);
});

test("keeps the latest result inside the efficiency overview", () => {
  assert.match(index, /class="efficiency-result"/);
  assert.match(index, /id="efficiency-status"/);
  assert.doesNotMatch(index, /id="kpi-status"/);
});

test("keeps hidden preview and diagnostic controls out of layout", () => {
  assert.match(index, /id="preview-badge"[^>]+hidden/);
  assert.match(styles, /\[hidden\]\s*\{[^}]*display:\s*none\s*!important;/s);
});

test("offers every semantic execution status as a filter", () => {
  for (const status of [
    "passed",
    "quality_advisory",
    "hard_gate_failed",
    "technical_failed",
    "infra_failed",
    "incomplete",
    "cancelled",
    "running",
  ]) {
    assert.match(index, new RegExp(`<option value="${status}">`));
  }
});

test("restores the per-run chat transcript surface", () => {
  assert.match(execution, /execution-transcript\.js/);
  assert.match(execution, /session-transcript-dialog/);
  assert.match(executionScript, /renderConversationLaunch/);
  assert.match(executionScript, /conversation-open/);
  assert.match(executionScript, /openConversationDialog/);
  assert.match(executionScript, /id: "prompt", label: "Prompt"/);
  assert.match(executionScript, /id: "sessions", label: "Sessions"/);
  assert.match(executionScript, /Complete run record/);
  assert.match(sampleExecutions, /availability: index < 3 \? "full"/);
  assert.match(sampleExecutions, /transcript:\s*\{/);
  assert.match(sampleExecutions, /criteria:\s*\[/);
  assert.match(sampleExecutions, /traces:\s*\{/);
});

test("uses the delta meaning to color efficiency sparklines", () => {
  assert.match(overview, /const efficiencyTrendColors =/);
  assert.match(overview, /efficiencyTrendColors\[meta\.css\]/);
  assert.doesNotMatch(overview, /const palette =/);
});

test("discovers local models and scenarios while keeping runner knobs advanced", () => {
  assert.match(index, /id="local-subject"[^>]+disabled/);
  assert.match(index, /id="local-scenario-options"/);
  assert.match(index, /class="local-advanced local-field-wide"/);
  assert.match(index, /id="local-catalog-refresh"/);
  assert.doesNotMatch(index, /name="model"|name="provider"/);
  assert.match(localRunner, /api\/local\/catalog/);
  assert.match(localRunner, /catalog\.models/);
  assert.match(localRunner, /catalog\.scenarios/);
});

test("uses the native local-run contract without import compatibility", () => {
  assert.match(overview, /Last completed/);
  assert.match(localRunner, /Results saved/);
  assert.match(localRunner, /job\?\.status === "completed" && job\.id/);
  assert.doesNotMatch(
    overview + localRunner,
    /execution_id|Results imported|results\.json file/,
  );
});

test("loads the local runner only for native local manifests", () => {
  assert.match(
    index,
    /if \(window\.HARNESS_EXECUTIONS\?\.mode === "local"\) \{\s*document\.write\('<script src="\.\/local-runner\.js"/s,
  );
  assert.match(overview, /if \(isLocal\) window\.HarnessLocalRunner\.initialize\(\)/);
  assert.doesNotMatch(overview, /api\/local|local-run-form|local-run-cancel/);
  assert.match(publisher, /"mode": "published"/);

  const loader = index.match(
    /<script>\s*(if \(window\.HARNESS_EXECUTIONS\?\.mode === "local"\) \{[\s\S]*?local-runner\.js[\s\S]*?\})\s*<\/script>/,
  )[1];
  const writesFor = (mode) => {
    const writes = [];
    vm.runInNewContext(loader, {
      document: { write: (value) => writes.push(value) },
      window: { HARNESS_EXECUTIONS: { mode } },
    });
    return writes;
  };
  assert.deepEqual(writesFor("published"), []);
  assert.deepEqual(writesFor("local"), [
    '<script src="./local-runner.js"></script>',
  ]);
});

test("keeps the completed runner log inside a padded local panel", () => {
  assert.match(index, /id="local-run-log" class="local-run-log"/);
  assert.match(styles, /\.local-runner\s*\{[^}]*padding:\s*28px 30px;[^}]*overflow:\s*hidden;/s);
  assert.match(styles, /\.local-run-log-shell\s*\{[^}]*overflow:\s*hidden;/s);
  assert.match(styles, /\.local-run-log\s*\{[^}]*max-width:\s*100%;[^}]*overflow-wrap:\s*anywhere;/s);
});

test("keeps comparison content padded with contained long values", () => {
  assert.match(index, /href="\.\/compare\.html/);
  const compare = fs.readFileSync(path.join(root, "compare.html"), "utf8");
  assert.match(compare, /id="compare-content" class="compare-content"/);
  assert.match(styles, /\.compare-content\s*>\s*\.panel\s*\{[^}]*padding:\s*28px 30px;[^}]*overflow:\s*hidden;/s);
  assert.match(styles, /\.compare-selection-card h2\s*\{[^}]*overflow-wrap:\s*anywhere;/s);
  assert.match(styles, /\.compare-metric-card\s*\{[^}]*overflow:\s*hidden;/s);
});
