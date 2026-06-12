import { readFile } from 'node:fs/promises';
import { parse } from 'yaml';
import { z } from 'zod';

const ConfigSchema = z.object({
  engine_url: z.string().default('ws://127.0.0.1:49134'),
  defaults: z
    .object({
      model: z.string().default(''),
      sandbox_mode: z
        .enum(['read-only', 'workspace-write', 'danger-full-access'])
        .default('workspace-write'),
      approval_policy: z.enum(['never', 'on-request', 'on-failure', 'untrusted']).default('never'),
      reasoning_effort: z.enum(['', 'minimal', 'low', 'medium', 'high', 'xhigh']).default(''),
      cwd: z.string().default(''),
      skip_git_repo_check: z.boolean().default(true),
    })
    .prefault({}),
  events_stream: z.string().default('agent::events'),
  raw_events_stream: z.string().default('codex::events'),
  iii_context: z.boolean().default(true),
  codex_executable: z.string().default(''),
  base_url: z.string().default(''),
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
