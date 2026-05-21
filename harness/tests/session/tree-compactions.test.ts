import { describe, expect, it } from 'vitest';
import {
  appendMessage,
  compact,
  compactionEntries,
  createSession,
} from '../../src/session/tree/operations.js';
import { InMemoryStore } from '../../src/session/tree/store.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

const userMsg = (text: string): AgentMessage => ({
  role: 'user',
  content: [{ type: 'text', text }],
  timestamp: 0,
});

describe('compactionEntries', () => {
  it('returns an empty array for a session with no compaction entries', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    await appendMessage(store, sid, null, userMsg('hello'));
    const result = await compactionEntries(store, sid);
    expect(result).toEqual([]);
  });

  it('returns all compaction entries sorted by timestamp ascending', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e1 = await appendMessage(store, sid, null, userMsg('a'));
    const id1 = await compact(store, sid, 'first compaction', { read_files: ['a.ts'] }, e1, 200);
    const id2 = await compact(store, sid, 'second compaction', {}, null, 400);
    const result = await compactionEntries(store, sid);
    expect(result).toHaveLength(2);
    expect(result[0]?.id).toBe(id1);
    expect(result[1]?.id).toBe(id2);
  });

  it('entry shape includes expected fields', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    await compact(store, sid, 'my summary', { modified_files: ['b.ts'] }, null, 150, 'tail-xyz');
    const result = await compactionEntries(store, sid);
    expect(result).toHaveLength(1);
    const entry = result[0];
    expect(entry?.summary).toBe('my summary');
    expect(entry?.tokens_before).toBe(150);
    expect(entry?.tail_start_id).toBe('tail-xyz');
    expect(entry?.details.modified_files).toEqual(['b.ts']);
    expect(typeof entry?.timestamp).toBe('number');
    expect(typeof entry?.id).toBe('string');
  });
});
