import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import type { CredentialStore } from '../store.js';

export function register(iii: ISdk, store: CredentialStore): void {
  iii.registerFunction(
    'auth::delete_token',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = requireString(obj, 'provider');
      await store.clear(provider);
      return { ok: true };
    },
    { description: 'Remove the stored credential for a provider.' },
  );
}
