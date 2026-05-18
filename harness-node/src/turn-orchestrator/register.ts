import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { register as registerAgentCall } from './agent-call.js';
import * as bootstrap from './bootstrap.js';
import { loadOrchestratorConfig } from './config.js';
import {
  CONDITION_FN_ID as ABORT_CONDITION_FN,
  HANDLER_FN_ID as ABORT_HANDLER_FN,
  handleAbortSignalWrite,
  isAbortSignalWrite,
} from './on-abort-signal.js';
import { register as registerRunStart } from './run-start.js';
import { register as registerSubscriber } from './subscriber.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = await loadConfig(ctx.configPath);
  const orchestratorCfg = loadOrchestratorConfig(cfg);
  registerRunStart(iii, orchestratorCfg);
  registerAgentCall(iii);
  registerSubscriber(iii, orchestratorCfg);

  // Reactive abort wake. Mirrors the pattern in
  // harness/fanout/sessions-poll.ts (post c59210788c).
  iii.registerFunction(ABORT_CONDITION_FN, async (event: unknown) => isAbortSignalWrite(event), {
    description:
      'Condition: state event sets session/<id>/abort_signal = true (state:created or state:updated).',
  });

  iii.registerFunction(
    ABORT_HANDLER_FN,
    async (event: unknown) => handleAbortSignalWrite(iii, event),
    {
      description:
        'State trigger adapter on scope=agent for abort_signal writes; publishes turn::step_requested so the orchestrator picks up the abort promptly.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: ABORT_HANDLER_FN,
    config: {
      scope: 'agent',
      condition_function_id: ABORT_CONDITION_FN,
    },
  });

  // Bootstrap best-effort skill download in the background.
  void bootstrap.run(iii, orchestratorCfg);
}
