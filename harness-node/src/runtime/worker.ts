/**
 * Common worker bootstrap. Ports the shape used by every Rust worker's
 * `main.rs` (`provider-anthropic/src/main.rs`, etc.):
 *
 *   - parse CLI flags (`--config`, `--url`, `--manifest`)
 *   - call `registerWorker(url, { workerName, ... })`
 *   - run `register(iii, { config, url })`
 *   - install SIGINT/SIGTERM handlers
 *   - call `iii.shutdown()` on exit
 *
 * Each worker passes a `register` callback that wires its bus surface.
 * Anything the callback returns (a refs object, etc.) is held until
 * shutdown — its only purpose is to keep references alive.
 *
 * `runWorker()` exposes the same lifecycle without CLI parsing so a
 * composite entry (see `src/index.ts`) can spin up several workers in a
 * single process.
 */

import { Command } from 'commander';
import { type ISdk, registerWorker } from 'iii-sdk';
import { logger } from './otel.js';

export type RegisterFn = (iii: ISdk, ctx: { configPath: string; url: string }) => Promise<unknown>;

export type WorkerDefinition = {
  /** Worker name reported to the engine (visible in `engine::workers::list`). */
  name: string;
  description?: string;
  /** Hook called once the SDK is connected; should register all bus functions. */
  register: RegisterFn;
};

export type RunWorkerOptions = WorkerDefinition & {
  /** Path to `config.yaml` forwarded to the `register` callback. */
  configPath: string;
  /** iii engine WebSocket URL. */
  url: string;
};

export type WorkerHandle = {
  name: string;
  shutdown: () => Promise<void>;
};

export type BootstrapOptions = WorkerDefinition & {
  /** Default path to `config.yaml`. */
  defaultConfigPath?: string;
  /** Default engine WebSocket URL. */
  defaultUrl?: string;
  /** Optional `--manifest` JSON producer (mirrors the Rust crates' manifest CLI). */
  manifest?: () => Record<string, unknown>;
};

export const DEFAULT_URL = 'ws://127.0.0.1:49134';
export const DEFAULT_CONFIG_PATH = './config.yaml';

/**
 * Connect to the engine, register the worker's bus surface, and return a
 * handle whose `shutdown()` tears the connection down. Does not parse
 * argv or install signal handlers — that is `bootstrapWorker`'s job.
 */
export async function runWorker(opts: RunWorkerOptions): Promise<WorkerHandle> {
  logger.info('worker starting', { name: opts.name, url: opts.url, config: opts.configPath });

  const iii = registerWorker(opts.url, { workerName: opts.name });
  let refs: unknown = null;
  try {
    refs = await opts.register(iii, { configPath: opts.configPath, url: opts.url });
  } catch (err) {
    logger.error('worker register failed', { name: opts.name, err: String(err) });
    await iii.shutdown().catch(() => {});
    throw err;
  }

  return {
    name: opts.name,
    shutdown: async () => {
      // Keep refs alive until shutdown so unregister handles aren't GC'd.
      void refs;
      logger.info('worker shutting down', { name: opts.name });
      await iii.shutdown().catch((err) => {
        logger.warn('shutdown failed', { name: opts.name, err: String(err) });
      });
    },
  };
}

export async function bootstrapWorker(opts: BootstrapOptions): Promise<void> {
  const program = new Command()
    .name(opts.name)
    .description(opts.description ?? '')
    .option('--config <path>', 'config file path', opts.defaultConfigPath ?? DEFAULT_CONFIG_PATH)
    .option(
      '--url <url>',
      'iii engine WebSocket URL',
      process.env.III_URL ?? opts.defaultUrl ?? DEFAULT_URL,
    )
    .option('--manifest', 'print manifest JSON and exit')
    .helpOption('-h, --help', 'print help');
  program.parse();
  const cli = program.opts<{ config: string; url: string; manifest?: boolean }>();

  if (cli.manifest) {
    if (opts.manifest) {
      process.stdout.write(`${JSON.stringify(opts.manifest(), null, 2)}\n`);
    } else {
      process.stdout.write(`{"name":"${opts.name}"}\n`);
    }
    return;
  }

  const handle = await runWorker({
    name: opts.name,
    description: opts.description,
    register: opts.register,
    configPath: cli.config,
    url: cli.url,
  });

  await waitForShutdown();
  await handle.shutdown();
}

/**
 * Resolve when the process receives SIGINT or SIGTERM. Exported so a
 * composite entry can await a single signal and then tear down every
 * worker handle it has created.
 */
export function waitForShutdown(): Promise<void> {
  return new Promise<void>((resolve) => {
    const onSignal = () => {
      process.off('SIGINT', onSignal);
      process.off('SIGTERM', onSignal);
      resolve();
    };
    process.on('SIGINT', onSignal);
    process.on('SIGTERM', onSignal);
  });
}
