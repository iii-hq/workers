import type { ISdk } from '../runtime/iii.js';
import type { TurnOrchestratorConfig } from './config.js';
import type { TurnStateRecord } from './state.js';
import { handleAwaiting, handleFinished, handleStreaming } from './states/assistant.js';
import {
  handleAwaitingApproval,
  handleExecute,
  handleFinalize,
  handlePrepare,
} from './states/functions.js';
import { handleProvisioning } from './states/provisioning.js';
import { handleSteering } from './states/steering.js';
import { handleTearingDown } from './states/tearing-down.js';

export async function step(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  rec: TurnStateRecord,
): Promise<void> {
  switch (rec.state) {
    case 'provisioning':
      return handleProvisioning(iii, cfg, rec);
    case 'awaiting_assistant':
      return handleAwaiting(iii, rec);
    case 'assistant_streaming':
      return handleStreaming(iii, rec);
    case 'assistant_finished':
      return handleFinished(iii, rec);
    case 'function_prepare':
      return handlePrepare(iii, rec);
    case 'function_execute':
      return handleExecute(iii, cfg, rec);
    case 'function_awaiting_approval':
      return handleAwaitingApproval(iii, rec);
    case 'function_finalize':
      return handleFinalize(iii, rec);
    case 'steering_check':
      return handleSteering(iii, rec);
    case 'tearing_down':
      return handleTearingDown(iii, rec);
    case 'stopped':
      return; // idempotent terminal
  }
}
