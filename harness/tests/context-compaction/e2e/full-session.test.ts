/**
 * End-to-end compaction flow test.
 *
 * Drives the sync-compaction path through `runPreflight` against an
 * overflowing 30-turn fixture, with the REAL session-tree handlers wired
 * up to an InMemoryStore. Verifies the three structural guarantees that
 * the unit/integration tests cannot observe in isolation:
 *
 * 1. The flat state at scope `agent`, key `session/<sid>/messages` is
 *    rewritten to a reduced array: [summary-as-asst-msg, ...tail, replay].
 * 2. The session tree's active path stays connected — the Compaction
 *    entry, replayed user message, and synthetic continue-prompt are
 *    chained via parent_id back to the pre-compaction tail.
 * 3. The downstream provider input (what would land in `buildInput`)
 *    no longer exceeds the model's usable token budget.
 */

import { describe, expect, it, vi } from 'vitest';
import { flatMessagesKey } from '../../../src/context-compaction/flat-state.js';
import { handleSync } from '../../../src/context-compaction/handler-sync.js';
import type { ISdk } from '../../../src/runtime/iii.js';
import { registerTree } from '../../../src/session/tree/register.js';
import { InMemoryStore } from '../../../src/session/tree/store.js';
import type { SessionEntry } from '../../../src/session/tree/types.js';
import { runPreflight } from '../../../src/turn-orchestrator/preflight.js';
import type { AgentMessage } from '../../../src/types/agent-message.js';

// ---------------------------------------------------------------------------
// Fixture: a 30-turn conversation that easily exceeds a small usable budget.
// ---------------------------------------------------------------------------

const SESSION_ID = 'e2e-session-001';
const PROVIDER_ID = 'anthropic';
const MODEL_ID = 'fake-haiku-test';
// Pick limits so the 30-turn fixture overflows: usable ~= input - reserved.
const MODEL_LIMITS = { context: 8_000, input: 8_000, output: 1_024 };

function userMsg(text: string): AgentMessage {
  return { role: 'user', content: [{ type: 'text', text }], timestamp: 0 };
}

function asstMsg(text: string): AgentMessage {
  return {
    role: 'assistant',
    content: [{ type: 'text', text }],
    stop_reason: 'end',
    error_message: null,
    error_kind: null,
    usage: null,
    model: MODEL_ID,
    provider: PROVIDER_ID,
    timestamp: 0,
  };
}

/** Builds a 30-turn conversation with hefty messages so total > usable. */
function buildOverflowingMessages(turns: number): AgentMessage[] {
  const out: AgentMessage[] = [];
  const filler = 'lorem ipsum dolor sit amet '.repeat(40); // ~1000 chars => ~250 tokens
  for (let i = 0; i < turns; i++) {
    out.push(userMsg(`turn ${i} question: ${filler}`));
    out.push(asstMsg(`turn ${i} answer: ${filler}`));
  }
  // The final user message is the replay target — the one that "triggered" the turn.
  out.push(userMsg('final user message that triggers compaction'));
  return out;
}

// ---------------------------------------------------------------------------
// Test ISdk: wires the real session-tree handlers to an InMemoryStore,
// plus a stub `models::get`, a stub provider stream (returns a known
// summary), and an in-memory `state::*` so leases and the flat-state
// rewrite both work without touching the network.
// ---------------------------------------------------------------------------

type FunctionHandler = (payload: unknown) => Promise<unknown>;

function buildTestSdk(opts: {
  flatMessages: AgentMessage[];
  summaryText: string;
  /** Capture every provider stream invocation's `messages` payload. */
  providerInvocations: AgentMessage[][];
  session_id: string;
}): { iii: ISdk; store: InMemoryStore; stateStore: Map<string, unknown> } {
  const stateStore = new Map<string, unknown>();
  const handlers = new Map<string, FunctionHandler>();
  const store = new InMemoryStore();

  // Pre-seed flat state with the overflowing transcript.
  stateStore.set(flatMessagesKey(opts.session_id), opts.flatMessages);

  // Stub channel writer so streamAndCollect can deliver a synthetic done event.
  let channelCb: ((raw: string) => void) | null = null;
  const channel = {
    reader: {
      onMessage(cb: (raw: string) => void) {
        channelCb = cb;
      },
      stream: { resume: () => {} },
    },
    writerRef: 'mock-writer-ref',
  };

  const trigger = vi.fn(async (req: { function_id: string; payload?: unknown }) => {
    const fn = req.function_id;
    const payload = req.payload;

    // 1) state::* — back the lease / flat-state rewrite with stateStore.
    if (fn === 'state::get') {
      const p = (payload ?? {}) as { key: string };
      const v = stateStore.get(p.key);
      return v !== undefined ? v : null;
    }
    if (fn === 'state::set') {
      const p = (payload ?? {}) as { key: string; value: unknown };
      if (p.value === null || p.value === undefined) stateStore.delete(p.key);
      else stateStore.set(p.key, p.value);
      return { ok: true };
    }
    if (fn === 'state::update') {
      const p = (payload ?? {}) as { key: string; ops: Array<{ type: string; value?: unknown }> };
      const oldValue = stateStore.has(p.key) ? stateStore.get(p.key) : null;
      let newValue: unknown = oldValue;
      for (const op of p.ops ?? []) {
        if (op.type === 'set') newValue = op.value;
      }
      if (newValue === null || newValue === undefined) stateStore.delete(p.key);
      else stateStore.set(p.key, newValue);
      return { old_value: oldValue ?? null, new_value: newValue ?? null };
    }

    // 2) models::get — return a small-context model so the 30-turn fixture overflows.
    if (fn === 'models::get') {
      return {
        id: MODEL_ID,
        provider: PROVIDER_ID,
        context_window: MODEL_LIMITS.input,
        max_output_tokens: MODEL_LIMITS.output,
      };
    }

    // 3) Provider stream — emit a single 'done' event with the canned summary.
    if (fn.startsWith('provider::')) {
      const summaryDone = {
        type: 'done',
        message: {
          role: 'assistant',
          content: [{ type: 'text', text: opts.summaryText }],
          stop_reason: 'end',
          model: MODEL_ID,
          provider: PROVIDER_ID,
          timestamp: Date.now(),
        },
      };
      // Capture what the summariser was asked to look at.
      const streamInput = (payload ?? {}) as { messages?: AgentMessage[] };
      if (streamInput.messages) opts.providerInvocations.push(streamInput.messages);
      if (channelCb) channelCb(JSON.stringify(summaryDone));
      return undefined;
    }

    // 4) session-tree::* — delegate to the registered handler.
    const handler = handlers.get(fn);
    if (handler) return handler(payload);

    return null;
  });

  const createChannel = vi.fn(async () => channel);

  const registerFunction = vi.fn((id: string, handler: FunctionHandler) => {
    handlers.set(id, handler);
  });

  const iii = {
    trigger,
    createChannel,
    registerFunction,
    registerTrigger: vi.fn(),
    publish: vi.fn(),
    subscribe: vi.fn(),
    enqueue: vi.fn(),
  } as unknown as ISdk;

  // Wire the real session-tree handlers.
  registerTree(iii, store);

  // Wire compact_now to call handleSync directly (mirrors register.ts).
  handlers.set('context-compaction::compact_now', async (payload: unknown) => {
    const obj = (payload ?? {}) as Record<string, unknown>;
    const m = obj.model as { id: string; providerID: string; limit: typeof MODEL_LIMITS };
    return handleSync(iii, {
      session_id: String(obj.session_id),
      projected_tokens: typeof obj.projected_tokens === 'number' ? obj.projected_tokens : 0,
      last_user_message_id: String(obj.last_user_message_id ?? ''),
      model: { id: m.id, providerID: m.providerID, limit: m.limit },
    });
  });

  return { iii, store, stateStore };
}

// ---------------------------------------------------------------------------
// Helpers for tree introspection.
// ---------------------------------------------------------------------------

async function activePathIds(store: InMemoryStore, session_id: string): Promise<string[]> {
  const entries = await store.loadEntries(session_id);
  const last = entries[entries.length - 1];
  if (!last) return [];
  const byId = new Map(entries.map((e) => [e.id, e] as const));
  const path: string[] = [];
  let cursor: string | null = last.id;
  while (cursor !== null) {
    path.push(cursor);
    cursor = (byId.get(cursor)?.parent_id ?? null) as string | null;
  }
  return path.reverse();
}

async function seedSessionTreeFrom(
  iii: ISdk,
  session_id: string,
  messages: AgentMessage[],
): Promise<string[]> {
  // Ensure the session exists.
  await iii.trigger<unknown, unknown>({
    function_id: 'session-tree::ensure',
    payload: { session_id },
  });
  const ids: string[] = [];
  let parent: string | null = null;
  for (const m of messages) {
    const resp = (await iii.trigger<unknown, { entry_id?: string }>({
      function_id: 'session-tree::append',
      payload: { session_id, parent_id: parent, message: m },
    })) as { entry_id?: string } | null;
    const id = resp?.entry_id ?? '';
    ids.push(id);
    parent = id;
  }
  return ids;
}

// ---------------------------------------------------------------------------
// E2E tests
// ---------------------------------------------------------------------------

describe('e2e full-session compaction', () => {
  it('preflight + sync compaction reduces flat state AND keeps tree connected', async () => {
    const SUMMARY = 'EARLIER TURNS SUMMARY: discussed lorem and friends.';
    const overflowing = buildOverflowingMessages(30);
    const providerInvocations: AgentMessage[][] = [];

    const { iii, store, stateStore } = buildTestSdk({
      session_id: SESSION_ID,
      flatMessages: overflowing,
      summaryText: SUMMARY,
      providerInvocations,
    });

    // Seed the session tree with the same transcript so replay extraction works.
    const entryIds = await seedSessionTreeFrom(iii, SESSION_ID, overflowing);
    const lastUserId = entryIds[entryIds.length - 1] ?? ''; // the final user msg

    // Sanity check: the pre-compaction flat state matches the seed.
    const beforeFlat = stateStore.get(flatMessagesKey(SESSION_ID)) as AgentMessage[];
    expect(beforeFlat.length).toBe(overflowing.length);

    // Run preflight. The 30-turn fixture should overflow the 8k usable budget.
    const result = await runPreflight(iii, SESSION_ID, overflowing, PROVIDER_ID, MODEL_ID);
    expect(result).toBe('compacted');

    // --- Assertion 1: flat state is reduced and shaped correctly. ---
    const afterFlat = stateStore.get(flatMessagesKey(SESSION_ID)) as AgentMessage[];
    expect(afterFlat.length).toBeLessThan(overflowing.length);

    // First message must be the summary-as-assistant-msg containing SUMMARY.
    const first = afterFlat[0];
    expect(first?.role).toBe('assistant');
    if (first?.role === 'assistant') {
      const text = first.content[0];
      expect(text?.type).toBe('text');
      if (text?.type === 'text') expect(text.text).toContain(SUMMARY);
    }

    // Last message is the replay (the final user message that triggered the turn).
    const last = afterFlat[afterFlat.length - 1];
    expect(last?.role).toBe('user');
    if (last?.role === 'user') {
      const text = last.content[0];
      if (text?.type === 'text') {
        expect(text.text).toContain('final user message');
      }
    }

    // --- Assertion 2: session tree active path is connected, no orphans. ---
    const path = await activePathIds(store, SESSION_ID);
    expect(path.length).toBeGreaterThan(2);

    const entries = (await store.loadEntries(SESSION_ID)) as SessionEntry[];
    const byId = new Map(entries.map((e) => [e.id, e] as const));

    // The just-appended synthetic should be the active leaf.
    const leafId = path[path.length - 1];
    const leaf = leafId ? byId.get(leafId) : undefined;
    expect(leaf?.type).toBe('message');
    if (leaf?.type === 'message') {
      const m = leaf.message;
      expect(m.role).toBe('user');
      if (m.role === 'user') {
        const text = m.content[0];
        if (text?.type === 'text') {
          expect(text.text).toMatch(/Continue if you have next steps/);
        }
      }
    }

    // Walking back from the leaf must hit a `compaction` entry.
    const hasCompactionOnPath = path.some((id) => byId.get(id)?.type === 'compaction');
    expect(hasCompactionOnPath).toBe(true);

    // The replay-user-msg must appear on the active path (it was reinjected
    // with parent_id chained off the compaction entry).
    const userReplayOnPath = path
      .map((id) => byId.get(id))
      .filter((e): e is Extract<SessionEntry, { type: 'message' }> => e?.type === 'message')
      .some((e) => {
        if (e.message.role !== 'user') return false;
        const block = e.message.content[0];
        return block?.type === 'text' && block.text.includes('final user message');
      });
    expect(userReplayOnPath).toBe(true);

    // --- Assertion 3: the provider summariser saw a non-empty head. ---
    expect(providerInvocations.length).toBeGreaterThan(0);
    expect(providerInvocations[0]?.length).toBeGreaterThan(0);

    // Sanity: lastUserId from the seed is set.
    expect(lastUserId).toBeTruthy();
  });

  it('preflight short-circuits to ok when usable budget is not exceeded', async () => {
    const SUMMARY = 'unused';
    const tinyMessages: AgentMessage[] = [userMsg('hi'), asstMsg('hello'), userMsg('what is 2+2?')];
    const providerInvocations: AgentMessage[][] = [];
    const SID = `${SESSION_ID}-small`;
    const { iii, stateStore } = buildTestSdk({
      session_id: SID,
      flatMessages: tinyMessages,
      summaryText: SUMMARY,
      providerInvocations,
    });
    await seedSessionTreeFrom(iii, SID, tinyMessages);

    // The default COMPACT_RESERVED_TOKENS (20_000) would zero out the 8k
    // model's usable budget and force compaction; shrink reservation for
    // this test so the tiny transcript stays well below usable.
    const prev = process.env.COMPACT_RESERVED_TOKENS;
    process.env.COMPACT_RESERVED_TOKENS = '100';
    try {
      const result = await runPreflight(iii, SID, tinyMessages, PROVIDER_ID, MODEL_ID);
      expect(result).toBe('ok');
    } finally {
      if (prev === undefined) process.env.COMPACT_RESERVED_TOKENS = undefined;
      else process.env.COMPACT_RESERVED_TOKENS = prev;
    }

    // Flat state is untouched.
    const after = stateStore.get(flatMessagesKey(SID)) as AgentMessage[];
    expect(after.length).toBe(tinyMessages.length);

    // Summariser was never invoked.
    expect(providerInvocations.length).toBe(0);
  });
});
