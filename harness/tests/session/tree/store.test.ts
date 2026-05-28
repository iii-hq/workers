import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../../src/runtime/iii.js';
import { IiiStateSessionStore } from '../../../src/session/tree/store.js';
import type { SessionEntry } from '../../../src/session/tree/types.js';

function makeEntry(id: string, timestamp: number): SessionEntry {
  return {
    type: 'custom_message',
    id,
    custom_type: 'system',
    content: null,
    timestamp,
  };
}

function fakeIii(entries: SessionEntry[]): ISdk {
  return {
    trigger: async <T, R>(req: { function_id: string }): Promise<R> => {
      if (req.function_id === 'state::list') {
        return entries as unknown as R;
      }
      return null as unknown as R;
    },
  } as unknown as ISdk;
}

describe('IiiStateSessionStore.loadEntries', () => {
  it('sorts entries by (timestamp, id) so non-monotonic ids still appear in temporal order', async () => {
    // Append order: e-b first (older t), then e-a (newer t).
    // Old id-only sort would return [e-a, e-b]; PR #150 sort returns [e-b, e-a].
    const stored = [makeEntry('e-a', 200), makeEntry('e-b', 100)];
    const store = new IiiStateSessionStore(fakeIii(stored));
    const out = await store.loadEntries('s1');
    expect(out.map((e) => e.id)).toEqual(['e-b', 'e-a']);
  });

  it('breaks ties on equal timestamps by id', async () => {
    const stored = [makeEntry('e-z', 100), makeEntry('e-a', 100)];
    const store = new IiiStateSessionStore(fakeIii(stored));
    const out = await store.loadEntries('s1');
    expect(out.map((e) => e.id)).toEqual(['e-a', 'e-z']);
  });
});
