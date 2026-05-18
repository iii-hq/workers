import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { resolveCredential } from '../resolve.js';
import type { CredentialStore } from '../store.js';

export function register(iii: ISdk, store: CredentialStore): void {
  iii.registerFunction(
    'auth::get_token',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = requireString(obj, 'provider');
      const resolved = await resolveCredential(store, provider, (k) => process.env[k]);
      return resolved?.credential ?? null;
    },
    { description: 'Fetch the stored credential for a provider.' },
  );
}
