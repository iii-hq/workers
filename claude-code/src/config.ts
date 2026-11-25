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
  // Deprecated and ignored: the feeds are the fixed functions
  // claude::on-agent-event / claude::on-raw-event. Kept as optional properties only
  // because the published schema is closed (additionalProperties: false), so a
  // stored configuration that still carries them keeps validating.
  events_stream: z
    .string()
    .optional()
    .meta({ deprecated: true })
    .describe(
      'Deprecated, ignored. AgentEvent frames are published on the claude::on-agent-event function.',
    ),
  raw_events_stream: z
    .string()
    .optional()
    .meta({ deprecated: true })
    .describe(
      'Deprecated, ignored. Raw Claude Code messages are published on the claude::on-raw-event function.',
    ),
  iii_context: z
    .boolean()
    .default(true)
    .describe('Prepend the iii runtime context so the agent discovers engine functions'),
  claude_executable: z
    .string()
    .default('')
    .describe('Path to the claude CLI; empty = resolve on PATH'),
  external_sessions: z
    .boolean()
    .default(false)
    .describe(
      'List all Claude Code sessions on the terminal host, allow callers to read assistant reply text from listed terminal sessions, and allow opening and stopping background sessions',
    ),
  terminal: z
    .object({
      executable: z
        .string()
        .default('')
        .describe(
          'Path to the claude binary on the TERMINAL host. Empty: resolve `claude` on PATH there, installing it first when auto_install is on. Unrelated to claude_executable, which is this worker’s own host.',
        ),
      args: z
        .array(z.string())
        .default([])
        .describe('Extra argv appended to every terminal session'),
      workspace_dir: z
        .string()
        .default('')
        .describe(
          "The worker's own folder on the terminal host (it keeps default-mcp.md there). Empty: `claude-code` under the ide worker's primary root. Not where sessions run: each runs in the folder the console page picks, else the compose project folder.",
        ),
      auto_install: z
        .boolean()
        .default(true)
        .describe('Install Claude Code from https://claude.ai/install.sh when it is missing'),
      setup_workspace: z
        .boolean()
        .default(true)
        .describe(
          'Before every session: install the published iii plugin (iii@iii, scope local) in its folder when missing, and append default-mcp',
        ),
    })
    .prefault({})
    .describe('The console terminal page: what runs when a person opens `claude` in the console'),
});

export type Config = z.infer<typeof ConfigSchema>;

/** The console terminal's slice, on its own so a test can build one. */
export const TerminalConfigSchema = ConfigSchema.shape.terminal;
export type TerminalConfig = Config['terminal'];

/**
 * The slice managed by the `configuration` worker. `engine_url` is excluded —
 * it is bootstrap (needed to reach the configuration worker), so it stays on
 * the local seed / `--url` and never hot-reloads.
 */
export const RuntimeConfigSchema = ConfigSchema.omit({ engine_url: true });
export type RuntimeConfig = z.infer<typeof RuntimeConfigSchema>;

/** JSON Schema published to the configuration worker. The registry validator
 *  has no `$schema` meta-schema, so strip the draft-2020-12 `$schema` key.
 *
 *  It describes what may be STORED (`io: 'input'`), not what a parse returns:
 *  a setting with a default is optional and carries that default. The
 *  configuration worker validates a stored value against this schema, and
 *  compose refuses to start the worker when it does not validate, so a
 *  setting a release adds must never make an older stored configuration
 *  invalid; this worker's own parse fills in the default. */
export function runtimeJsonSchema(): Record<string, unknown> {
  const out = z.toJSONSchema(RuntimeConfigSchema, { io: 'input' }) as Record<string, unknown>;
  // Input mode leaves objects open; the stored value stays closed, as it was,
  // so a misspelt setting is still refused rather than silently ignored.
  closeLike(out, z.toJSONSchema(RuntimeConfigSchema) as Record<string, unknown>);
  delete out.$schema;
  return out;
}

type SchemaNode = { properties?: Record<string, SchemaNode>; additionalProperties?: unknown };

/** Copy `additionalProperties` from `like` onto the matching objects of `node`. */
function closeLike(node: SchemaNode, like: SchemaNode): void {
  if ('additionalProperties' in like && node.properties) {
    node.additionalProperties = like.additionalProperties;
  }
  for (const [key, child] of Object.entries(node.properties ?? {})) {
    const other = like.properties?.[key];
    if (other) closeLike(child, other);
  }
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
