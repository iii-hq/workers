import { describe, expect, it } from 'vitest'
import { coverageText, mergeChats, mergeMessages, messageLabel, type MessageView } from '../src/inbox-model'

const message = (id: string, timestamp_ms: number, overrides: Partial<MessageView> = {}): MessageView => ({
  message_id: id, chat_id: '123@s.whatsapp.net', sender_id: '123@s.whatsapp.net',
  from_me: false, timestamp_ms, kind: 'text', text: `Mensagem ${id}`, source: 'live', ...overrides,
})

describe('message pages and live refresh', () => {
  it('renders newest-first API pages chronologically and deduplicates overlapping pages', () => {
    const initial = mergeMessages([], [message('c', 3), message('b', 2)], '123@s.whatsapp.net')
    expect(mergeMessages(initial, [message('b', 2), message('a', 1)], '123@s.whatsapp.net').map((item) => item.message_id)).toEqual(['a', 'b', 'c'])
  })
  it('replaces a receipt update without dropping older pages or accepting a different chat', () => {
    const original = [message('a', 1), message('b', 2, { from_me: true, status: 'sent' })]
    const next = mergeMessages(original, [message('b', 2, { from_me: true, status: 'read' }), message('c', 3, { chat_id: 'other@g.us' })], '123@s.whatsapp.net')
    expect(next).toHaveLength(2)
    expect(next[1].status).toBe('read')
    expect(original[1].status).toBe('sent')
  })
  it('keeps distinct incoming and outgoing identities and stable timestamp ties', () => {
    expect(mergeMessages([], [message('same', 1), message('same', 1, { from_me: true })], '123@s.whatsapp.net')).toHaveLength(2)
  })
  it('keeps two group senders with the same message id, and reconciles the own-device echo', () => {
    const next = mergeMessages([], [message('same', 1, { sender_id: 'a@lid' }), message('same', 1, { sender_id: 'b@lid' })], '123@s.whatsapp.net')
    expect(next).toHaveLength(2)
    expect(mergeMessages([message('mine', 1, { from_me: true, sender_id: 'me', source: 'sent' })], [message('mine', 1, { from_me: true, sender_id: '15555555555@s.whatsapp.net', source: 'live' })], '123@s.whatsapp.net')).toHaveLength(1)
  })
})

describe('conversation summaries', () => {
  it('updates live previews, keeps previously loaded pages, and orders newest first', () => {
    const a = { chat_id: 'a', title: 'Ana', is_group: false, message_count: 2, last_message: message('a', 1) }
    const b = { chat_id: 'b', title: 'Equipe', is_group: true, message_count: 4, last_message: message('b', 3) }
    const next = mergeChats([a, b], [{ ...a, message_count: 3, last_message: message('a2', 4) }])
    expect(next.map((chat) => chat.chat_id)).toEqual(['a', 'b'])
    expect(next[0].message_count).toBe(3)
  })
  it('labels media without inventing an attachment or a download', () => {
    expect(messageLabel(message('a', 1, { kind: 'image', text: '' }))).toContain('Imagem')
    expect(messageLabel(message('a', 1, { kind: 'document', text: 'Relatório.pdf' }))).toBe('Relatório.pdf')
  })
  it.each([['expired', 'expirada'], ['view_once', 'visualização única'], ['deleted', 'removida'], ['ephemeral', 'temporária']])('labels %s without fabricating a body', (kind, label) => {
    expect(messageLabel(message('a', 1, { kind, text: '' }))).toContain(label)
  })
})

describe('honest coverage copy', () => {
  it('does not imply complete WhatsApp history before any history sync', () => {
    const copy = coverageText({ capture_started_at_ms: 1700000000000, message_limit: 10000, retention_truncated: false })
    expect(copy).toContain('capturadas')
    expect(copy).toContain('histórico')
    expect(copy).not.toContain('completo')
  })
  it('shows retention loss separately from received history', () => {
    expect(coverageText({ capture_started_at_ms: 1, message_limit: 100, retention_truncated: true, last_history_sync_progress: 50 })).toContain('mais antigas')
  })
})
