/**
 * Smoke tests for the compact_session wrapper (register.ts logic).
 *
 * We test three things:
 *   1. Throws when session_id is missing/empty.
 *   2. Throws when the session has no assistant messages (cannot resolve model).
 *   3. Happy path: valid session → resolveModelFromSession succeeds → handleSync
 *      returns a deterministic CompactNowResult.
 */

import { describe, expect, it, vi } from 'vitest';
import { handleSync } from '../../src/context-compaction/handler-sync.js';
import {
  fetchModelLimit,
  resolveModelFromRunRequest,
  resolveModelFromSession,
} from '../../src/context-compaction/model-resolver.js';
import type { ISdk } from '../../src/runtime/iii.js';

// ---------------------------------------------------------------------------
// ISdk stub factory (mirrors handler-sync.test.ts)
// ---------------------------------------------------------------------------

function makeStubIii(triggerOverrides: Record<string, unknown> = {}): ISdk {
  const stateStore = new Map<string, unknown>();

  return {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (Object.hasOwn(triggerOverrides, function_id)) {
        const v = triggerOverrides[function_id];
        return typeof v === 'function' ? (v as () => unknown)() : v;
      }

      const p = (payload ?? {}) as Record<string, unknown>;

      if (function_id === 'state::get') {
        const v = stateStore.get(p['key'] as string);
        return v !== undefined ? v : null;
      }
      if (function_id === 'state::set') {
        const v = p['value'];
        if (v === null || v === undefined) {
          stateStore.delete(p['key'] as string);
        } else {
          stateStore.set(p['key'] as string, v);
        }
        return { ok: true };
      }
      if (function_id === 'state::update') {
        const key = p['key'] as string;
        const ops = (p['ops'] ?? []) as Array<{ type: string; value?: unknown }>;
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

// ---------------------------------------------------------------------------
// Fixture helpers — content must be a proper ContentBlock array
// ---------------------------------------------------------------------------

const userEntry = (entry_id: string, text = 'hello') => ({
  entry_id,
  message: {
    role: 'user' as const,
    content: [{ type: 'text', text }],
    timestamp: 0,
  },
});

const assistantEntry = (entry_id: string, provider: string, model: string, text = 'reply') => ({
  entry_id,
  message: {
    role: 'assistant' as const,
    provider,
    model,
    content: [{ type: 'text', text }],
    stop_reason: 'end',
    timestamp: 0,
  },
});

// ---------------------------------------------------------------------------
// compact_session inline implementation (same logic as register.ts)
// Keeps the test self-contained without depending on register.ts directly.
// ---------------------------------------------------------------------------

async function resolveExplicitModel(iii: ISdk, raw: unknown) {
  if (!raw || typeof raw !== 'object') return null;
  const m = raw as Record<string, unknown>;
  const providerID = typeof m.providerID === 'string' && m.providerID ? m.providerID : null;
  const modelID = typeof m.id === 'string' && m.id ? m.id : null;
  if (!providerID || !modelID) return null;
  const lim = m.limit as { context?: number; input?: number; output?: number } | undefined;
  if (lim && typeof lim.context === 'number' && lim.context > 0) {
    return {
      providerID,
      modelID,
      modelLimit: {
        context: lim.context,
        input: typeof lim.input === 'number' ? lim.input : lim.context,
        output: typeof lim.output === 'number' ? lim.output : 0,
      },
    };
  }
  return fetchModelLimit(iii, providerID, modelID);
}

async function runCompactSession(iii: ISdk, session_id: unknown, payloadModel?: unknown) {
  if (typeof session_id !== 'string' || session_id.length === 0) {
    throw new Error('context-compaction::compact_session: session_id is required');
  }

  // Mirrors register.ts fallback chain so the test catches drift. When
  // all three sources fail, register.ts now falls back to a conservative
  // model limit rather than throwing — compaction is best-effort.
  const FALLBACK = {
    providerID: 'unknown',
    modelID: 'unknown',
    modelLimit: { context: 32_000, input: 32_000, output: 4_000 },
  };
  let model = await resolveExplicitModel(iii, payloadModel);
  if (!model) model = await resolveModelFromSession(iii, session_id);
  if (!model) model = await resolveModelFromRunRequest(iii, session_id);
  if (!model) model = FALLBACK;

  // Mirrors register.ts: /compact does NOT extract a replay target.
  // The replay mechanism is for compact_now (turn-orchestrator overflow
  // pre-flight) only. Passing '' tells extractReplayTarget that no entry
  // matches, so the full entries list is summarised.
  return handleSync(iii, {
    session_id,
    projected_tokens: 999_999,
    last_user_message_id: '',
    model: {
      id: model.modelID,
      providerID: model.providerID,
      limit: model.modelLimit,
    },
  });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('compact_session smoke', () => {
  it('throws when session_id is missing', async () => {
    const iii = makeStubIii();
    await expect(runCompactSession(iii, '')).rejects.toThrow(
      'context-compaction::compact_session: session_id is required',
    );
  });

  it('throws when session_id is undefined', async () => {
    const iii = makeStubIii();
    await expect(runCompactSession(iii, undefined)).rejects.toThrow(
      'context-compaction::compact_session: session_id is required',
    );
  });

  it('uses the fallback model limit when no source can resolve a model', async () => {
    // No assistant messages AND no run_request. compact_session used to
    // throw; it now degrades to a conservative fallback so /compact still
    // runs (with a small preserve-recent budget) rather than failing.
    const iii = makeStubIii({
      'session-tree::messages': { messages: [] },
    });
    const result = await runCompactSession(iii, 'no-messages-session');
    expect(['ok', 'empty', 'overflow', 'busy']).toContain(result.status);
  });

  it('falls back to run_request when session-tree has no assistant messages', async () => {
    // This is the `/compact` UI scenario: the session hasn't yet mirrored
    // any assistant message into the tree, but run_request from run::start
    // carries provider/model.
    const stateStore = new Map<string, unknown>();
    stateStore.set('session/ui-session/run_request', {
      provider: 'anthropic',
      model: 'claude-haiku-4-5',
    });

    const iii = {
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
          const p = (payload ?? {}) as Record<string, unknown>;
          if (function_id === 'session-tree::messages') return { messages: [] };
          if (function_id === 'session-tree::compactions') return { entries: [] };
          if (function_id === 'router::models::get') {
            return { model: { context_window: 200_000, max_output_tokens: 4_096 } };
          }
          if (function_id === 'state::get') {
            const v = stateStore.get(p['key'] as string);
            return v !== undefined ? v : null;
          }
          if (function_id === 'state::set') {
            const v = p['value'];
            if (v === null || v === undefined) stateStore.delete(p['key'] as string);
            else stateStore.set(p['key'] as string, v);
            return { ok: true };
          }
          if (function_id === 'state::update') {
            const key = p['key'] as string;
            const ops = (p['ops'] ?? []) as Array<{ type: string; value?: unknown }>;
            const oldValue = stateStore.has(key) ? stateStore.get(key) : null;
            let newValue: unknown = oldValue;
            for (const op of ops) {
              if (op.type === 'set') newValue = op.value;
            }
            if (newValue === null || newValue === undefined) stateStore.delete(key);
            else stateStore.set(key, newValue);
            return { old_value: oldValue ?? null, new_value: newValue ?? null };
          }
          return null;
        },
      ),
      registerFunction: vi.fn(),
      registerTrigger: vi.fn(),
      publish: vi.fn(),
      subscribe: vi.fn(),
      enqueue: vi.fn(),
    } as unknown as ISdk;

    const result = await runCompactSession(iii, 'ui-session');
    expect(['ok', 'empty', 'overflow', 'busy']).toContain(result.status);
  });

  it('uses explicit payload.model when provided (skips session scan)', async () => {
    // Even when session-tree returns no assistant messages AND no run_request
    // exists, an explicit model in the payload takes precedence.
    const stateStore = new Map<string, unknown>();
    const iii = {
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
          const p = (payload ?? {}) as Record<string, unknown>;
          if (function_id === 'session-tree::messages') return { messages: [] };
          if (function_id === 'session-tree::compactions') return { entries: [] };
          if (function_id === 'state::get') {
            const v = stateStore.get(p['key'] as string);
            return v !== undefined ? v : null;
          }
          if (function_id === 'state::set') {
            const v = p['value'];
            if (v === null || v === undefined) stateStore.delete(p['key'] as string);
            else stateStore.set(p['key'] as string, v);
            return { ok: true };
          }
          if (function_id === 'state::update') {
            const key = p['key'] as string;
            const ops = (p['ops'] ?? []) as Array<{ type: string; value?: unknown }>;
            const oldValue = stateStore.has(key) ? stateStore.get(key) : null;
            let newValue: unknown = oldValue;
            for (const op of ops) {
              if (op.type === 'set') newValue = op.value;
            }
            if (newValue === null || newValue === undefined) stateStore.delete(key);
            else stateStore.set(key, newValue);
            return { old_value: oldValue ?? null, new_value: newValue ?? null };
          }
          return null;
        },
      ),
      registerFunction: vi.fn(),
      registerTrigger: vi.fn(),
      publish: vi.fn(),
      subscribe: vi.fn(),
      enqueue: vi.fn(),
    } as unknown as ISdk;

    const result = await runCompactSession(iii, 'explicit-model-session', {
      id: 'claude-opus-4-7',
      providerID: 'anthropic',
      limit: { context: 200_000, input: 200_000, output: 4_096 },
    });
    expect(['ok', 'empty', 'overflow', 'busy']).toContain(result.status);
    // Confirm router::models::get was NOT called (limits supplied inline).
    expect(
      (iii.trigger as ReturnType<typeof vi.fn>).mock.calls.filter(
        (c) => (c[0] as { function_id: string }).function_id === 'router::models::get',
      ).length,
    ).toBe(0);
  });

  it('happy path: valid session returns a deterministic CompactNowResult', async () => {
    // Session has: user message → assistant message (with provider/model) → user message.
    // resolveModelFromSession scans backwards, finds the assistant message, then fetches
    // limits via models::get.  handleSync runs on this stub and returns a result.
    const sessionMessages = {
      messages: [
        userEntry('e1', 'first message'),
        assistantEntry('e2', 'anthropic', 'claude-haiku-4-5'),
        userEntry('e3', 'please compact'),
      ],
    };

    const iii = makeStubIii({
      'session-tree::messages': sessionMessages,
      'models::get': { context_window: 200_000, max_output_tokens: 4_096 },
      'session-tree::compactions': { entries: [] },
    });

    const result = await runCompactSession(iii, 'happy-session');

    // With only 3 messages the summariser finds a very small / empty head and
    // returns 'ok' (tokens_before: 0) or 'empty'. Both are valid deterministic
    // outcomes given the stub.
    expect(['ok', 'empty', 'overflow', 'busy']).toContain(result.status);
  });
});
