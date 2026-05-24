/**
 * The persisted run request and its single typed parser. `loadRunRequest`
 * (persistence) parses the raw `session/<sid>/run_request` value through
 * `parseRunRequest` once, so every consumer reads a fully-typed `RunRequest`
 * instead of re-guarding `unknown` fields.
 */

import type { Mode } from './system-prompt.js';

export type RunRequest = {
  provider: string;
  model: string;
  mode: Mode | null;
  system_prompt: string;
  function_schemas: unknown[];
};

function parseMode(value: unknown): Mode | null {
  return value === 'plan' || value === 'ask' || value === 'agent' ? value : null;
}

export function parseRunRequest(raw: Record<string, unknown>): RunRequest {
  return {
    provider: typeof raw.provider === 'string' ? raw.provider : '',
    model: typeof raw.model === 'string' ? raw.model : '',
    mode: parseMode(raw.mode),
    system_prompt: typeof raw.system_prompt === 'string' ? raw.system_prompt : '',
    function_schemas: Array.isArray(raw.function_schemas) ? raw.function_schemas : [],
  };
}
