import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadHarnessConfig } from './config.js';
import { spawnPumps } from './fanout/index.js';
import { register as registerFs } from './fs.js';
import { register as registerPolicyFn } from './policy-fn.js';
import { loadAndWatch } from './policy.js';
import { register as registerStatus } from './status.js';
import { FanoutState, registerSubscriptions } from './ui-subscribe.js';

export async function register(iii: ISdk, ctx: { configPath: string; url: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const harness = loadHarnessConfig(cfg);
  registerStatus(iii);
  const fanoutState = new FanoutState();
  registerSubscriptions(iii, fanoutState);
  spawnPumps(iii, fanoutState);
  registerFs(iii, ctx.url);
  const handle = await loadAndWatch(harness.permissions_path);
  registerPolicyFn(iii, handle);
}
