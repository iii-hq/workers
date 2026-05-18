import type { ISdk } from '../../runtime/iii.js';
import type { CredentialStore } from '../store.js';

export function register(iii: ISdk, store: CredentialStore): void {
  iii.registerFunction(
    'auth::list_providers',
    async () => {
      const entries = await store.list();
      return { providers: entries.map(([p]) => p) };
    },
    { description: 'List every provider with a stored credential.' },
  );
}
