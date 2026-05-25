import type { ISdk } from '../../runtime/iii.js';
import type { OverridesStore } from '../store.js';

export function register(iii: ISdk, store: OverridesStore): void {
  iii.registerFunction(
    'provider_config::list',
    async () => {
      const entries = await store.list();
      return {
        entries: entries.map(([provider, overrides]) => ({ provider, overrides })),
      };
    },
    { description: 'List all providers with runtime overrides.' },
  );
}
