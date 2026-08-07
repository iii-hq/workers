const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = __dirname;
const index = fs.readFileSync(path.join(root, "index.html"), "utf8");
const execution = fs.readFileSync(path.join(root, "execution.html"), "utf8");
const executionScript = fs.readFileSync(path.join(root, "execution.js"), "utf8");
const sampleExecutions = fs.readFileSync(
  path.join(root, "sample-executions.js"),
  "utf8",
);
const overview = fs.readFileSync(path.join(root, "overview.js"), "utf8");
const styles = fs.readFileSync(path.join(root, "styles.css"), "utf8");

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
