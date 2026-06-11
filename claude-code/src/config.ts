import { readFile } from 'node:fs/promises';
import { parse } from 'yaml';
import { z } from 'zod';

const ConfigSchema = z.object({
  engine_url: z.string().default('ws://127.0.0.1:49134'),
  defaults: z
    .object({
      model: z.string().default(''),
      permission_mode: z
        .enum(['default', 'acceptEdits', 'plan', 'bypassPermissions'])
        .default('acceptEdits'),
      max_turns: z.number().int().positive().default(50),
      cwd: z.string().default(''),
      append_system_prompt: z.string().default(''),
      allowed_tools: z.array(z.string()).default([]),
      disallowed_tools: z.array(z.string()).default([]),
    })
    .prefault({}),
  approval_gate: z.boolean().default(false),
  events_stream: z.string().default('agent::events'),
  raw_events_stream: z.string().default('claude::events'),
  claude_executable: z.string().default(''),
});

export type Config = z.infer<typeof ConfigSchema>;

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
