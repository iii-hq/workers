import type { ISdk } from '../runtime/iii.js';
import { register as registerAssistantStreaming } from './assistant-streaming/process.js';
import { register as registerFinishing } from './finishing.js';
import { register as registerFunctionAwaitingApproval } from './function-awaiting-approval/process.js';
import { register as registerFunctionExecute } from './function-execute/process.js';
import { register as registerFunctionResolve } from './function-resolve.js';
import { register as registerGetState } from './get-state.js';
import { registerPreTriggerType } from './hooks/registry.js';
import { register as registerRunAbort } from './run-abort.js';
import { register as registerRunStart } from './run-start.js';
import { register as registerProvisioning } from './provisioning/process.js';
import { registerTurnCompletedTriggerType } from './turn-completed.js';

export async function register(iii: ISdk, _ctx: { configPath: string }): Promise<void> {
  // Trigger types first: sibling workers (the approval-gate) bind to them
  // as soon as they appear, and the FSM steps below consult/emit them.
  registerPreTriggerType(iii);
  registerTurnCompletedTriggerType(iii);

  registerRunStart(iii);
  registerRunAbort(iii);
  registerProvisioning(iii);
  registerFunctionExecute(iii);
  registerFunctionAwaitingApproval(iii);
  registerFunctionResolve(iii);
  registerAssistantStreaming(iii);
  registerFinishing(iii);
  registerGetState(iii);
}
