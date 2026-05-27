/**
 * Shared minimal `ISdk` fake for approval-gate tests. Captures every
 * `iii.trigger` call by `function_id` so individual tests can assert on
 * the payload without re-implementing the mock surface each time.
 */

import type { ISdk } from 'iii-sdk';
import { vi } from 'vitest';

export type TriggerCall = { function_id: string; payload: unknown; action?: unknown };

export type FakeIii = {
  iii: ISdk;
  calls: TriggerCall[];
  streamSets: unknown[];
};

export function fakeIii(): FakeIii {
  const calls: TriggerCall[] = [];
  const streamSets: unknown[] = [];

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
      calls.push({ function_id, payload, action });
      if (function_id === 'stream::set') {
        streamSets.push(payload);
      }
      return null;
    }),
  } as unknown as ISdk;

  return { iii, calls, streamSets };
}
