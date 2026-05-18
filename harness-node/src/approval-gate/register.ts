import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadApprovalGateConfig } from './config.js';
import { handleConsume } from './consume.js';
import { handleGateEvent } from './gate-subscriber.js';
import { handleListPending, handleResolveWithEvents } from './pending.js';
import { IiiStateBus } from './state-bus.js';
import { handleSweepSession } from './sweep.js';
import { FN_CONSUME, FN_LIST_PENDING, FN_RESOLVE, FN_SWEEP_SESSION } from './types.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = loadApprovalGateConfig(await loadConfig(ctx.configPath));
  const bus = new IiiStateBus(iii);

  iii.registerFunction(
    FN_RESOLVE,
    async (payload: unknown) =>
      handleResolveWithEvents(iii, bus, cfg.approval_state_scope, payload),
    {
      description:
        'Flip a pending approval entry to allow or deny, emit approval_resolved, and wake the paused session via run::resume.',
    },
  );

  iii.registerFunction(
    FN_LIST_PENDING,
    async (payload: unknown) => handleListPending(bus, cfg.approval_state_scope, payload),
    { description: 'Return pending approvals for a session.' },
  );

  iii.registerFunction(
    FN_CONSUME,
    async (payload: unknown) => handleConsume(bus, cfg.approval_state_scope, payload),
    { description: 'Return resolved approval decisions for a session once.' },
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
      description:
        'Consult policy::check_permissions and either allow, deny, or return a pending approval reply.',
    },
  );

  iii.registerTrigger({
    type: 'durable:subscriber',
    function_id: 'policy::approval_gate',
    config: { topic: cfg.topic },
  });
}
