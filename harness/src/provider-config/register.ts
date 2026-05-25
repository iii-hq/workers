import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadProviderConfigConfig } from './config.js';
import { register as registerClear } from './handlers/clear.js';
import { register as registerGet } from './handlers/get.js';
import { register as registerList } from './handlers/list.js';
import { register as registerSet } from './handlers/set.js';
import { DbOverridesStore } from './store.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const providerCfg = loadProviderConfigConfig(cfg);
  const store = new DbOverridesStore(iii, { databaseName: providerCfg.database_name });
  await store.init();
  registerGet(iii, store);
  registerSet(iii, store);
  registerClear(iii, store);
  registerList(iii, store);
}
