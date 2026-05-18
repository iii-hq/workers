import { describe, expect, it } from 'vitest';
import {
  activePath,
  appendMessage,
  cloneSession,
  createSession,
  exportHtml,
  fork,
  loadMessages,
  loadMessagesWithEntryIds,
  reconcile,
  tree,
} from '../../src/session/tree/operations.js';
import { InMemoryStore } from '../../src/session/tree/store.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

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

  it('loadMessages filters non-message entries from active path', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const e1 = await appendMessage(store, sid, null, userMsg('a'));
    await appendMessage(store, sid, e1, asstMsg('b'));
    const messages = await loadMessages(store, sid);
    expect(messages).toHaveLength(2);
    expect((messages[0] as { content: Array<{ text: string }> }).content[0].text).toBe('a');
  });

  it('reconcile appends missing tail messages', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    await appendMessage(store, sid, null, userMsg('one'));
    const result = await reconcile(store, sid, [userMsg('one'), asstMsg('two'), userMsg('three')]);
    expect(result.repaired).toBe(2);
    const after = await loadMessages(store, sid);
    expect(after).toHaveLength(3);
  });

  it('reconcile is idempotent', async () => {
    const store = new InMemoryStore();
    const sid = await createSession(store);
    const snap = [userMsg('a'), asstMsg('b')];
    await reconcile(store, sid, snap);
    const second = await reconcile(store, sid, snap);
    expect(second.repaired).toBe(0);
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
