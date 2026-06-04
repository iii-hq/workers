import type { ISdk } from '../runtime/iii.js';
import { register as registerAssistantStreaming } from './assistant-streaming/process.js';
import { register as registerFunctionAwaitingApproval } from './function-awaiting-approval/process.js';
import { register as registerFunctionExecute } from './function-execute/process.js';
import { register as registerGetState } from './get-state.js';
import { register as registerRunStart } from './run-start.js';
import { register as registerProvisioning } from './provisioning/process.js';
import { register as registerSteeringCheck } from './steering-check/process.js';

export async function register(iii: ISdk, _ctx: { configPath: string }): Promise<void> {
  registerRunStart(iii);
  registerProvisioning(iii);
  registerAssistantStreaming(iii);
  registerFunctionExecute(iii);
  registerFunctionAwaitingApproval(iii);
  registerSteeringCheck(iii);
  registerGetState(iii);
}
