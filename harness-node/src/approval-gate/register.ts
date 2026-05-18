import { loadConfig } from '../runtime/config.js';
import type { ISdk } from '../runtime/iii.js';
import { loadApprovalGateConfig } from './config.js';
import { handleGateEvent } from './gate-subscriber.js';
import { handleListPending, handleResolve } from './pending.js';
import { IiiStateBus } from './state-bus.js';
import { FN_LIST_PENDING, FN_RESOLVE } from './types.js';

export async function register(iii: ISdk, ctx: { configPath: string }): Promise<void> {
  const cfg = loadApprovalGateConfig(await loadConfig(ctx.configPath));
  const bus = new IiiStateBus(iii);

  iii.registerFunction(
    FN_RESOLVE,
    async (payload: unknown) => handleResolve(bus, cfg.approval_state_scope, payload),
    { description: 'Flip a pending approval entry to allow or deny.' },
  );

  iii.registerFunction(
    FN_LIST_PENDING,
    async (payload: unknown) => handleListPending(bus, cfg.approval_state_scope, payload),
    { description: 'Return pending approvals for a session.' },
  );

  iii.registerFunction(
    'policy::approval_gate',
    async (envelope: unknown) => handleGateEvent({ iii, bus, cfg }, envelope),
    {
      description:
        'Consult policy::check_permissions and either allow, deny, or pause for user resolution via approval::resolve.',
    },
  );

  iii.registerTrigger({
    type: 'durable:subscriber',
    function_id: 'policy::approval_gate',
    config: { topic: cfg.topic },
  });
}
