/**
 * Shared minimal `ISdk` fake for approval-gate tests. Captures every
 * `iii.trigger` call by `function_id` so individual tests can assert on
 * the payload without re-implementing the mock surface each time.
 */

import type { ISdk } from 'iii-sdk';
import { vi } from 'vitest';
import type {
  StateSetApprovalPayload,
  TurnStepPayload,
} from '../../../src/approval-gate/schemas.js';

export type TriggerCall = { function_id: string; payload: unknown };

export type FakeIii = {
  iii: ISdk;
  calls: TriggerCall[];
  /** Convenience filters for the function ids approval-gate cares about. */
  setCalls: StateSetApprovalPayload[];
  stepCalls: TurnStepPayload[];
  streamSets: unknown[];
};

export function fakeIii(): FakeIii {
  const calls: TriggerCall[] = [];
  const setCalls: FakeIii['setCalls'] = [];
  const stepCalls: FakeIii['stepCalls'] = [];
  const streamSets: unknown[] = [];

  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      calls.push({ function_id, payload });
      // Single cast at the wire/typed boundary: the fake accepts any
      // trigger surface, but our convenience arrays know what to expect
      // for the approval-gate's specific function ids.
      if (function_id === 'state::set') {
        setCalls.push(payload as StateSetApprovalPayload);
      } else if (function_id === 'turn::step') {
        stepCalls.push(payload as TurnStepPayload);
      } else if (function_id === 'stream::set') {
        streamSets.push(payload);
      }
      return null;
    }),
  } as unknown as ISdk;

  return { iii, calls, setCalls, stepCalls, streamSets };
}
