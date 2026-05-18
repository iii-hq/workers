import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadApprovalGateConfig } from './config.js';
import { handleGateEvent } from './gate-subscriber.js';
import { TRIGGER_FN_ID as ON_DECISION_FN, handleDecisionWritten } from './on-decision-written.js';
import { handleResolveWithEvents } from './pending.js';
import { IiiStateBus } from './state-bus.js';
import { handleSweepSession } from './sweep.js';
import { FN_RESOLVE, FN_SWEEP_SESSION } from './types.js';

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
    FN_SWEEP_SESSION,
    async (payload: unknown) => handleSweepSession(bus, cfg.approval_state_scope, payload),
    { description: "Resolve a session's pending approvals as denied (used on abort)." },
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

  iii.registerFunction(
    ON_DECISION_FN,
    async (event: unknown) => handleDecisionWritten(iii, event),
    {
      description:
        'State trigger adapter on scope=approvals; extracts session_id from key and publishes turn::step_requested.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: ON_DECISION_FN,
    config: { scope: cfg.approval_state_scope },
  });
}
