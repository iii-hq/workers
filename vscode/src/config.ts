import { readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { isAbsolute, resolve } from 'node:path';
import { parse } from 'yaml';
import { z } from 'zod';

const port = z.number().int().min(1).max(65535);

const RuntimeFields = z.object({
  code_executable: z
    .string()
    .default('')
    .describe('Path to the VS Code CLI; empty = `code` on PATH'),
  data_dir: z
    .string()
    .default('~/.iii/vscode')
    .describe('Directory holding one server-data and cli-data folder per workspace'),
  bind_host: z
    .string()
    .default('127.0.0.1')
    .describe('Loopback address the VS Code Server listens on (127.0.0.1, localhost, or ::1)'),
  port_min: port.default(18080).describe('First port tried for a new workspace'),
  port_max: port.default(18180).describe('Last port tried for a new workspace'),
  start_timeout_ms: z
    .number()
    .int()
    .positive()
    .default(180_000)
    .describe('How long a start waits for the server to answer over HTTP'),
  stop_grace_ms: z
    .number()
    .int()
    .positive()
    .default(5_000)
    .describe('Grace period between SIGTERM and SIGKILL when stopping a server'),
});

const portRange = {
  check: (cfg: { port_min: number; port_max: number }) => cfg.port_min <= cfg.port_max,
  params: { message: 'port_min must not exceed port_max', path: ['port_max'] },
};

export const RuntimeConfigSchema = RuntimeFields.refine(portRange.check, portRange.params);
export type RuntimeConfig = z.infer<typeof RuntimeConfigSchema>;

const ConfigSchema = RuntimeFields.extend({
  engine_url: z.string().default('ws://127.0.0.1:49134'),
}).refine(portRange.check, portRange.params);

export type Config = z.infer<typeof ConfigSchema>;

export function runtimeJsonSchema(): Record<string, unknown> {
  const out = z.toJSONSchema(RuntimeConfigSchema) as Record<string, unknown>;
  delete out.$schema;
  return out;
}

export function toRuntime(cfg: Config): RuntimeConfig {
  const { engine_url: _drop, ...runtime } = cfg;
  return runtime;
}

export function expandHome(path: string, home = homedir()) {
  if (path === '~') return home;
  if (path.startsWith('~/')) return resolve(home, path.slice(2));
  return isAbsolute(path) ? path : resolve(path);
}

export async function loadConfig(path: string): Promise<Config> {
  let raw: unknown = {};
  try {
    raw = parse(await readFile(path, 'utf8')) ?? {};
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err;
  }
  return ConfigSchema.parse(raw);
}
