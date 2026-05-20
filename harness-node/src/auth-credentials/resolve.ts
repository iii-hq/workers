/**
 * Pure resolution helpers. Mirrors
 * `auth-credentials/src/lib.rs::{find_env_keys, resolve_credential, status_for}`.
 */

import type { CredentialStore } from './store.js';
import {
  type AuthSource,
  type AuthStatus,
  type Credential,
  ENV_VAR_MAP,
  type EnvKeyMatch,
  envVarFor,
} from './types.js';

export type EnvGetter = (key: string) => string | undefined;

export function findEnvKeys(getter: EnvGetter): EnvKeyMatch[] {
  const out: EnvKeyMatch[] = [];
  for (const [provider, env_var] of ENV_VAR_MAP) {
    const value = getter(env_var);
    if (!value) continue;
    out.push({
      provider,
      env_var,
      key_prefix: value.slice(0, 8),
    });
  }
  return out;
}

export async function resolveCredential(
  store: CredentialStore,
  provider: string,
  getter: EnvGetter,
): Promise<{ credential: Credential; source: AuthSource } | null> {
  const stored = await store.get(provider);
  if (stored) return { credential: stored, source: 'stored' };
  const envVar = envVarFor(provider);
  if (!envVar) return null;
  const key = getter(envVar);
  if (!key) return null;
  return { credential: { type: 'api_key', key }, source: 'environment' };
}

export function statusFor(
  resolved: { credential: Credential; source: AuthSource } | null,
): AuthStatus {
  if (!resolved) return { configured: false };
  return {
    configured: true,
    source: resolved.source,
    label: labelFor(resolved.credential),
  };
}

function labelFor(cred: Credential): string {
  if (cred.type === 'api_key') return `api-key:${cred.key.slice(0, 6)}…`;
  return 'oauth';
}
