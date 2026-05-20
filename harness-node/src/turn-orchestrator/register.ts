import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { register as registerAgentTrigger } from './agent-trigger.js';
import * as bootstrap from './bootstrap.js';
import { loadOrchestratorConfig } from './config.js';
import { register as registerGetState } from './get-state.js';
import { register as registerOnAbortSignal } from './on-abort-signal.js';
import { register as registerOnRecordWritten } from './on-record-written.js';
import { register as registerOnTurnStateChanged } from './on-turn-state-changed.js';
import { register as registerRunStart } from './run-start.js';
import { recoverPendingApprovals } from './approval-resume.js';
import { register as registerSubscriber } from './subscriber.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const orchestratorCfg = loadOrchestratorConfig(cfg);
  registerRunStart(iii);
  registerAgentTrigger(iii);
  registerSubscriber(iii, orchestratorCfg);
  await recoverPendingApprovals(iii);
  registerGetState(iii);
  registerOnAbortSignal(iii);
  registerOnRecordWritten(iii);
  registerOnTurnStateChanged(iii);

  // Bootstrap best-effort skill download in the background.
  void bootstrap.run(iii, orchestratorCfg);
}
