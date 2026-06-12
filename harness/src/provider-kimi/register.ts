import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import {
  type ProviderDeclaration,
  registerWithRouter,
  subscribeRouterReady,
} from '../runtime/provider-resolve.js';
import { PROVIDER_ID } from './auth.js';
import { loadWorkerConfig } from './config.js';
import { discoverAndRegister } from './discover.js';
import { register as registerRefresh } from './refresh-fn.js';
import { register as registerStream } from './stream-fn.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const worker = loadWorkerConfig(cfg);
  registerStream(iii, worker);
  registerRefresh(iii, worker);

  // Self-declare into the llm-router registry (api key + settings schema).
  // Retries until the router is reachable; re-declares on `router::ready`.
  const decl: ProviderDeclaration = {
    id: PROVIDER_ID,
    display_name: 'kimi (moonshot)',
    credential_env_var: 'MOONSHOT_API_KEY',
    defaults: {
      api_url: worker.default_api_url,
      max_tokens: worker.default_max_tokens,
    },
    supports_model_listing: true,
  };
  void registerWithRouter(iii, decl);
  subscribeRouterReady(iii, PROVIDER_ID, () => void registerWithRouter(iii, decl));

  setImmediate(() => {
    discoverAndRegister(iii, worker).catch((err) => {
      logger.warn('kimi startup discovery threw', { err: String(err) });
    });
  });
}
