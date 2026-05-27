/**
 * Richer fake `ISdk` for tests that drive `createTurnStore` end-to-end:
 * implements an in-memory `state::get` / `state::set` / `state::update`
 * surface (matching the engine's semantics for `saveRecord`) and captures
 * every `turn::*` wake into `wakeInvocations`.
 *
 * Use this for FSM-wake integration tests; for pure unit tests, use the
 * lighter `fakeIii` helper.
 */

import { vi } from 'vitest';
import type { ISdk } from '../../../src/runtime/iii.js';

export type WakeInvocation = {
  session_id: string;
  function_id: string;
  action?: unknown;
};

export type StateStoreIii = {
  iii: ISdk;
  wakeInvocations: WakeInvocation[];
  stateStore: Map<string, unknown>;
};

export function fakeIiiWithStateStore(): StateStoreIii {
  const stateStore = new Map<string, unknown>();
  const wakeInvocations: WakeInvocation[] = [];
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
        if (function_id === 'state::get') {
          const p = payload as { scope: string; key: string };
          const v = stateStore.get(`${p.scope}/${p.key}`);
          return v === undefined ? null : structuredClone(v);
        }

        if (function_id === 'state::set') {
          const p = payload as { scope: string; key: string; value: unknown };
          const storeKey = `${p.scope}/${p.key}`;
          const old_value = stateStore.has(storeKey)
            ? structuredClone(stateStore.get(storeKey))
            : null;
          const new_value = structuredClone(p.value);
          stateStore.set(storeKey, new_value);
          return { old_value, new_value };
        }

        if (function_id === 'state::update') {
          return { old_value: 0 };
        }

        if (function_id.startsWith('turn::')) {
          const p = payload as { session_id: string };
          wakeInvocations.push({ session_id: p.session_id, function_id, action });
          return null;
        }

        return null;
      },
    ),
  };

  return { iii: iii as unknown as ISdk, wakeInvocations, stateStore };
}
