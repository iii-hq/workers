import { readFile } from 'node:fs/promises';
import { parse } from 'yaml';
import { z } from 'zod';

const ConfigSchema = z.object({
  engine_url: z.string().default('ws://127.0.0.1:49134'),
  defaults: z
    .object({
      model: z
        .string()
        .default('')
        .describe('Model id used when a run omits one; empty = Pi default'),
      thinking_level: z
        .enum(['off', 'minimal', 'low', 'medium', 'high', 'xhigh'])
        .default('medium')
        .describe('Reasoning effort for the turn'),
      cwd: z.string().default('').describe('Default working directory a turn runs in'),
      tools: z.array(z.string()).default([]).describe('Tools Pi may use (empty = its default set)'),
      agent_dir: z.string().default('').describe('Directory of custom Pi agent definitions'),
    })
    .prefault({})
    .describe('Per-turn defaults applied when a pi::run payload omits a field'),
  events_stream: z
    .string()
    .default('agent::events')
    .describe('Stream carrying the translated AgentEvent frames'),
  raw_events_stream: z
    .string()
    .default('pi::events')
    .describe('Stream carrying the raw Pi events, verbatim'),
  iii_context: z
    .boolean()
    .default(true)
    .describe('Prepend the iii runtime context so the agent discovers engine functions'),
});

export type Config = z.infer<typeof ConfigSchema>;

/**
 * The slice managed by the `configuration` worker. `engine_url` is excluded —
 * it is bootstrap (needed to reach the configuration worker), so it stays on
 * the local seed / `--url` and never hot-reloads.
 */
export const RuntimeConfigSchema = ConfigSchema.omit({ engine_url: true });
export type RuntimeConfig = z.infer<typeof RuntimeConfigSchema>;

/** JSON Schema published to the configuration worker. The registry validator
 *  has no `$schema` meta-schema, so strip the draft-2020-12 `$schema` key. */
export function runtimeJsonSchema(): Record<string, unknown> {
  const out = z.toJSONSchema(RuntimeConfigSchema) as Record<string, unknown>;
  delete out.$schema;
  return out;
}

/** The runtime slice of a full config, for use as `initial_value`. */
export function toRuntime(cfg: Config): RuntimeConfig {
  const { engine_url: _drop, ...runtime } = cfg;
  return runtime;
}

export async function loadConfig(path: string): Promise<Config> {
  let raw: unknown = {};
  try {
    raw = parse(await readFile(path, 'utf8')) ?? {};
  } catch (err) {
    // a missing config file falls back to defaults; anything else
    // (YAML parse error, permissions) must fail the worker fast
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err;
  }
  return ConfigSchema.parse(raw);
}
