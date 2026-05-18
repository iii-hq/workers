import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import type { CredentialStore } from '../store.js';
import type { Credential } from '../types.js';

export function register(iii: ISdk, store: CredentialStore): void {
  iii.registerFunction(
    'auth::set_token',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = requireString(obj, 'provider');
      const cred = obj.credential;
      if (!cred || typeof cred !== 'object') {
        throw new Error('missing required field: credential');
      }
      const c = cred as Record<string, unknown>;
      if (c.type !== 'api_key' && c.type !== 'oauth') {
        throw new Error(`invalid credential.type: ${String(c.type)}`);
      }
      await store.set(provider, c as unknown as Credential);
      return { ok: true };
    },
    { description: 'Persist a credential for a provider.' },
  );
}
