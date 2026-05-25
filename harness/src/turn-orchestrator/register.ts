import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import * as bootstrap from './bootstrap.js';
import { loadOrchestratorConfig } from './config.js';
import { register as registerAssistantStreaming } from './assistant-streaming/process.js';
import { register as registerFunctionAwaitingApproval } from './function-awaiting-approval/process.js';
import { register as registerFunctionExecute } from './function-execute/process.js';
import { register as registerGetState } from './get-state.js';
import { register as registerRunStart } from './run-start.js';
import { recoverParkedApprovals, register as registerOnApproval } from './on-approval.js';
import { register as registerProvisioning } from './provisioning/process.js';
import { register as registerSteeringCheck } from './steering-check/process.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const orchestratorCfg = loadOrchestratorConfig(cfg);
  registerRunStart(iii);
  registerProvisioning(iii, orchestratorCfg);
  registerAssistantStreaming(iii);
  registerFunctionExecute(iii);
  registerFunctionAwaitingApproval(iii);
  registerSteeringCheck(iii);
  registerGetState(iii);
  registerOnApproval(iii);
  await recoverParkedApprovals(iii);

  void bootstrap.run(iii, orchestratorCfg);
}
