import { describe, expect, it, vi } from 'vitest';
import { handleSync } from '../../src/context-compaction/handler-sync.js';
import type { ISdk } from '../../src/runtime/iii.js';

/**
 * In-memory state store that correctly simulates state::get / state::set
 * so the lease nonce round-trip succeeds.
 */
function makeStubIii(triggerOverrides: Record<string, unknown> = {}): ISdk {
  const stateStore = new Map<string, unknown>();

  return {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      // Honour explicit overrides first
      if (Object.hasOwn(triggerOverrides, function_id)) {
        const v = triggerOverrides[function_id];
        return typeof v === 'function' ? (v as () => unknown)() : v;
      }

      const p = (payload ?? {}) as Record<string, unknown>;

      // In-memory state simulation so lease nonce round-trip works
      if (function_id === 'state::get') {
        const v = stateStore.get(p.key as string);
        return v !== undefined ? v : null;
      }
      if (function_id === 'state::set') {
        const v = p.value;
        if (v === null || v === undefined) {
          stateStore.delete(p.key as string);
        } else {
          stateStore.set(p.key as string, v);
        }
        return { ok: true };
      }
      if (function_id === 'state::update') {
        const key = p.key as string;
        const ops = (p.ops ?? []) as Array<{ type: string; value?: unknown }>;
        const oldValue = stateStore.has(key) ? stateStore.get(key) : null;
        let newValue: unknown = oldValue;
        for (const op of ops) {
          if (op.type === 'set') newValue = op.value;
        }
        if (newValue === null || newValue === undefined) {
          stateStore.delete(key);
        } else {
          stateStore.set(key, newValue);
        }
        return { old_value: oldValue ?? null, new_value: newValue ?? null };
      }

      return null;
    }),
    registerFunction: vi.fn(),
    registerTrigger: vi.fn(),
    publish: vi.fn(),
    subscribe: vi.fn(),
    enqueue: vi.fn(),
  } as unknown as ISdk;
}

const defaultModel = {
  id: 'claude-haiku-4-5',
  providerID: 'anthropic',
  limit: { context: 200_000, input: 200_000, output: 4_096 },
};

describe('handleSync smoke', () => {
  it('returns empty when session has no messages', async () => {
    const iii = makeStubIii({
      // No messages → selectWithEntryIds head will be empty → 'empty'
      'session::messages': { messages: [] },
    });

    const result = await handleSync(iii, {
      session_id: 'test-session',
      projected_tokens: 50_000,
      last_user_message_id: 'msg-1',
      model: defaultModel,
    });

    // Empty session → select returns empty head → summarizeAndAppend returns
    // 'empty' → handler-sync surfaces it as { status: 'empty' }.
    expect(result.status).toBe('empty');
  });

  it('returns busy when lease cannot be acquired within timeout', async () => {
    // Pre-fill state with a fresh lease to block acquisition
    const stateStore = new Map<string, unknown>();
    const leaseKey = 'session/test-session-busy/compaction_lease';
    stateStore.set(leaseKey, { nonce: 'held-by-other', ts: Date.now() - 1000 });

    const iii = {
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
          const p = (payload ?? {}) as Record<string, unknown>;
          if (function_id === 'state::get') {
            const v = stateStore.get(p.key as string);
            return v !== undefined ? v : null;
          }
          if (function_id === 'state::set') {
            const v = p.value;
            if (v === null || v === undefined) {
              stateStore.delete(p.key as string);
            } else {
              stateStore.set(p.key as string, v);
            }
            return { ok: true };
          }
          return null;
        },
      ),
      registerFunction: vi.fn(),
      registerTrigger: vi.fn(),
    } as unknown as ISdk;

    process.env.COMPACT_BUSY_TIMEOUT_MS = '1';
    try {
      const result = await handleSync(iii, {
        session_id: 'test-session-busy',
        projected_tokens: 50_000,
        last_user_message_id: 'msg-1',
        model: defaultModel,
      });
      expect(result.status).toBe('busy');
    } finally {
      process.env.COMPACT_BUSY_TIMEOUT_MS = undefined;
    }
  });
});
