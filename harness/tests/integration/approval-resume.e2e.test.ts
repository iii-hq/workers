import { describe, expect, it, vi } from 'vitest';
import { handleResolveRequest } from '../../src/approval-gate/resolve.js';
import {
  handleApprovalDecisionWrite,
  isApprovalDecisionWrite,
} from '../../src/turn-orchestrator/on-approval.js';
import type { ISdk } from '../../src/runtime/iii.js';
import { TURN_STATE_SCOPE } from '../../src/turn-orchestrator/state.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

async function flushMicrotasks(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

/**
 * Fake iii where `state::set` re-emits a state event and feeds it to the
 * approval reactive trigger on the `approvals` scope — exercising the
 * producer → trigger → wake path.
 */
function fakeIii(): {
  iii: ISdk;
  wakeTriggers: Array<{ session_id: string; function_id: string }>;
  stateStore: Map<string, unknown>;
} {
  const stateStore = new Map<string, unknown>();
  const wakeTriggers: Array<{ session_id: string; function_id: string }> = [];

  const iii = {
    trigger: vi.fn(
      async ({
        function_id,
        payload,
        action,
      }: {
        function_id: string;
        payload: unknown;
        action?: unknown;
      }) => {
        if (function_id === 'state::set') {
          const p = payload as { scope: string; key: string; value: unknown };
          const fullKey = `${p.scope}/${p.key}`;
          const old_value = stateStore.get(fullKey) ?? null;
          stateStore.set(fullKey, p.value);
          const event = {
            event_type: old_value == null ? 'state:created' : 'state:updated',
            scope: p.scope,
            key: p.key,
            old_value,
            new_value: p.value,
            message_type: 'state',
          };
          if (p.scope === 'approvals' && isApprovalDecisionWrite(event)) {
            await handleApprovalDecisionWrite(iii as unknown as ISdk, event);
          }
          return null;
        }

        if (function_id === 'state::get') {
          const p = payload as { scope: string; key: string };
          return stateStore.get(`${p.scope}/${p.key}`) ?? null;
        }

        if (function_id.startsWith('turn::') && action != null) {
          const p = payload as { session_id: string };
          wakeTriggers.push({ session_id: p.session_id, function_id });
          return null;
        }

        return null;
      },
    ),
  };

  return { iii: iii as unknown as ISdk, wakeTriggers, stateStore };
}

describe('approval reactive trigger', () => {
  it('approval::resolve persists the decision and the trigger enqueues turn::{state}', async () => {
    const { iii, wakeTriggers, stateStore } = fakeIii();
    const rec = newRecord('sess-x');
    rec.state = 'function_awaiting_approval';
    stateStore.set(`${TURN_STATE_SCOPE}/sess-x`, rec);

    const out = await handleResolveRequest(iii, {
      session_id: 'sess-x',
      function_call_id: 'fc-1',
      decision: 'allow',
    });
    expect(out).toEqual({ ok: true });

    await flushMicrotasks();

    expect(stateStore.get('approvals/sess-x/fc-1')).toEqual({ decision: 'allow', reason: null });
    expect(wakeTriggers).toHaveLength(1);
    expect(wakeTriggers[0]).toMatchObject({
      session_id: 'sess-x',
      function_id: 'turn::function_awaiting_approval',
    });
  });
});
