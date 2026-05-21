import { describe, expect, it } from 'vitest';
import { appendMessage, createSession, updatePart } from '../../src/session/tree/operations.js';
import { InMemoryStore } from '../../src/session/tree/store.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

const userMsg = (text: string): AgentMessage => ({
  role: 'user',
  content: [{ type: 'text', text }],
  timestamp: 0,
});

const functionResultMsg = (output: string): AgentMessage => ({
  role: 'function_result',
  function_call_id: 'call-1',
  function_id: 'my_tool',
  content: [{ type: 'text', text: output }],
  details: { original: true },
  is_error: false,
  timestamp: 0,
});

describe('updatePart', () => {
  it('replaces content on a function_result entry', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const entryId = await appendMessage(store, sid, null, functionResultMsg('original output'));

    await updatePart(store, sid, entryId, {
      output: 'compacted output',
      compacted_at: '2024-01-01',
    });

    const entries = await store.loadEntries(sid);
    const updated = entries.find((e) => e.id === entryId);
    expect(updated?.type).toBe('message');
    if (updated?.type === 'message' && updated.message.role === 'function_result') {
      expect(updated.message.content).toEqual([{ type: 'text', text: 'compacted output' }]);
      expect((updated.message.details as Record<string, unknown>).compacted_at).toBe('2024-01-01');
      // Original detail fields are preserved
      expect((updated.message.details as Record<string, unknown>).original).toBe(true);
    }
  });

  it('uses [output pruned] fallback when output is null', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const entryId = await appendMessage(store, sid, null, functionResultMsg('original output'));

    await updatePart(store, sid, entryId, { output: null, compacted_at: null });

    const entries = await store.loadEntries(sid);
    const updated = entries.find((e) => e.id === entryId);
    if (updated?.type === 'message' && updated.message.role === 'function_result') {
      expect(updated.message.content).toEqual([{ type: 'text', text: '[output pruned]' }]);
    }
  });

  it('is a no-op for non-function_result message entries', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const entryId = await appendMessage(store, sid, null, userMsg('user text'));

    // Should not throw and should not modify the entry
    await expect(
      updatePart(store, sid, entryId, { output: 'ignored', compacted_at: null }),
    ).resolves.toBeUndefined();

    const entries = await store.loadEntries(sid);
    const entry = entries.find((e) => e.id === entryId);
    if (entry?.type === 'message' && entry.message.role === 'user') {
      expect(entry.message.content).toEqual([{ type: 'text', text: 'user text' }]);
    }
  });

  it('is a no-op when entry_id does not exist', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);

    await expect(
      updatePart(store, sid, 'nonexistent-id', { output: 'x', compacted_at: null }),
    ).resolves.toBeUndefined();
  });
});
