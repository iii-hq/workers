#!/usr/bin/env node
// Prompt eval runner: benches system-prompt variants against a LIVE rig.
//
// For every (arm x scenario) pair it starts a fresh harness session via
// `iii trigger harness::send`, waits for the run to settle, reads the
// transcript back through `session::messages`, and grades it with the
// scenario's assertions. Arms differ only in the `system_prompt` /
// `system_prompt_strategy` send options; the "current" arm sends no
// override, so it exercises whatever the router serves the rig today.
//
// Requires: a running engine + harness + a real provider, and the `iii`
// CLI on PATH. This is deliberately NOT the deterministic conformance
// suite (harness/evals/integration): scripted routers cannot measure
// prompt-driven behavior, so these scenarios run against live models and
// grade conduct (what was called, in what order) rather than exact text.
//
// Usage:
//   node run.mjs                         # all arms, all scenarios
//   node run.mjs --scenario bulk-two-fields --arm candidate
//   node run.mjs --address <engine-host> --out ./out

import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

function parseArgs(argv) {
  const args = { out: join(HERE, "out") };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--scenario") args.scenario = argv[++i];
    else if (a === "--arm") args.arm = argv[++i];
    else if (a === "--address") args.address = argv[++i];
    else if (a === "--out") args.out = argv[++i];
    else if (a === "--help") {
      console.log("node run.mjs [--scenario id[,id]] [--arm name[,name]] [--address host] [--out dir]");
      process.exit(0);
    }
  }
  return args;
}

const ARGS = parseArgs(process.argv.slice(2));

function trig(fn, payload, timeoutMs = 90_000) {
  const cli = ["trigger", fn, "--json", JSON.stringify(payload)];
  if (ARGS.address) cli.push("--address", ARGS.address);
  const r = spawnSync("iii", cli, { encoding: "utf8", timeout: timeoutMs });
  try {
    return JSON.parse(r.stdout);
  } catch {
    return { _raw: (r.stdout || "").slice(-300), _err: (r.stderr || "").slice(-300) };
  }
}

const sleep = (ms) => new Promise((res) => setTimeout(res, ms));

function countTokens(model, text) {
  const out = trig("context::count-tokens", { model: { id: model }, system_prompt: text, messages: [] });
  return out.tokens ?? out.token_count ?? null;
}

async function waitSettled(sessionId, timeoutS, quietS) {
  const t0 = Date.now();
  let quiet = 0;
  while ((Date.now() - t0) / 1000 < timeoutS) {
    await sleep(6000);
    const st = trig("harness::status", { session_id: sessionId });
    const terminal = ["completed", "failed", "cancelled", undefined, null].includes(st.status);
    quiet = terminal ? quiet + 6 : 0;
    if (quiet >= quietS) break;
  }
  return trig("harness::status", { session_id: sessionId });
}

function readTranscript(sessionId) {
  const resp = trig("session::messages", { session_id: sessionId });
  const calls = [];
  let generations = 0;
  for (const m of resp.messages ?? []) {
    const msg = m.message ?? {};
    if (msg.role === "assistant") {
      generations++;
      for (const b of msg.content ?? []) {
        if (b.type === "function_call") calls.push(b.arguments?.function ?? b.function_id ?? "");
      }
    } else if (msg.role === "function_result") {
      if (!calls.length || calls[calls.length - 1] !== msg.function_id) calls.push(msg.function_id);
    }
  }
  return { calls: calls.filter(Boolean), generations };
}

function grade(assertions, armName, { calls, result }) {
  const outcomes = [];
  for (const a of assertions) {
    if (a.arms && !a.arms.includes(armName)) continue;
    const re = a.pattern ? new RegExp(a.pattern) : null;
    let pass;
    switch (a.type) {
      case "result_matches":
        pass = re.test(result ?? "");
        break;
      case "calls_include":
        pass = calls.some((c) => re.test(c));
        break;
      case "calls_exclude":
        pass = !calls.some((c) => re.test(c));
        break;
      case "call_order": {
        const before = calls.findIndex((c) => new RegExp(a.before).test(c));
        const after = calls.findIndex((c) => new RegExp(a.after).test(c));
        pass = after === -1 || (before !== -1 && before < after);
        break;
      }
      default:
        pass = false;
    }
    outcomes.push({ ...a, pass: pass || Boolean(a.optional), raw_pass: pass });
  }
  return outcomes;
}

const spec = JSON.parse(readFileSync(join(HERE, "scenarios.json"), "utf8"));
const runId = Date.now().toString(36).slice(-5);
const wantScenarios = ARGS.scenario ? ARGS.scenario.split(",") : null;
const wantArms = ARGS.arm ? ARGS.arm.split(",") : null;

const arms = spec.arms
  .filter((a) => !wantArms || wantArms.includes(a.name))
  .map((a) => ({
    ...a,
    prompt: a.system_prompt_file ? readFileSync(resolve(HERE, a.system_prompt_file), "utf8") : null,
  }));
const scenarios = spec.scenarios.filter((s) => !wantScenarios || wantScenarios.includes(s.id));
if (!arms.length || !scenarios.length) {
  console.error("nothing selected: check --arm / --scenario against scenarios.json");
  process.exit(2);
}

const report = { run_id: runId, model: spec.model, arms: {}, results: [] };
for (const arm of arms) {
  const served = arm.prompt ?? trig("router::system_prompt::get", {}).system_prompt ?? "";
  report.arms[arm.name] = {
    prompt_tokens: served ? countTokens(spec.model, served) : null,
    source:
      arm.system_prompt_file ??
      (served ? "router-served" : "router-served (no default provider resolved; count unavailable)"),
  };
}

let failed = 0;
for (const scenario of scenarios) {
  for (const arm of arms) {
    const sessionId = `pbench-${runId}-${arm.name}-${scenario.id}`;
    const options = { max_turns: scenario.max_turns, functions: { allow: ["*"] } };
    if (arm.prompt) {
      options.system_prompt = arm.prompt;
      options.system_prompt_strategy = arm.strategy ?? "override";
    }
    const sent = trig("harness::send", {
      session_id: sessionId,
      model: spec.model,
      message: scenario.message,
      options,
    });
    if (!sent.accepted) {
      console.error(`SEND FAILED ${arm.name}/${scenario.id}:`, JSON.stringify(sent).slice(0, 200));
      failed++;
      continue;
    }
    console.log(`running ${arm.name}/${scenario.id} (${sessionId})`);
    const status = await waitSettled(sessionId, scenario.timeout_s, scenario.quiet_s);
    const transcript = readTranscript(sessionId);
    const outcomes = grade(scenario.assertions, arm.name, {
      calls: transcript.calls,
      result: String(status.result ?? ""),
    });
    const scenarioFailed = outcomes.some((o) => !o.pass);
    if (scenarioFailed) failed++;
    report.results.push({
      scenario: scenario.id,
      arm: arm.name,
      session_id: sessionId,
      status: status.status,
      turns: status.turn_count,
      generations: transcript.generations,
      calls: transcript.calls,
      result_head: String(status.result ?? "").slice(0, 200),
      assertions: outcomes,
      pass: !scenarioFailed,
    });
    console.log(`  ${scenarioFailed ? "FAIL" : "pass"} turns=${status.turn_count} gens=${transcript.generations}`);
  }
}

mkdirSync(ARGS.out, { recursive: true });
writeFileSync(join(ARGS.out, `report-${runId}.json`), JSON.stringify(report, null, 2));

const lines = [
  `# Prompt eval run ${runId}`,
  "",
  `Model: ${spec.model}`,
  "",
  "| arm | prompt tokens | source |",
  "|---|---|---|",
  ...Object.entries(report.arms).map(([n, a]) => `| ${n} | ${a.prompt_tokens ?? "?"} | ${a.source} |`),
  "",
  "| scenario | arm | pass | turns | gens | calls |",
  "|---|---|---|---|---|---|",
  ...report.results.map(
    (r) => `| ${r.scenario} | ${r.arm} | ${r.pass ? "pass" : "FAIL"} | ${r.turns} | ${r.generations} | ${r.calls.length} |`
  ),
];
writeFileSync(join(ARGS.out, `report-${runId}.md`), lines.join("\n") + "\n");
console.log(`\nreport: ${join(ARGS.out, `report-${runId}.md`)}`);
if (failed) {
  console.error(`${failed} scenario run(s) failed`);
  process.exit(1);
}
