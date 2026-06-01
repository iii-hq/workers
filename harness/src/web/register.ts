/**
 * Worker entry: load config, register every `web::*` handler. Follows the
 * same per-feature register composition as the other workers.
 */

import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadWebConfig } from './config.js';
import { register as registerFetch } from './handlers/fetch.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const web = loadWebConfig(cfg);
  registerFetch(iii, web);
}
