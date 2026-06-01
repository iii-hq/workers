import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { declareProvider } from '../runtime/provider-resolve.js';
import { PROVIDER_ID } from './auth.js';
import { register as registerComplete } from './complete.js';
import { loadWorkerConfig } from './config.js';
import { discoverAndRegister } from './discover.js';
import { register as registerRefresh } from './refresh-fn.js';
import { register as registerStream } from './stream-fn.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const worker = loadWorkerConfig(cfg);
  registerComplete(iii, worker);
  registerStream(iii, worker);
  registerRefresh(iii, worker);

  // Self-declare into the harness configuration schema (api key + settings).
  void declareProvider(iii, {
    id: PROVIDER_ID,
    display_name: 'anthropic',
    credential_env_var: 'ANTHROPIC_API_KEY',
    defaults: {
      api_url: worker.default_api_url,
      max_tokens: worker.default_max_tokens,
    },
    supports_model_listing: true,
  });

  // Fire-and-forget startup discovery: pull the live model list into the
  // catalog so the picker shows current models. Deferred so a slow upstream
  // (or the registry still coming up) doesn't block harness boot.
  setImmediate(() => {
    discoverAndRegister(iii, worker).catch((err) => {
      logger.warn('anthropic startup discovery threw', { err: String(err) });
    });
  });
}
