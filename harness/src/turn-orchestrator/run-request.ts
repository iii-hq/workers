/**
 * The persisted run request. Writers (`run::start`, then `provisioning`) always
 * store a complete record and state is cleared on deploy, so it's read back typed
 * (not re-parsed); `loadRunRequest` falls back to defaultRunRequest when absent.
 */

import type { Mode } from './system-prompt.js';

export type RunRequest = {
  provider: string;
  model: string;
  mode: Mode | null;
  system_prompt: string;
  function_schemas: unknown[];
  /** Optional reasoning/thinking level ('off'|'minimal'|'low'|'medium'|'high'|'xhigh'). */
  thinking_level?: string;
  /**
   * Provider resolved by `router::route` during provisioning, pinned as the
   * explicit provider on `router::chat` so preview and execution can never
   * diverge. Empty when routing failed (the router re-decides server-side).
   */
  routed_provider?: string;
};

/** Empty run request used as the absent-record fallback in `loadRunRequest`. */
export function defaultRunRequest(): RunRequest {
  return { provider: '', model: '', mode: null, system_prompt: '', function_schemas: [] };
}
