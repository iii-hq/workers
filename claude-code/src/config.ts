import { readFile } from 'node:fs/promises';
import { parse } from 'yaml';
import { z } from 'zod';

const ConfigSchema = z.object({
  engine_url: z.string().default('ws://127.0.0.1:49134'),
  defaults: z
    .object({
      model: z.string().default('').describe('Model id or alias; empty = Claude Code default'),
      permission_mode: z
        .enum(['default', 'acceptEdits', 'plan', 'bypassPermissions'])
        .default('acceptEdits')
        .describe('Claude Code permission mode for headless turns'),
      max_turns: z
        .number()
        .int()
        .positive()
        .default(50)
        .describe('Max agent turns before the run stops'),
      cwd: z.string().default('').describe('Default working directory a turn runs in'),
      append_system_prompt: z
        .string()
        .default('')
        .describe('Text appended to the Claude Code system prompt'),
      allowed_tools: z
        .array(z.string())
        .default([])
        .describe('Tools Claude Code may use (empty = its default set)'),
      disallowed_tools: z.array(z.string()).default([]).describe('Tools Claude Code may never use'),
    })
    .prefault({})
    .describe('Per-turn defaults applied when a claude::run payload omits a field'),
  approval_gate: z
    .boolean()
    .default(false)
    .describe('Route tool calls through the approval-gate worker'),
  events_stream: z
    .string()
    .default('agent::events')
    .describe('Stream carrying the translated AgentEvent frames'),
  raw_events_stream: z
    .string()
    .default('claude::events')
    .describe('Stream carrying the raw Claude Code messages, verbatim'),
  iii_context: z
    .boolean()
    .default(true)
    .describe('Prepend the iii runtime context so the agent discovers engine functions'),
  claude_executable: z
    .string()
    .default('')
    .describe('Path to the claude CLI; empty = resolve on PATH'),
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
