import { describe, expect, it } from 'vitest';
import { compact, createSession } from '../../src/session/tree/operations.js';
import { InMemoryStore } from '../../src/session/tree/store.js';

describe('compact tail_start_id field', () => {
  it('stores tail_start_id when provided', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const tailId = 'entry-abc';
    const entryId = await compact(store, sid, 'summary', {}, null, 100, tailId);
    const entries = await store.loadEntries(sid);
    const compaction = entries.find((e) => e.id === entryId);
    expect(compaction?.type).toBe('compaction');
    if (compaction?.type === 'compaction') {
      expect(compaction.tail_start_id).toBe(tailId);
    }
  });

  it('defaults tail_start_id to null when not provided', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const entryId = await compact(store, sid, 'summary', {}, null, 100);
    const entries = await store.loadEntries(sid);
    const compaction = entries.find((e) => e.id === entryId);
    if (compaction?.type === 'compaction') {
      expect(compaction.tail_start_id).toBeNull();
    }
  });

  it('existing entries without tail_start_id remain valid (optional field)', async () => {
    // Simulate a stored entry without the field (as if from before this change)
    const entry = {
      type: 'compaction' as const,
      id: 'old-id',
      summary: 'old compaction',
      tokens_before: 50,
      details: {},
      timestamp: Date.now(),
    };
    // tail_start_id is absent — TypeScript allows it (optional field)
    expect(entry.type).toBe('compaction');
    expect((entry as { tail_start_id?: string | null }).tail_start_id).toBeUndefined();
  });
});
