import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadHookFanoutConfig } from './config.js';
import { register as registerPublishCollect } from './publish-collect.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const fanoutCfg = loadHookFanoutConfig(cfg);
  registerPublishCollect(iii, fanoutCfg);
}
