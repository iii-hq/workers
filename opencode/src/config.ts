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
        .describe('Model id used when a run omits one; empty = OpenCode default'),
      cwd: z.string().default('').describe('Default working directory a turn runs in'),
      agent: z.string().default('').describe('Named OpenCode agent to run the turn as'),
    })
    .prefault({})
    .describe('Per-turn defaults applied when an opencode::run payload omits a field'),
  // Deprecated and ignored: the feeds are the fixed trigger types
  // opencode::agent-event / opencode::raw-event. Kept optional so stored
  // configs that still carry them pass the published schema
  // (additionalProperties: false) instead of being rejected.
  events_stream: z.string().optional().meta({
    deprecated: true,
    description:
      'Deprecated, ignored. AgentEvent frames are delivered on the opencode::agent-event trigger type.',
  }),
  raw_events_stream: z.string().optional().meta({
    deprecated: true,
    description:
      'Deprecated, ignored. Raw OpenCode events are delivered on the opencode::raw-event trigger type.',
  }),
  iii_context: z
    .boolean()
    .default(true)
    .describe('Prepend the iii runtime context so the agent discovers engine functions'),
  opencode_executable: z
    .string()
    .default('')
    .describe('Path to the opencode CLI; empty = resolve on PATH'),
});

export type Config = z.infer<typeof ConfigSchema>;

/**
 * The slice managed by the `configuration` worker. `engine_url` is excluded —
 * it is bootstrap (needed to reach the configuration worker), so it stays on
 * the local seed / `--url` and never hot-reloads.
 */
export const RuntimeConfigSchema = ConfigSchema.omit({ engine_url: true });
export type RuntimeConfig = z.infer<typeof RuntimeConfigSchema>;

// The registry's publish validator has no `$schema` meta-schema registered, so
// the draft-2020-12 `$schema` key z.toJSONSchema stamps at the root fails
// publish. Strip it; the schema body is what the engine + registry consume.
function stripSchema(schema: Record<string, unknown>): Record<string, unknown> {
  delete schema.$schema;
  return schema;
}

/** JSON Schema published to the configuration worker. */
export function runtimeJsonSchema(): Record<string, unknown> {
  return stripSchema(z.toJSONSchema(RuntimeConfigSchema) as Record<string, unknown>);
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
