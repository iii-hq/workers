import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { register as registerTrigger } from './trigger.js';
import { loadHarnessConfig } from './config.js';
import { spawnPumps } from './fanout/index.js';
import { register as registerFs } from './fs.js';
import { registerPolicy } from './policy/check-permissions.js';
import { loadAndWatch } from './policy/handle.js';
import { registerProviderRegistry } from './providers/register.js';
import { registerProviderRefreshOnConfig } from './providers/refresh-on-config.js';
import { FanoutState, registerSubscriptions } from './ui-subscribe.js';

export async function register(iii: ISdk, ctx: { configPath: string; url: string }): Promise<void> {
  const fanoutState = new FanoutState();

  const cfg = await loadConfig(ctx.configPath);
  const harness = loadHarnessConfig(cfg);

  registerTrigger(iii);
  registerSubscriptions(iii, fanoutState);
  spawnPumps(iii, fanoutState);
  registerFs(iii, ctx.url);

  // Provider credentials + settings + permissions now live in the
  // `configuration` worker (`harness` entry), owned by this registry.
  const registry = await registerProviderRegistry(iii);
  // Re-discover provider models whenever that entry changes so the picker
  // reflects added/removed credentials without a manual refresh.
  registerProviderRefreshOnConfig(iii, registry, fanoutState);

  const handle = await loadAndWatch(harness.permissions_path);
  registerPolicy(iii, handle);
}
