import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import {
  type ProviderDeclaration,
  registerWithRouter,
  subscribeRouterReady,
} from '../runtime/provider-resolve.js';
import { buildAuthHeaders, PROVIDER_ID } from './auth.js';
import { loadWorkerConfig } from './config.js';
import { discoverAndRegister } from './discover.js';
import { register as registerLoad } from './load-fn.js';
import { register as registerRefresh } from './refresh-fn.js';
import { register as registerStream } from './stream-fn.js';
import { register as registerUnload } from './unload-fn.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const worker = loadWorkerConfig(cfg);
  registerStream(iii, worker);
  registerRefresh(iii, worker);
  registerLoad(iii, worker);
  registerUnload(iii, worker);

  // Self-declare into the llm-router registry. The api_url is env-driven
  // (LMSTUDIO_BASE_URL) so we don't pin it as a stored default; only
  // max_tokens is seeded for the form. Retries until the router is
  // reachable; re-declares on `router::ready`.
  const decl: ProviderDeclaration = {
    id: PROVIDER_ID,
    display_name: 'lm studio',
    credential_env_var: 'LMSTUDIO_API_KEY',
    defaults: { max_tokens: worker.default_max_tokens },
    supports_model_listing: true,
  };
  void registerWithRouter(iii, decl);
  subscribeRouterReady(iii, PROVIDER_ID, () => void registerWithRouter(iii, decl));

  // Fire-and-forget startup discovery: probe LM Studio's /api/v0/models
  // and register each currently-loaded LLM so the picker shows real model
  // IDs. Wrapped in setImmediate so a slow/unreachable LM Studio host
  // doesn't block the rest of the harness from coming up — the router may
  // also still be registering when this runs.
  setImmediate(() => {
    runStartupDiscovery(iii, worker.default_api_url).catch((err) => {
      logger.warn('lmstudio startup discovery threw', { err: String(err) });
    });
  });
}

async function runStartupDiscovery(iii: ISdk, chatUrl: string): Promise<void> {
  try {
    const headers = await buildAuthHeaders(iii, chatUrl);
    await discoverAndRegister(iii, chatUrl, headers);
  } catch (err) {
    // discoverAndRegister already logs its own failures; this catch
    // covers the buildAuthHeaders call.
    logger.warn('lmstudio startup discovery: header build failed', {
      err: String(err),
    });
  }
}
