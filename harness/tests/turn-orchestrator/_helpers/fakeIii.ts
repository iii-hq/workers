/**
 * Shared fake `ISdk` builder for turn-orchestrator tests.
 *
 * Covers the canonical "`{ iii, calls }`" pattern duplicated across most
 * unit tests: every `iii.trigger(...)` is captured into `calls[]`. Pass
 * any `Partial<ISdk>` fields (e.g. `createChannel`, `registerFunction`)
 * directly; pass a `responder` to control what `trigger()` returns.
 *
 * For the heavier "state-store + event-capture" shape (used by the e2e
 * tests under `tests/integration/`), keep the per-file implementations
 * — this helper deliberately stays narrow.
 */

import { vi } from 'vitest';
import type { ISdk } from '../../../src/runtime/iii.js';

export type TriggerCall = {
  function_id: string;
  payload: unknown;
  action?: unknown;
  timeoutMs?: number;
};

/** Responder: either a `function_id`-keyed lookup or a callback. */
export type FakeIiiResponder =
  | Record<string, unknown>
  | ((req: { function_id: string; payload: unknown }) => unknown | Promise<unknown>);

export type FakeIiiOptions = Partial<ISdk> & {
  responder?: FakeIiiResponder;
};

export function fakeIii(opts: FakeIiiOptions = {}): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const { responder, ...overrides } = opts;

  const iii = {
    trigger: async <T, R>(req: {
      function_id: string;
      payload: T;
      action?: unknown;
      timeoutMs?: number;
    }): Promise<R> => {
      calls.push({
        function_id: req.function_id,
        payload: req.payload,
        action: req.action,
        timeoutMs: req.timeoutMs,
      });
      if (!responder) return null as R;
      if (typeof responder === 'function') {
        return (await responder({ function_id: req.function_id, payload: req.payload })) as R;
      }
      return ((responder as Record<string, unknown>)[req.function_id] ?? null) as R;
    },
    registerFunction: vi.fn(),
    ...overrides,
  } as unknown as ISdk;

  return { iii, calls };
}
