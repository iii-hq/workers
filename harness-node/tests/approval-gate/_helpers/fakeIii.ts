/**
 * Shared minimal `ISdk` fake for approval-gate tests. Captures every
 * `iii.trigger` call by `function_id` so individual tests can assert on
 * the payload without re-implementing the mock surface each time.
 */

import type { ISdk } from 'iii-sdk';
import { vi } from 'vitest';

export type TriggerCall = { function_id: string; payload: unknown };

export type FakeIii = {
  iii: ISdk;
  calls: TriggerCall[];
  resumeCalls: TriggerCall[];
  streamSets: unknown[];
};

export function fakeIii(): FakeIii {
  const calls: TriggerCall[] = [];
  const resumeCalls: TriggerCall[] = [];
  const streamSets: unknown[] = [];

  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      calls.push({ function_id, payload });
      if (function_id.startsWith('turn::approval_resume::')) {
        resumeCalls.push({ function_id, payload });
      } else if (function_id === 'stream::set') {
        streamSets.push(payload);
      }
      return null;
    }),
  } as unknown as ISdk;

  return { iii, calls, resumeCalls, streamSets };
}
