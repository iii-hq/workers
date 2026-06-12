/**
 * Integration tests: prune flow using the large fixture.
 *
 * Tests:
 * 1. Large fixture with big tool output → pruned_tokens >= PRUNE_MIN_FREE (20_000).
 * 2. Pruned parts count matches session::update_message calls captured.
 */
import { describe, expect, it, vi } from 'vitest';
import { prune } from '../../../src/context-compaction/prune.js';
import type { AgentMessage } from '../../../src/types/agent-message.js';
import { loadFixture } from '../../fixtures/load.js';

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

const PRUNE_MIN_FREE = 20_000;

type MockTriggerReq = { function_id: string; payload: Record<string, unknown>; timeoutMs?: number };

function buildPruneMock(entries: Array<{ entry_id: string; message: AgentMessage }>) {
  const updatePartCalls: Array<{ entry_id: string }> = [];

  const trigger = vi.fn(async (req: MockTriggerReq) => {
    const { function_id, payload } = req;

    if (function_id === 'session::messages') {
      return { messages: entries };
    }
    if (function_id === 'session::update_message') {
      updatePartCalls.push({ entry_id: (payload as { entry_id: string }).entry_id });
      return { updated: true, revision: 1 };
    }

    return null;
  });

  const iii = { trigger } as unknown as Parameters<typeof prune>[0];

  return { iii, updatePartCalls };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const largeFixture = loadFixture('large-with-media');

describe('flow-prune: large fixture with big tool output', () => {
  it('prunes at least PRUNE_MIN_FREE tokens from the large fixture', async () => {
    const entries = largeFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message as AgentMessage,
    }));

    const { iii } = buildPruneMock(entries);

    const result = await prune(iii, largeFixture.session_id, {
      protectTokens: 0,
      minFree: PRUNE_MIN_FREE,
      protectedTools: [],
    });

    // The 50KB tool output alone = ~12,500 tokens; with other tool outputs should exceed 20k
    expect(result.pruned_tokens).toBeGreaterThanOrEqual(PRUNE_MIN_FREE);
    expect(result.pruned_parts).toBeGreaterThan(0);
  });

  it('pruned_parts count matches the number of session::update_message calls', async () => {
    const entries = largeFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message as AgentMessage,
    }));

    const { iii, updatePartCalls } = buildPruneMock(entries);

    const result = await prune(iii, largeFixture.session_id, {
      protectTokens: 0,
      minFree: PRUNE_MIN_FREE,
      protectedTools: [],
    });

    // The update_part calls must exactly match pruned_parts
    expect(updatePartCalls).toHaveLength(result.pruned_parts);
  });

  it('does not prune protected tools', async () => {
    const entries = largeFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message as AgentMessage,
    }));

    // Find the entry IDs of shell::run function results (which exist in the fixture)
    const shellRunEntries = entries.filter(
      (e) =>
        e.message.role === 'function_result' &&
        (e.message as { function_id?: string }).function_id === 'shell::run',
    );

    const { iii, updatePartCalls } = buildPruneMock(entries);

    await prune(iii, largeFixture.session_id, {
      protectTokens: 0,
      minFree: PRUNE_MIN_FREE,
      protectedTools: ['shell::run'],
    });

    // None of the protected shell::run entries should appear in update_part calls
    const prunedIds = new Set(updatePartCalls.map((c) => c.entry_id));
    for (const e of shellRunEntries) {
      expect(prunedIds.has(e.entry_id)).toBe(false);
    }
  });
});
