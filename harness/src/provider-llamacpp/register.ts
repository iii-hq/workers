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
import { register as registerRefresh } from './refresh-fn.js';
import { register as registerStream } from './stream-fn.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const worker = loadWorkerConfig(cfg);
  registerStream(iii, worker);
  registerRefresh(iii, worker);

  // Self-declare into the llm-router registry. api_url is env-driven
  // (LLAMACPP_BASE_URL); only max_tokens is seeded. Retries until the
  // router is reachable; re-declares on `router::ready`.
  const decl: ProviderDeclaration = {
    id: PROVIDER_ID,
    display_name: 'llama.cpp',
    credential_env_var: 'LLAMACPP_API_KEY',
    defaults: { max_tokens: worker.default_max_tokens },
    supports_model_listing: true,
  };
  void registerWithRouter(iii, decl);
  subscribeRouterReady(iii, PROVIDER_ID, () => void registerWithRouter(iii, decl));

  // Fire-and-forget startup discovery: probe llama-server's /v1/models
  // and register the (single) loaded model so the picker shows its real
  // id instead of just the placeholder. Wrapped in setImmediate so a
  // slow/unreachable host doesn't block the rest of the harness from
  // coming up — the router may also still be registering when this runs.
  //
  // Note: llama-server CANNOT load/unload models at runtime, so this
  // provider does NOT register `provider::llamacpp::load_model` /
  // `unload_model`. To run a different model, restart `llama-server`
  // with a different `-m`.
  setImmediate(() => {
    runStartupDiscovery(iii, worker.default_api_url).catch((err) => {
      logger.warn('llamacpp startup discovery threw', { err: String(err) });
    });
  });
}

async function runStartupDiscovery(iii: ISdk, chatUrl: string): Promise<void> {
  try {
    const headers = await buildAuthHeaders(iii, chatUrl);
    await discoverAndRegister(iii, chatUrl, headers);
  } catch (err) {
    logger.warn('llamacpp startup discovery: header build failed', {
      err: String(err),
    });
  }
}
