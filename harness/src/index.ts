#!/usr/bin/env node
/**
 * Composite entry-point: spins up every harness worker in one
 * process. CLI parsing happens once here so the per-worker
 * `bootstrapWorker` calls don't fight over `process.argv`. Each worker's
 * register callback is reused as-is from its own folder, so the bus
 * surface is identical to running them in separate processes.
 */

import { Command } from 'commander';
import { register as registerApprovalGate } from './approval-gate/resolve.js';
import { initDefaultMode } from './approval-gate/settings/default-mode.js';
import { registerSettingsHandlers as registerApprovalSettings } from './approval-gate/settings/register.js';
import { register as registerContextCompaction } from './context-compaction/register.js';
import { register as registerHarness } from './harness/register.js';
import { register as registerHookFanout } from './hook-fanout/register.js';
import { register as registerLlmBudget } from './llm-budget/register.js';
import { logger } from './runtime/otel.js';
import {
  DEFAULT_CONFIG_PATH,
  DEFAULT_URL,
  type WorkerDefinition,
  type WorkerHandle,
  runWorker,
  waitForShutdown,
} from './runtime/worker.js';
import { register as registerTurnOrchestrator } from './turn-orchestrator/register.js';
import { register as registerWeb } from './web/register.js';

const WORKERS: readonly WorkerDefinition[] = [
  {
    name: 'harness',
    description:
      'Meta-worker: ui::models::subscribe/unsubscribe, harness::fs::read_inline, policy::check_permissions.',
    register: (iii, ctx) => registerHarness(iii, ctx),
  },
  {
    name: 'turn-orchestrator',
    description:
      'Durable run::start state machine driving each agent turn through provisioning, assistant, and function-execute.',
    register: (iii, ctx) => registerTurnOrchestrator(iii, ctx),
  },
  {
    name: 'approval-gate',
    description:
      'Registers approval::resolve and the per-session mode/allow-list settings handlers; persists human decisions to the approvals and approval_settings scopes.',
    register: async (iii) => {
      await registerApprovalGate(iii);
      registerApprovalSettings(iii);
      await initDefaultMode(iii);
    },
  },
  {
    name: 'hook-fanout',
    description:
      'Generic publish-collect primitive: publishes a topic via iii::durable::publish, collects subscriber replies on agent::hook_reply, applies a merge rule, returns the merged result.',
    register: (iii, ctx) => registerHookFanout(iii, ctx),
  },
  {
    name: 'llm-budget',
    description: 'LLM spend caps with alerts, forecast, period rollover (budget::*).',
    register: (iii) => registerLlmBudget(iii),
  },
  {
    name: 'context-compaction',
    description:
      'Thin user-initiated /compact wrapper (context-compaction::compact_session) over the context-manager worker; persists the compaction round trip as a session bookkeeping entry. The transcript is never modified.',
    register: (iii) => registerContextCompaction(iii),
  },
  {
    name: 'web',
    description: 'Outbound HTTP client on the iii bus (web::fetch).',
    register: (iii, ctx) => registerWeb(iii, ctx),
  },
];

async function main(): Promise<void> {
  const program = new Command()
    .name('harness')
    .description('Run every harness worker in one process.')
    .option('--config <path>', 'config file path', DEFAULT_CONFIG_PATH)
    .option('--url <url>', 'iii engine WebSocket URL', process.env.III_URL ?? DEFAULT_URL)
    .option('--manifest', 'print combined manifest JSON and exit')
    .helpOption('-h, --help', 'print help');
  program.parse();
  const cli = program.opts<{ config: string; url: string; manifest?: boolean }>();

  if (cli.manifest) {
    const manifest = WORKERS.map(({ name, description }) => ({ name, description }));
    process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
    return;
  }

  logger.info('harness starting', {
    url: cli.url,
    config: cli.config,
    workers: WORKERS.length,
  });

  const handles: WorkerHandle[] = [];
  try {
    for (const def of WORKERS) {
      const handle = await runWorker({
        name: def.name,
        description: def.description,
        register: def.register,
        configPath: cli.config,
        url: cli.url,
      });
      handles.push(handle);
    }
  } catch (err) {
    logger.error('harness startup failed; tearing down workers', { err: String(err) });
    await shutdownAll(handles);
    throw err;
  }

  logger.info('harness ready', { workers: handles.map((h) => h.name) });

  await waitForShutdown();
  logger.info('harness shutting down');
  await shutdownAll(handles);
}

async function shutdownAll(handles: readonly WorkerHandle[]): Promise<void> {
  await Promise.all(
    handles.map((h) =>
      h.shutdown().catch((err) => {
        logger.warn('worker shutdown failed', { name: h.name, err: String(err) });
      }),
    ),
  );
}

await main();
