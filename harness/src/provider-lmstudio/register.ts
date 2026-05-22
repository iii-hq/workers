import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { buildAuthHeaders } from './auth.js';
import { register as registerComplete } from './complete.js';
import { loadWorkerConfig } from './config.js';
import { discoverAndRegister } from './discover.js';
import { register as registerLoad } from './load-fn.js';
import { register as registerRefresh } from './refresh-fn.js';
import { register as registerStream } from './stream-fn.js';
import { register as registerUnload } from './unload-fn.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const worker = loadWorkerConfig(cfg);
  registerComplete(iii, worker);
  registerStream(iii, worker);
  registerRefresh(iii, worker);
  registerLoad(iii, worker);
  registerUnload(iii, worker);

  // Fire-and-forget startup discovery: probe LM Studio's /api/v0/models
  // and register each currently-loaded LLM so the picker shows real model
  // IDs. Wrapped in setImmediate so a slow/unreachable LM Studio host
  // doesn't block the rest of the harness from coming up — the auth and
  // models-catalog workers may also still be registering when this runs.
  setImmediate(() => {
    runStartupDiscovery(iii, worker.default_api_url).catch((err) => {
      logger.warn('lmstudio startup discovery threw', { err: String(err) });
    });
  });
}

async function runStartupDiscovery(iii: ISdk, chatUrl: string): Promise<void> {
  try {
    const headers = await buildAuthHeaders(iii);
    await discoverAndRegister(iii, chatUrl, headers);
  } catch (err) {
    // discoverAndRegister already logs its own failures; this catch
    // covers the buildAuthHeaders call.
    logger.warn('lmstudio startup discovery: header build failed', {
      err: String(err),
    });
  }
}
