import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { register as registerAgentCall } from './agent-call.js';
import * as bootstrap from './bootstrap.js';
import { loadOrchestratorConfig } from './config.js';
import { register as registerRunStart } from './run-start.js';
import { register as registerSubscriber } from './subscriber.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const orchestratorCfg = loadOrchestratorConfig(cfg);
  registerRunStart(iii, orchestratorCfg);
  registerAgentCall(iii);
  registerSubscriber(iii, orchestratorCfg);
  // Bootstrap best-effort skill download in the background.
  void bootstrap.run(iii, orchestratorCfg);
}
