import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadApprovalGateConfig } from './config.js';
import { handleGateEvent } from './gate-subscriber.js';
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
        'Flip an approval to allow or deny, emit approval_resolved, and wake the paused session via turn::step_requested.',
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
}
