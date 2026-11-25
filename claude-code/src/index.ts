#!/usr/bin/env node
/**
 * Worker bootstrap: connect, register the configuration schema, fetch the live
 * config, register claude::* functions, wait for SIGINT/SIGTERM. Config is
 * managed by the `configuration` worker (config.yaml is the seed); the live
 * value hot-reloads. Mirrors the binary-worker lifecycle.
 *
 * Two halves, one worker and one login: `claude::run` drives Claude Code
 * headless from here, and `claude::terminal::*` drives the same CLI in a
 * `shell::pty` session a person types into. Both report onto the same
 * `claude::on-agent-event` feed (raw Claude Code messages: `claude::on-raw-event`).
 */

import { parseArgs } from 'node:util';
import { registerWorker } from 'iii-sdk';
import { type Config, loadConfig } from './config.js';
import {
  type ConfigHolder,
  ReplyPermission,
  observeConfigChanges,
  registerClaudeConfig,
} from './configuration.js';
import { registerAgentFeeds } from './agent-feed.js';
import { registerAgents } from './agents.js';
import { registerExternalReplies } from './external-replies.js';
import { pluginSourceLine } from './iii-plugin/install.js';
import { resolveClaudeExecutable } from './executable.js';
import { ranSinceStart, register, startupScanDone } from './run.js';
import { startStatusRecovery } from './session-link.js';
import { settleStaleTurns } from './turns.js';
import { registerActivity } from './terminal/activity.js';
import { registerAuth } from './terminal/auth.js';
import { registerTerminal } from './terminal/terminal.js';
import { registerUi } from './terminal/ui.js';
import { type HostPrepared, inventoryHost, prepareHost } from './terminal/workspace.js';

const { values } = parseArgs({
  options: {
    config: { type: 'string', default: './config.yaml' },
    url: { type: 'string' },
  },
  strict: false,
});

const seed = await loadConfig(String(values.config));
const url =
  (values.url ? String(values.url) : undefined) ??
  process.env.III_URL ??
  process.env.III_ENGINE_URL ??
  seed.engine_url;
const bootConfig: Config = {
  ...seed,
  engine_url: url,
  claude_executable: resolveClaudeExecutable(seed.claude_executable),
};

// Child processes (headless turns and the hooks they run) find the engine
// through III_URL; make it the address we connected to.
process.env.III_URL = url;

const iii = registerWorker(url, { workerName: 'claude-code' });

// Best-effort: a configuration-worker hiccup at boot must not stop the worker
// from registering claude::*; it falls back to the seed via fetchRuntime.
try {
  await registerClaudeConfig(iii, bootConfig);
} catch (err) {
  console.warn(`configuration initialization failed; continuing with the seed: ${String(err)}`);
}

// Live snapshot: start from the seed, then refresh from the configuration
// worker. `claude_executable` is re-resolved on every refresh so a live change
// to it (or an empty value) re-runs the PATH lookup.
const holder: ConfigHolder = { current: bootConfig };

// The terminal host: the CLI (installed when missing), the `iii` CLI, the
// user. It talks to the `ide` worker and can take minutes on a cold host, so
// it is chained off the config reload rather than awaited: `claude::*` must
// register whether or not there is a terminal host to prepare. Everything a
// SESSION depends on (its folder, the plugin there, the http address,
// default-mcp) is prepared when the session starts, by
// claude::terminal::describe.
let prepared: HostPrepared = {
  home: bootConfig.terminal.workspace_dir,
  executable: '',
  env: {},
  detail: 'the terminal host has not been prepared yet',
  bridge: '',
};
let preparing: Promise<void> = Promise.resolve();
// A queued reconcile must not let a slower, superseded preparation overwrite
// a newer one's result: each run is stamped with the generation current when
// it started, and only commits `prepared` if that generation is still current.
let preparationGeneration = 0;
const reconcileTerminal = () => {
  const generation = ++preparationGeneration;
  const config = holder.current.terminal;
  preparing = preparing
    .then(async () => {
      const candidate = await prepareHost(iii, config);
      if (generation !== preparationGeneration) return;
      prepared = candidate;
      if (prepared.detail) console.warn(`claude-code terminal: ${prepared.detail}`);
      else console.log(`claude-code terminal: ${prepared.executable} ready`);
    })
    .catch((err) => {
      if (generation !== preparationGeneration) return;
      prepared = { ...prepared, executable: '', detail: String(err) };
      console.warn(`claude-code terminal: host is not ready: ${String(err)}`);
    });
};

const replyPermission = new ReplyPermission();
let watchingConfig = false;
const refresh = async () => {
  const latest = await replyPermission.refresh(iii, () => watchingConfig);
  if (!latest) return;
  const { runtime } = latest;
  const merged: Config = runtime
    ? { engine_url: bootConfig.engine_url, ...runtime }
    : { ...bootConfig };
  merged.claude_executable = resolveClaudeExecutable(merged.claude_executable);
  holder.current = merged;
  // A new workspace or binary applies to the next session, without a restart.
  reconcileTerminal();
};

await observeConfigChanges(iii, refresh, (active) => {
  watchingConfig = active;
  if (!active) replyPermission.invalidate();
});

// The two event feeds: claude::on-agent-event (AgentEvent frames) and
// claude::on-raw-event (verbatim Claude Code messages), each with its own
// observers and sequence counter.
const feeds = registerAgentFeeds(iii);
const emit = feeds.agent.emit;
const emitRaw = feeds.raw.emit;
register(
  iii,
  () => holder.current,
  emit,
  emitRaw,
  (id) => agents.heldElsewhere(id),
);
// A session this worker left `working` (a lost final status, or a restart
// mid-run) is settled now: nothing of ours can be running yet.
// Statuses this worker could not write (a lost final status, or a restart
// mid-run) are settled now, retried while session-manager is not up, and
// again whenever session-manager registers; runs since startup stay tracked
// until the first scan that listed successfully.
startStatusRecovery(iii, ranSinceStart, startupScanDone);
// Run records left `working` by the previous process get their settle, so an
// observer waiting on one of those turns is woken.
void settleStaleTurns(iii, ranSinceStart);

// The terminal half: the hook sink, the session inventory (which reads the
// sink's sessions), what a session runs, who pays for it, and
// the console page that opens it.
const tracker = registerActivity(iii, emit);
const agents = registerAgents(
  iii,
  () => inventoryHost(prepared),
  () => holder.current.external_sessions,
  tracker,
);
registerExternalReplies(iii, replyPermission, agents.replyRow);
registerTerminal(
  iii,
  () => prepared,
  () => holder.current.terminal,
  agents.resolveOpen,
);
registerAuth(iii, () => prepared);
registerUi(iii);

console.log(`claude-code worker connected to ${url}`);
console.log(pluginSourceLine());

const shutdown = async () => {
  try {
    await iii.shutdown?.();
  } finally {
    process.exit(0);
  }
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
