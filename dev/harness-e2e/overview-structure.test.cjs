const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = __dirname;
const index = fs.readFileSync(path.join(root, "index.html"), "utf8");
const execution = fs.readFileSync(path.join(root, "execution.html"), "utf8");
const sampleExecutions = fs.readFileSync(
  path.join(root, "sample-executions.js"),
  "utf8",
);
const styles = fs.readFileSync(path.join(root, "styles.css"), "utf8");

test("places operational health before scenarios, efficiency, and history", () => {
  const latest = index.indexOf('class="panel latest-health"');
  const matrix = index.indexOf('class="panel health-panel"');
  const efficiency = index.indexOf('class="panel efficiency-overview"');
  const executions = index.indexOf('class="panel executions-panel"');

  assert.ok(latest >= 0);
  assert.ok(latest < matrix);
  assert.ok(matrix < efficiency);
  assert.ok(efficiency < executions);
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

test("publishes diagnostics without transcript or prompt surfaces", () => {
  assert.doesNotMatch(execution, /execution-transcript\.js|session-transcript-dialog/);
  assert.match(execution, /Prompts,\s*transcripts, model responses/s);
  assert.doesNotMatch(sampleExecutions, /["'](?:prompt|transcript)["']\s*:/);
});
