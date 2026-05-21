import { describe, expect, it } from 'vitest';
import {
  appendMessage,
  appendSynthetic,
  createSession,
  loadMessages,
} from '../../src/session/tree/operations.js';
import { InMemoryStore } from '../../src/session/tree/store.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

const userMsg = (text: string): AgentMessage => ({
  role: 'user',
  content: [{ type: 'text', text }],
  timestamp: 0,
});

describe('appendSynthetic', () => {
  it('appends a user-role message entry with the given text', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const entryId = await appendSynthetic(store, sid, { text: 'synthetic message' });
    expect(typeof entryId).toBe('string');
    const messages = await loadMessages(store, sid);
    expect(messages).toHaveLength(1);
    const msg = messages[0];
    expect(msg?.role).toBe('user');
    if (msg?.role === 'user') {
      expect(msg.content).toEqual([{ type: 'text', text: 'synthetic message' }]);
    }
  });

  it('uses provided parent_id to thread into the conversation tree', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e1 = await appendMessage(store, sid, null, userMsg('first'));
    const e2 = await appendSynthetic(store, sid, { text: 'synthetic reply', parent_id: e1 });
    const entries = await store.loadEntries(sid);
    const syntheticEntry = entries.find((e) => e.id === e2);
    expect(syntheticEntry?.parent_id).toBe(e1);
  });

  it('metadata is accepted but does not cause errors (discarded silently)', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    // Should not throw even with metadata provided
    await expect(
      appendSynthetic(store, sid, { text: 'with meta', metadata: { foo: 'bar' } }),
    ).resolves.toBeDefined();
  });

  it('defaults parent_id to null when not provided', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const entryId = await appendSynthetic(store, sid, { text: 'root message' });
    const entries = await store.loadEntries(sid);
    const entry = entries.find((e) => e.id === entryId);
    expect(entry?.parent_id).toBeNull();
  });
});
