import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { register as registerAgentTrigger } from './agent-trigger.js';
import * as bootstrap from './bootstrap.js';
import { loadOrchestratorConfig } from './config.js';
import { register as registerGetState } from './get-state.js';
import { register as registerOnAbortSignal } from './on-abort-signal.js';
import { register as registerRunStart } from './run-start.js';
import { recoverPendingApprovals } from './approval-resume.js';
import {
  registerAssistantFinished,
  registerAssistantStreaming,
  registerFunctionAwaitingApproval,
  registerFunctionExecute,
  registerProvisioning,
  registerSteeringCheck,
  registerTearingDown,
} from './states/index.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const orchestratorCfg = loadOrchestratorConfig(cfg);
  registerRunStart(iii);
  registerAgentTrigger(iii);
  registerProvisioning(iii, orchestratorCfg);
  registerAssistantStreaming(iii);
  registerAssistantFinished(iii);
  registerFunctionExecute(iii);
  registerFunctionAwaitingApproval(iii);
  registerSteeringCheck(iii);
  registerTearingDown(iii);
  await recoverPendingApprovals(iii);
  registerGetState(iii);
  registerOnAbortSignal(iii);

  void bootstrap.run(iii, orchestratorCfg);
}
