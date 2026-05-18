import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadApprovalGateConfig } from './config.js';
import { handleGateEvent } from './gate-subscriber.js';
import {
  CONDITION_FN_ID as ON_DECISION_CONDITION,
  TRIGGER_FN_ID as ON_DECISION_FN,
  handleDecisionWritten,
  isDecisionWrite,
} from './on-decision-written.js';
import { handleResolveWithEvents } from './pending.js';
import { IiiStateBus } from './state-bus.js';
import { FN_RESOLVE } from './types.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = loadApprovalGateConfig(await loadConfig(ctx.configPath));
  const bus = new IiiStateBus(iii);

  iii.registerFunction(
    FN_RESOLVE,
    async (payload: unknown) =>
      handleResolveWithEvents(iii, bus, cfg.approval_state_scope, payload),
    {
      description:
        'Flip an approval to allow or deny. Writing the decision is itself the wake-up event.',
    },
  );

  iii.registerFunction(
    'policy::approval_gate',
    async (envelope: unknown) => handleGateEvent({ iii, bus, cfg }, envelope),
    {
      description: 'Consult policy::check_permissions and reply allow, deny, or pending.',
    },
  );

  iii.registerTrigger({
    type: 'durable:subscriber',
    function_id: 'policy::approval_gate',
    config: { topic: cfg.topic },
  });

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
