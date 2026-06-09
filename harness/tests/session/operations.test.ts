import { describe, expect, it, vi } from 'vitest';
import {
  activePath,
  appendMessage,
  appendMessages,
  cloneSession,
  createSession,
  exportHtml,
  fork,
  loadMessages,
  loadMessagesWithEntryIds,
  tree,
} from '../../src/session/tree/operations.js';
import { InMemoryStore } from '../../src/session/tree/store.js';
import { entryTimestamp } from '../../src/session/tree/types.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

/**
 * InMemoryStore returns entries in insertion order, but the production
 * IiiStateSessionStore sorts loadEntries by (timestamp, id). Leaf-resolution
 * bugs only surface under that sort, so this double reproduces it.
 */
class SortingStore extends InMemoryStore {
  async loadEntries(session_id: string) {
    const entries = await super.loadEntries(session_id);
    return entries.sort((a, b) => {
      const t = entryTimestamp(a) - entryTimestamp(b);
      if (t !== 0) return t;
      return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
    });
  }
}

const userMsg = (text: string): AgentMessage => ({
  role: 'user',
  content: [{ type: 'text', text }],
  timestamp: 0,
});

const asstMsg = (text: string): AgentMessage => ({
  role: 'assistant',
  content: [{ type: 'text', text }],
  stop_reason: 'end',
  model: 'm',
  provider: 'p',
  timestamp: 0,
});

describe('session-tree operations', () => {
  it('create + append + activePath threads parent ids', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store, 'demo');
    const e1 = await appendMessage(store, sid, null, userMsg('hi'));
    const e2 = await appendMessage(store, sid, e1, asstMsg('hello'));
    const path = await activePath(store, sid);
    expect(path).toEqual([e1, e2]);
  });

  it('append with null parent_id chains to the active tip', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e1 = await appendMessage(store, sid, null, userMsg('a'));
    const e2 = await appendMessage(store, sid, e1, asstMsg('b'));
    const e3 = await appendMessage(store, sid, null, userMsg('c'));
    const path = await activePath(store, sid);
    expect(path).toEqual([e1, e2, e3]);
  });

  it('appendMessages chains a batch and is equivalent to serial appends', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e0 = await appendMessage(store, sid, null, userMsg('seed'));

    const ids = await appendMessages(store, sid, null, [asstMsg('a'), userMsg('b'), asstMsg('c')]);

    expect(ids).toHaveLength(3);
    // The batch chains onto the existing leaf and links each entry to the prior.
    const path = await activePath(store, sid);
    expect(path).toEqual([e0, ...ids]);
    const messages = await loadMessages(store, sid);
    expect(messages.map((m) => (m.content[0] as { text: string }).text)).toEqual([
      'seed',
      'a',
      'b',
      'c',
    ]);
  });

  it('appendMessages resolves the active leaf once for the whole batch', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    await appendMessage(store, sid, null, userMsg('seed'));

    // loadEntries backs activePath; one call means the leaf was resolved once.
    const loadEntriesSpy = vi.spyOn(store, 'loadEntries');
    const appendManySpy = vi.spyOn(store, 'appendMany');

    await appendMessages(store, sid, null, [asstMsg('a'), userMsg('b'), asstMsg('c')]);

    expect(loadEntriesSpy).toHaveBeenCalledTimes(1);
    expect(appendManySpy).toHaveBeenCalledTimes(1);
    expect(appendManySpy.mock.calls[0]?.[1]).toHaveLength(3);
  });

  it('appendMessages with an explicit parent skips leaf resolution', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e0 = await appendMessage(store, sid, null, userMsg('seed'));
    const loadEntriesSpy = vi.spyOn(store, 'loadEntries');

    const ids = await appendMessages(store, sid, e0, [asstMsg('a')]);

    expect(loadEntriesSpy).not.toHaveBeenCalled();
    const path = await activePath(store, sid);
    expect(path).toEqual([e0, ...ids]);
  });

  it('appendMessages on an empty list is a no-op', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const appendManySpy = vi.spyOn(store, 'appendMany');

    const ids = await appendMessages(store, sid, null, []);

    expect(ids).toEqual([]);
    expect(appendManySpy).not.toHaveBeenCalled();
  });

  it('keeps a later append on the active path when its timestamp ties or precedes the batch (sorted store, leaf = chain tip)', async () => {
    const store = new SortingStore();
    const sid = await createSession(store);
    const now = vi.spyOn(Date, 'now');

    // Batch lands at t=100 (all entries share the append timestamp).
    now.mockReturnValue(100);
    const batch = await appendMessages(store, sid, null, [
      asstMsg('a'),
      userMsg('b'),
      asstMsg('c'),
    ]);

    // A later single append whose clock did NOT advance (or stepped back): its
    // timestamp sorts before the batch tail. Pre-fix, the sort-max leaf
    // heuristic orphaned it; the chain-tip leaf keeps it on the path.
    now.mockReturnValue(99);
    const tail = await appendMessage(store, sid, null, asstMsg('d'));
    now.mockRestore();

    expect(await activePath(store, sid)).toEqual([...batch, tail]);
    const texts = (await loadMessages(store, sid)).map(
      (m) => (m.content[0] as { text: string }).text,
    );
    expect(texts).toEqual(['a', 'b', 'c', 'd']);
  });

  it('loadMessages filters non-message entries from active path', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e1 = await appendMessage(store, sid, null, userMsg('a'));
    await appendMessage(store, sid, e1, asstMsg('b'));
    const messages = await loadMessages(store, sid);
    expect(messages).toHaveLength(2);
    expect((messages[0] as { content: Array<{ text: string }> }).content[0].text).toBe('a');
  });

  it('fork copies path entries with re-mapped ids', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store, 'orig');
    const e1 = await appendMessage(store, sid, null, userMsg('a'));
    const e2 = await appendMessage(store, sid, e1, asstMsg('b'));
    const newSid = await fork(store, sid, e2);
    const newMessages = await loadMessages(store, newSid);
    expect(newMessages).toHaveLength(2);
    const oldEntries = await store.loadEntries(sid);
    const newEntries = await store.loadEntries(newSid);
    const overlap = oldEntries.some((o) => newEntries.some((n) => n.id === o.id));
    expect(overlap).toBe(false);
  });

  it('clone duplicates every entry with re-mapped ids', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store, 'orig');
    const e1 = await appendMessage(store, sid, null, userMsg('a'));
    await appendMessage(store, sid, e1, asstMsg('b'));
    const newSid = await cloneSession(store, sid);
    const newEntries = await store.loadEntries(newSid);
    expect(newEntries).toHaveLength(2);
  });

  it('tree returns the nested structure rooted at parent-less entry', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e1 = await appendMessage(store, sid, null, userMsg('a'));
    await appendMessage(store, sid, e1, asstMsg('b'));
    const node = await tree(store, sid);
    expect(node.entry.id).toBe(e1);
    expect(node.children).toHaveLength(1);
  });

  it('exportHtml emits a self-contained document for the active path', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e1 = await appendMessage(store, sid, null, userMsg('hello world'));
    await appendMessage(store, sid, e1, asstMsg('hi'));
    const html = await exportHtml(store, sid);
    expect(html).toContain('<!DOCTYPE html>');
    expect(html).toContain('hello world');
    expect(html).toContain('class="entry user"');
  });

  it('loadMessagesWithEntryIds pairs each message with its entry id', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e1 = await appendMessage(store, sid, null, userMsg('hi'));
    const pairs = await loadMessagesWithEntryIds(store, sid);
    expect(pairs).toHaveLength(1);
    expect(pairs[0]?.entry_id).toBe(e1);
  });
});
