/**
 * The persisted run request and its single typed parser. `loadRunRequest`
 * (persistence) parses the raw `session/<sid>/run_request` value through
 * `parseRunRequest` once, so every consumer reads a fully-typed `RunRequest`
 * instead of re-guarding `unknown` fields.
 */

import { z } from 'zod';
import type { Mode } from './system-prompt.js';

export const RunRequestSchema = z.object({
  provider: z.string().catch(''),
  model: z.string().catch(''),
  mode: z
    .unknown()
    .transform((v): Mode | null =>
      v === 'plan' || v === 'ask' || v === 'agent' ? v : null,
    ),
  system_prompt: z.string().catch(''),
  function_schemas: z.array(z.unknown()).catch([]),
});

export type RunRequest = z.infer<typeof RunRequestSchema>;

export function parseRunRequest(raw: unknown): RunRequest {
  return RunRequestSchema.parse(raw ?? {});
}
