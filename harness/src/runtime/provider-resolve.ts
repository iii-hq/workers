/**
 * Provider-side helpers for the llm-router provider protocol:
 *
 *   - `registerWithRouter` declares a provider (id, defaults, env var) into
 *     the router registry at startup, persists the registration token, and
 *     retries with capped backoff until the router is reachable.
 *   - `subscribeRouterReady` re-declares when the router (re)boots — the
 *     router publishes `router::ready` at the end of every boot.
 *   - `resolveProviderViaRouter` fetches the credential + settings (api_url,
 *     max_tokens) at request time via the token-gated
 *     `router::provider::resolve`.
 *   - `routerRegistrationToken` exposes the token to sibling modules
 *     (model discovery reconciles through the same gate).
 *
 * Token lifecycle: the router mints a bearer token on first registration and
 * hard-rejects later registrations that don't present it, so the token is
 * persisted in iii-state (scope `llm-provider-registration`) and reloaded on
 * boot. A presented token is adopted by a fresh router registry, so the
 * persisted copy survives router-state wipes too. The one unrecoverable case
 * — harness state lost while the router registry survives — needs an operator
 * to clear the router's `llm-router`/`registry` state key.
 *
 * This module also owns the `Credential` shape returned by
 * `router::provider::resolve` (the resolved api key / oauth token).
 */

import type { ISdk } from './iii.js';
import { logger } from './otel.js';
import { stateGet, stateSet } from './state.js';

export type ApiKeyCredential = {
  type: 'api_key';
  key: string;
};

export type OAuthCredential = {
  type: 'oauth';
  access_token: string;
  refresh_token?: string;
  expires_at?: number;
  scopes?: string[];
  provider_extra?: unknown;
};

export type Credential = ApiKeyCredential | OAuthCredential;

export type ProviderDeclaration = {
  id: string;
  display_name?: string;
  /** Env var consulted as a credential fallback when no `api_key` is configured. */
  credential_env_var?: string;
  /** Optional explicit JSON Schema; when omitted the router derives one from `defaults`. */
  config_schema?: Record<string, unknown>;
  defaults?: { api_url?: string; max_tokens?: number } & Record<string, unknown>;
  supports_model_listing?: boolean;
};

export type ProviderResolveResult = {
  configured: boolean;
  source: 'config' | 'env' | 'none';
  credential: Credential | null;
  api_url: string | null;
  max_tokens: number | null;
};

type RouterRegisterResponse = {
  ok?: boolean;
  id?: string;
  registration_token?: string;
};

/** iii-state scope holding `{ token }` per provider id. */
const TOKEN_SCOPE = 'llm-provider-registration';

const REGISTER_TIMEOUT_MS = 5_000;
const RESOLVE_TIMEOUT_MS = 5_000;
/** How long a resolve waits for boot-time registration to land a token. */
const TOKEN_WAIT_MS = 30_000;
const RETRY_BASE_MS = 250;
const RETRY_CAP_MS = 5_000;

const tokens = new Map<string, string>();
const tokenWaiters = new Map<string, Array<(token: string) => void>>();
const registrationInFlight = new Set<string>();

function setToken(provider: string, token: string): void {
  tokens.set(provider, token);
  const waiters = tokenWaiters.get(provider) ?? [];
  tokenWaiters.delete(provider);
  for (const notify of waiters) notify(token);
}

/**
 * The provider's registration token, waiting up to `waitMs` for the boot-time
 * registration loop to land one. Sibling modules (discovery reconcile) gate
 * their router writes on this.
 */
export function routerRegistrationToken(
  provider: string,
  waitMs: number = TOKEN_WAIT_MS,
): Promise<string> {
  const existing = tokens.get(provider);
  if (existing !== undefined) return Promise.resolve(existing);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(
        new Error(
          `no router registration token for provider \`${provider}\` after ${waitMs}ms ` +
            '(llm-router down or registration still retrying)',
        ),
      );
    }, waitMs);
    const waiters = tokenWaiters.get(provider) ?? [];
    waiters.push((token) => {
      clearTimeout(timer);
      resolve(token);
    });
    tokenWaiters.set(provider, waiters);
  });
}

function isRegistrationRejected(err: unknown): boolean {
  return /registration_rejected|registration token mismatch|bound to another worker/i.test(
    String(err),
  );
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Declare the provider into the llm-router registry, retrying with capped
 * backoff until it lands (the provider is useless to the router until
 * registered, and the loop is idle-cheap). Resolves the module-level token
 * on success. A token rejection is terminal — retrying cannot fix it.
 */
export async function registerWithRouter(iii: ISdk, decl: ProviderDeclaration): Promise<void> {
  if (registrationInFlight.has(decl.id)) return;
  registrationInFlight.add(decl.id);
  try {
    const persisted = await stateGet<{ token?: string }>(iii, TOKEN_SCOPE, decl.id);
    const token =
      tokens.get(decl.id) ?? (typeof persisted?.token === 'string' ? persisted.token : undefined);
    let delayMs = RETRY_BASE_MS;
    for (;;) {
      try {
        const res = await iii.trigger<unknown, RouterRegisterResponse>({
          function_id: 'router::provider::register',
          payload: token !== undefined ? { ...decl, token } : { ...decl },
          timeoutMs: REGISTER_TIMEOUT_MS,
        });
        const granted =
          typeof res?.registration_token === 'string' && res.registration_token.length > 0
            ? res.registration_token
            : null;
        if (granted) {
          if (granted !== token) {
            await stateSet(iii, TOKEN_SCOPE, decl.id, { token: granted });
          }
          setToken(decl.id, granted);
          logger.info('provider registered with llm-router', { provider: decl.id });
          return;
        }
        logger.warn('router registration returned no token; retrying', { provider: decl.id });
      } catch (err) {
        if (isRegistrationRejected(err)) {
          logger.error(
            'router rejected the registration token; operator action required ' +
              '(clear the llm-router `registry` state key to re-bind)',
            { provider: decl.id, err: String(err) },
          );
          return;
        }
        logger.warn('router registration failed; retrying', {
          provider: decl.id,
          err: String(err),
        });
      }
      await sleep(delayMs);
      delayMs = Math.min(delayMs * 2, RETRY_CAP_MS);
    }
  } finally {
    registrationInFlight.delete(decl.id);
  }
}

/**
 * Re-declare when the router boots. The router publishes `router::ready` at
 * the end of every boot; topology events are NOT a substitute (engine worker
 * UUIDs cannot be mapped to provider ids).
 */
export function subscribeRouterReady(iii: ISdk, providerId: string, redeclare: () => void): void {
  const fnId = `provider::${providerId}::on_router_ready`;
  try {
    iii.registerFunction(fnId, async () => {
      redeclare();
      return null;
    });
    iii.registerTrigger({
      type: 'subscribe',
      function_id: fnId,
      config: { topic: 'router::ready' },
    });
  } catch (err) {
    logger.warn('could not bind router::ready re-declare trigger', {
      provider: providerId,
      err: String(err),
    });
  }
}

export async function resolveProviderViaRouter(
  iii: ISdk,
  provider: string,
): Promise<ProviderResolveResult> {
  const token = await routerRegistrationToken(provider);
  const res = await iii.trigger<unknown, Partial<ProviderResolveResult>>({
    function_id: 'router::provider::resolve',
    payload: { id: provider, token },
    timeoutMs: RESOLVE_TIMEOUT_MS,
  });
  if (!res || typeof res !== 'object') {
    return { configured: false, source: 'none', credential: null, api_url: null, max_tokens: null };
  }
  return {
    configured: res.configured === true,
    source: res.source ?? 'none',
    credential: res.credential ?? null,
    api_url: res.api_url ?? null,
    max_tokens: res.max_tokens ?? null,
  };
}

/** Test seam: clear tokens and in-flight registration guards between cases. */
export function _resetRouterRegistrationForTests(): void {
  tokens.clear();
  tokenWaiters.clear();
  registrationInFlight.clear();
}

/** Test seam: seed a registration token without a register round-trip. */
export function _seedRouterRegistrationTokenForTests(provider: string, token: string): void {
  setToken(provider, token);
}
