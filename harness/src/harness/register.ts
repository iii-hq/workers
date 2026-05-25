import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { register as registerTrigger } from './trigger.js';
import { loadHarnessConfig } from './config.js';
import { spawnPumps } from './fanout/index.js';
import { register as registerFs } from './fs.js';
import { registerPolicy } from './policy/check-permissions.js';
import { loadAndWatch } from './policy/handle.js';
import { FanoutState, registerSubscriptions } from './ui-subscribe.js';

export async function register(iii: ISdk, ctx: { configPath: string; url: string }): Promise<void> {
  const fanoutState = new FanoutState();

  const cfg = await loadConfig(ctx.configPath);
  const harness = loadHarnessConfig(cfg);

  registerTrigger(iii);
  registerSubscriptions(iii, fanoutState);
  spawnPumps(iii, fanoutState);
  registerFs(iii, ctx.url);

  const handle = await loadAndWatch(harness.permissions_path);
  registerPolicy(iii, handle);
}
