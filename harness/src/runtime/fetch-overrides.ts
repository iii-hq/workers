/**
 * Shared bridge to `provider_config::get`. Each provider's `buildConfig`
 * uses this to layer runtime overrides on top of its config.yaml defaults.
 *
 * Lives in `runtime/` rather than `provider-config/` so the dependency arrow
 * runs provider → runtime, not provider → sibling worker.
 *
 * Errors are swallowed by design: a missing or down provider-config worker
 * means "no overrides", not "fail the chat request".
 */

import type { ProviderOverrides } from '../provider-config/types.js';
import type { ISdk } from './iii.js';

const FETCH_OVERRIDES_TIMEOUT_MS = 2_000;

export async function fetchOverrides(iii: ISdk, provider: string): Promise<ProviderOverrides> {
  try {
    const result = await iii.trigger<unknown, ProviderOverrides | null>({
      function_id: 'provider_config::get',
      payload: { provider },
      timeoutMs: FETCH_OVERRIDES_TIMEOUT_MS,
    });
    return result && typeof result === 'object' ? result : {};
  } catch {
    return {};
  }
}
