import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadApprovalGateConfig } from './config.js';
import {
  CONDITION_FN_ID as ON_DECISION_CONDITION,
  TRIGGER_FN_ID as ON_DECISION_FN,
  handleDecisionWritten,
  isDecisionWrite,
} from './on-decision-written.js';
import { handleResolveWithEvents } from './pending.js';
import { FN_RESOLVE } from './types.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = loadApprovalGateConfig(await loadConfig(ctx.configPath));

  iii.registerFunction(
    FN_RESOLVE,
    async (payload: unknown) =>
      handleResolveWithEvents(iii, cfg.approval_state_scope, payload),
    {
      description:
        'Flip an approval to allow or deny. Writing the decision is itself the wake-up event.',
    },
  );

  iii.registerFunction(ON_DECISION_CONDITION, async (event: unknown) => isDecisionWrite(event), {
    description:
      'Condition: state event is a real approval decision write (state:created or state:updated, new_value.decision present).',
  });

  iii.registerFunction(
    ON_DECISION_FN,
    async (event: unknown) => handleDecisionWritten(iii, event),
    {
      description:
        'State trigger adapter on scope=approvals; extracts session_id from key and invokes turn::step directly.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: ON_DECISION_FN,
    config: {
      scope: cfg.approval_state_scope,
      condition_function_id: ON_DECISION_CONDITION,
    },
  });
}
