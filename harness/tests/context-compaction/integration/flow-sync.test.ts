/**
 * Integration tests: sync (pre-turn) compaction flow via handleSync.
 *
 * Tests:
 * 1. Success path: status === 'ok', no replay reinjection (the last user
 *    message stays on the path), session-tree::append_synthetic posts the
 *    continue nudge.
 * 2. Summariser stream returns error → status === 'overflow'.
 * 3. Lease held → status === 'busy'.
 */
import { describe, expect, it, vi } from 'vitest';
import { payloadStoreKey, stateStoreKey } from '../../_helpers/stateStoreKey.js';
import { handleSync } from '../../../src/context-compaction/handler-sync.js';
import type { ISdk } from '../../../src/runtime/iii.js';
import { loadFixture } from '../../fixtures/load.js';

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

type MockTriggerReq = { function_id: string; payload: Record<string, unknown>; timeoutMs?: number };

const defaultModel = {
  id: 'claude-haiku-4-5',
  providerID: 'anthropic',
  limit: { context: 200_000, input: 200_000, output: 4_096 },
};

/**
 * Build an ISdk-compatible mock for the sync flow.
 *
 * @param opts.fixtureMessages - entries returned by session-tree::messages
 * @param opts.providerError - if true, make createChannel reject to simulate summariser error
 * @param opts.stateStore - optional shared state store for lease simulation
 */
function buildSyncMock(opts: {
  fixtureMessages: Array<{ entry_id: string; message: unknown }>;
  providerError?: boolean;
  stateStore?: Map<string, unknown>;
}) {
  const { fixtureMessages, providerError = false, stateStore = new Map<string, unknown>() } = opts;

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

  const doneSummaryEvent = JSON.stringify({
    type: 'done',
    message: {
      role: 'assistant',
      content: [{ type: 'text', text: 'Sync compaction summary text.' }],
      stop_reason: 'end',
      model: 'claude-haiku-4-5',
      provider: 'anthropic',
      timestamp: Date.now(),
    },
  });

  const appendCalls: unknown[] = [];
  const appendSyntheticCalls: unknown[] = [];
  const updatePartCalls: unknown[] = [];

  const trigger = vi.fn(async (req: MockTriggerReq) => {
    const { function_id, payload } = req;

    if (function_id === 'session-tree::messages') {
      return { messages: fixtureMessages };
    }
    if (function_id === 'session-tree::compactions') {
      return { entries: [] };
    }
    if (function_id === 'session-tree::compact') {
      return undefined;
    }
    if (function_id === 'session-tree::append') {
      appendCalls.push(payload);
      return undefined;
    }
    if (function_id === 'session-tree::append_synthetic') {
      appendSyntheticCalls.push(payload);
      return undefined;
    }
    if (function_id === 'session-tree::update_part') {
      updatePartCalls.push(payload);
      return { ok: true };
    }
    if (function_id === 'state::get') {
      const v = stateStore.get(payloadStoreKey(payload as { scope?: string; key?: string }));
      return v !== undefined ? v : null;
    }
    if (function_id === 'state::set') {
      const p = payload as { key: string; value: unknown; scope?: string };
      const storeKey = payloadStoreKey(p);
      if (p.value === null || p.value === undefined) {
        stateStore.delete(storeKey);
      } else {
        stateStore.set(storeKey, p.value);
      }
      return { ok: true };
    }
    if (function_id === 'state::update') {
      const p = payload as {
        key: string;
        scope?: string;
        ops: Array<{ type: string; value?: unknown }>;
      };
      const storeKey = payloadStoreKey(p);
      const oldValue = stateStore.has(storeKey) ? stateStore.get(storeKey) : null;
      let newValue: unknown = oldValue;
      for (const op of p.ops ?? []) {
        if (op.type === 'set') newValue = op.value;
      }
      if (newValue === null || newValue === undefined) {
        stateStore.delete(storeKey);
      } else {
        stateStore.set(storeKey, newValue);
      }
      return { old_value: oldValue ?? null, new_value: newValue ?? null };
    }
    if (function_id.startsWith('provider::')) {
      if (channelCb) {
        channelCb(doneSummaryEvent);
      }
      return undefined;
    }

    return undefined;
  });

  const createChannel = providerError
    ? vi.fn().mockRejectedValue(new Error('provider channel unavailable'))
    : vi.fn(async () => channel);

  const iii = { trigger, createChannel } as unknown as ISdk;

  return { iii, trigger, appendCalls, appendSyntheticCalls, updatePartCalls };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const mediumFixture = loadFixture('medium-with-tools');

describe('flow-sync: success path', () => {
  it('returns ok and appends the continue nudge (no replay reinjection)', async () => {
    const fixtureMessages = mediumFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message,
    }));

    // Use the second-to-last user message as the replay target
    const userEntries = mediumFixture.entries.filter((e) => {
      const msg = e.message as { role: string };
      return msg.role === 'user';
    });
    // Pick a user entry that is NOT the very first so extractReplayTarget finds a truncated head
    const lastUserEntry = userEntries[userEntries.length - 1]!;

    const { iii, appendCalls, appendSyntheticCalls } = buildSyncMock({ fixtureMessages });

    const result = await handleSync(iii, {
      session_id: mediumFixture.session_id,
      projected_tokens: 150_000,
      last_user_message_id: lastUserEntry.id,
      model: defaultModel,
    });

    expect(result.status).toBe('ok');

    // The last user message is already on the path as the compaction's
    // parent; we no longer reinject it via session-tree::append.
    expect(appendCalls).toHaveLength(0);

    // append_synthetic posts the continue nudge as a child of the compaction.
    expect(appendSyntheticCalls).toHaveLength(1);
    const syntheticPayload = appendSyntheticCalls[0] as Record<string, unknown>;
    expect(syntheticPayload.session_id).toBe(mediumFixture.session_id);
    expect(String(syntheticPayload.text)).toContain('Continue');
  });
});

describe('flow-sync: summariser error → overflow', () => {
  it('returns overflow when the summariser stream fails', async () => {
    const fixtureMessages = mediumFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message,
    }));

    // providerError=true makes createChannel reject, causing summarizeAndAppend to return 'compact'
    const { iii } = buildSyncMock({ fixtureMessages, providerError: true });

    const result = await handleSync(iii, {
      session_id: `${mediumFixture.session_id}-err`,
      projected_tokens: 150_000,
      last_user_message_id: 'nonexistent-last-msg',
      model: defaultModel,
    });

    expect(result.status).toBe('overflow');
  });
});

describe('flow-sync: lease held → busy', () => {
  it('returns busy when another compaction is already holding the lease', async () => {
    const fixtureMessages = mediumFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message,
    }));

    // Pre-populate the state store with an active lease for this session
    const sessionId = `${mediumFixture.session_id}-busy`;
    const stateStore = new Map<string, unknown>();
    const leaseStoreKey = stateStoreKey('compaction_lease', sessionId);
    stateStore.set(leaseStoreKey, { nonce: 'held-by-another', ts: Date.now() - 1000 });

    const { iii } = buildSyncMock({ fixtureMessages, stateStore });

    // Set a very short busy timeout so the test completes quickly
    const original = process.env.COMPACT_BUSY_TIMEOUT_MS;
    process.env.COMPACT_BUSY_TIMEOUT_MS = '1';
    try {
      const result = await handleSync(iii, {
        session_id: sessionId,
        projected_tokens: 150_000,
        last_user_message_id: 'any-msg',
        model: defaultModel,
      });

      expect(result.status).toBe('busy');
    } finally {
      if (original === undefined) {
        delete process.env.COMPACT_BUSY_TIMEOUT_MS;
      } else {
        process.env.COMPACT_BUSY_TIMEOUT_MS = original;
      }
    }
  });
});
