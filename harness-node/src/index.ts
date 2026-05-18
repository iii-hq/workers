#!/usr/bin/env node
/**
 * Composite entry-point: spins up every harness-node worker in one
 * process. CLI parsing happens once here so the per-worker
 * `bootstrapWorker` calls don't fight over `process.argv`. Each worker's
 * register callback is reused as-is from its own folder, so the bus
 * surface is identical to running them in separate processes.
 */

import { Command } from 'commander';
import { register as registerApprovalGate } from './approval-gate/register.js';
import { register as registerAuthCredentials } from './auth-credentials/register.js';
import { register as registerContextCompaction } from './context-compaction/register.js';
import { register as registerHarness } from './harness/register.js';
import { register as registerHookFanout } from './hook-fanout/register.js';
import { register as registerLlmBudget } from './llm-budget/register.js';
import { register as registerModelsCatalog } from './models-catalog/register.js';
import { register as registerProviderAnthropic } from './provider-anthropic/register.js';
import { register as registerProviderOpenai } from './provider-openai/register.js';
import { logger } from './runtime/otel.js';
import {
  DEFAULT_CONFIG_PATH,
  DEFAULT_URL,
  type WorkerDefinition,
  type WorkerHandle,
  runWorker,
  waitForShutdown,
} from './runtime/worker.js';
import { register as registerSession } from './session/register.js';
import { register as registerTurnOrchestrator } from './turn-orchestrator/register.js';

const WORKERS: readonly WorkerDefinition[] = [
  {
    name: 'harness',
    description:
      'Meta-worker: harness::status, ui::subscribe/unsubscribe, harness::fs::read_inline, policy::check_permissions, agent::events fanout to subscribed browsers.',
    register: (iii, ctx) => registerHarness(iii, ctx),
  },
  {
    name: 'turn-orchestrator',
    description:
      'Durable run::start state machine driving each agent turn through provisioning, assistant, function-execute, steering, tearing-down.',
    register: (iii, ctx) => registerTurnOrchestrator(iii, ctx),
  },
  {
    name: 'approval-gate',
    description:
      'Hook subscriber on agent::before_function_call that consults policy::check_permissions and pauses calls for user approval.',
    register: (iii, ctx) => registerApprovalGate(iii, ctx),
  },
  {
    name: 'session',
    description:
      'Session storage (parent-id tree under session-tree::*) and per-session inbox (session-inbox::*) backed by iii state.',
    register: (iii, ctx) => registerSession(iii, ctx),
  },
  {
    name: 'hook-fanout',
    description:
      'Generic publish-collect primitive: publishes a topic via iii::durable::publish, collects subscriber replies on agent::hook_reply, applies a merge rule, returns the merged result.',
    register: (iii, ctx) => registerHookFanout(iii, ctx),
  },
  {
    name: 'auth-credentials',
    description: 'Credential store for provider API keys and OAuth tokens (auth::*).',
    register: (iii, ctx) => registerAuthCredentials(iii, ctx),
  },
  {
    name: 'models-catalog',
    description: 'Model capabilities catalog on the iii bus (models::list/get/supports/register).',
    register: (iii) => registerModelsCatalog(iii),
  },
  {
    name: 'provider-anthropic',
    description:
      'Anthropic Messages API streaming provider on the iii bus (provider::anthropic::stream + ::complete).',
    register: (iii, ctx) => registerProviderAnthropic(iii, ctx),
  },
  {
    name: 'provider-openai',
    description:
      'OpenAI Chat Completions streaming provider on the iii bus (provider::openai::stream + ::complete).',
    register: (iii, ctx) => registerProviderOpenai(iii, ctx),
  },
  {
    name: 'llm-budget',
    description: 'LLM spend caps with alerts, forecast, period rollover (budget::*).',
    register: (iii) => registerLlmBudget(iii),
  },
  {
    name: 'context-compaction',
    description:
      'Out-of-band session-history compactor. Subscribes to agent::events::TurnEnd and writes a session-tree Compaction entry when the running token count crosses the configured threshold.',
    register: (iii) => registerContextCompaction(iii),
  },
];

async function main(): Promise<void> {
  const program = new Command()
    .name('harness-node')
    .description('Run every harness-node worker in one process.')
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

  logger.info('harness-node starting', {
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
    logger.error('harness-node startup failed; tearing down workers', { err: String(err) });
    await shutdownAll(handles);
    throw err;
  }

  logger.info('harness-node ready', { workers: handles.map((h) => h.name) });

  await waitForShutdown();
  logger.info('harness-node shutting down');
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
