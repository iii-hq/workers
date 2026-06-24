import { vi } from 'vitest';
import type { ISdk } from 'iii-sdk';

export type TriggerCall = { function_id: string; payload: Record<string, unknown> };

export type FakeIii = {
  iii: ISdk;
  calls: TriggerCall[];
  state: Map<string, unknown>;
  streamFrames: (stream: string) => Array<Record<string, unknown>>;
  registered: Map<string, (payload: unknown) => Promise<unknown>>;
};

/**
 * In-memory stand-in for the engine bus: `state::get/set/list` backed by a
 * Map keyed `${scope}/${key}`, `stream::set` recorded as plain calls, and
 * `registerFunction` captured so tests can invoke handlers at the same
 * unknown boundary the engine uses.
 */
export function fakeIii(): FakeIii {
  const calls: TriggerCall[] = [];
  const state = new Map<string, unknown>();
  const registered = new Map<string, (payload: unknown) => Promise<unknown>>();

  const iii = {
    trigger: async (req: { function_id: string; payload: Record<string, unknown> }) => {
      // Clone like the wire would: the live bus serializes payloads, so
      // later caller-side mutation must not rewrite recorded calls.
      const payload = structuredClone(req.payload);
      calls.push({ function_id: req.function_id, payload });
      const { scope, key, value } = payload as { scope?: string; key?: string; value?: unknown };
      if (req.function_id === 'state::set') {
        state.set(`${scope}/${key}`, value);
        return null;
      }
      if (req.function_id === 'state::get') return state.get(`${scope}/${key}`) ?? null;
      if (req.function_id === 'state::list') {
        return [...state.entries()].filter(([k]) => k.startsWith(`${scope}/`)).map(([, v]) => v);
      }
      return null;
    },
    registerFunction: vi.fn((fnId: string, handler: (payload: unknown) => Promise<unknown>) => {
      registered.set(fnId, handler);
    }),
    registerTrigger: vi.fn(),
  } as unknown as ISdk;

  const streamFrames = (stream: string) =>
    calls
      .filter(
        (c) =>
          c.function_id === 'stream::set' &&
          (c.payload as { stream_name?: string }).stream_name === stream,
      )
      .map((c) => c.payload);

  return { iii, calls, state, streamFrames, registered };
}
