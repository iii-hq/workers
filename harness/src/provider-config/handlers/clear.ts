import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import type { OverridesStore } from '../store.js';

export function register(iii: ISdk, store: OverridesStore): void {
  iii.registerFunction(
    'provider_config::clear',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = requireString(obj, 'provider');
      await store.clear(provider);
      return { ok: true };
    },
    { description: 'Remove runtime overrides for a provider.' },
  );
}
