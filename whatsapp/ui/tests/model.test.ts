import { describe, expect, it } from 'vitest'
import {
  ACTIVITY_LIMIT, DEFAULTS, EMPTY_FEED, configNumber, eventDetail, feedForSession, isOpaqueField,
  mergeEvents, normalizeRecipient, pairingIsCurrent, pairingPhone, requestScope, validateMessage,
  withConfigField, withDatabaseField, type Session, type WorkerEvent,
} from '../src/model'

const event = (sequence: number, overrides: Partial<WorkerEvent> = {}): WorkerEvent => ({
  session_id: 'support', sequence, kind: 'message.received', timestamp_ms: 1000 + sequence, data: {}, ...overrides,
})
const session = (created_at_ms: number): Session => ({ session_id: 'support', created_at_ms, status: 'pairing' })

describe('recipient addressing matches the native worker contract', () => {
  it.each([
    ['+5511999999999', '5511999999999@s.whatsapp.net'],
    ['5511999999999', '5511999999999@s.whatsapp.net'],
    ['+1 (555) 123-4567', '15551234567@s.whatsapp.net'],
    [' +55 (11) 99999-9999 ', '5511999999999@s.whatsapp.net'],
    ['0 123-456', '0123456@s.whatsapp.net'],
    ['+123 456 789 012 345', '123456789012345@s.whatsapp.net'],
    [' 5511999999999@c.us ', '5511999999999@s.whatsapp.net'],
    ['123456789:2@c.us', '123456789:2@s.whatsapp.net'],
    ['0001:0@lid', '0001:0@lid'],
    ['123456789:2@lid', '123456789:2@lid'],
    ['123456789:65535@s.whatsapp.net', '123456789:65535@s.whatsapp.net'],
    ['120363000000000000@g.us', '120363000000000000@g.us'],
    ['1234-5678@g.us', '1234-5678@g.us'],
    ['123456789@newsletter', '123456789@newsletter'],
    ['status@broadcast', 'status@broadcast'],
    ['12345@broadcast', '12345@broadcast'],
  ])('normalizes %s without changing an opaque identity', (input, expected) => {
    expect(normalizeRecipient(input)).toBe(expected)
  })
  it.each(['', '123456', '+1234567890123456', '+12 (34) 56', '+123 456 789 012 3456', '+1 (555) 123-4567 ext 9', '+1.555.123.4567', '++15551234567', '1+5551234567', '1555/1234567', '123@unknown', 'a@lid', '@g.us', '1--2@g.us', '12@lid@evil', '1:65536@lid', '1:-1@lid', '1:2:3@lid', 'status@s.whatsapp.net', '+1 (555) 123-4567@s.whatsapp.net', '1234 - 5678@g.us'])('rejects %s before sending', (input) => {
    expect(() => normalizeRecipient(input)).toThrow()
  })
  it.each([
    ['+5511999999999', '5511999999999'],
    ['+1 (555) 123-4567', '15551234567'],
    [' +55 (11) 99999-9999 ', '5511999999999'],
    ['0 123-456', '0123456'],
    ['+123 456 789 012 345', '123456789012345'],
  ])('normalizes phone pairing %s to account digits', (input, expected) => {
    expect(pairingPhone(input)).toBe(expected)
  })
  it.each(['', '+12 (34) 56', '+123 456 789 012 3456', '+1 (555) 123-4567 ext 9', '+1.555.123.4567', '++15551234567', '1+5551234567', '1555/1234567', '5511999999999@s.whatsapp.net', '123456789:2@c.us', '0001:0@lid', '1234-5678@g.us', '123456789@newsletter', 'status@broadcast'])('rejects invalid phone pairing or recipient JID %s', (input) => {
    expect(() => pairingPhone(input)).toThrow()
  })
  it('measures text limits in UTF-8 bytes and rejects a blank message', () => {
    expect(validateMessage('  \n ')).toBeTruthy()
    expect(validateMessage('á'.repeat(32768))).toBeNull()
    expect(validateMessage('á'.repeat(32769))).toContain('65,536')
  })
})

describe('bounded event reconciliation', () => {
  it('deduplicates overlapping polls, orders events and ignores another session', () => {
    const first = mergeEvents(EMPTY_FEED, { events: [event(2), event(1)], next_sequence: 2, truncated: false }, 'support')
    const next = mergeEvents(first, { events: [event(2), event(3), event(4, { session_id: 'other' })], next_sequence: 4, truncated: false }, 'support')
    expect(next.events.map((item) => item.sequence)).toEqual([1, 2, 3])
    expect(next.cursor).toBe(4)
    expect(first.events.map((item) => item.sequence)).toEqual([1, 2])
  })
  it('advances to the server cursor even when the page has no events', () => {
    expect(mergeEvents(EMPTY_FEED, { events: [], next_sequence: 4096, truncated: true }, 'support')).toEqual({ events: [], cursor: 4096, truncated: true })
  })
  it('retains the newest visible window without losing the polling cursor', () => {
    const next = mergeEvents(EMPTY_FEED, { events: Array.from({ length: 240 }, (_, n) => event(n + 1)), next_sequence: 240, truncated: false }, 'support')
    expect(next.events).toHaveLength(ACTIVITY_LIMIT)
    expect(next.events[0].sequence).toBe(141)
    expect(next.cursor).toBe(240)
  })
  it('keeps the retention-gap warning after subsequent successful polls', () => {
    const first = mergeEvents(EMPTY_FEED, { events: [event(10)], next_sequence: 10, truncated: true }, 'support')
    expect(mergeEvents(first, { events: [event(11)], next_sequence: 11, truncated: false }, 'support').truncated).toBe(true)
  })
  it('resets a recreated session before requesting its first page even if the server echoes the old cursor', () => {
    const previous = { events: [event(900)], cursor: 900, truncated: false }
    const reset = feedForSession(previous, session(1000), session(2000))
    expect(reset).toEqual({ events: [], cursor: 0, truncated: true })
    expect(mergeEvents(reset, { events: [event(1)], next_sequence: 1, truncated: false }, 'support').events.map((item) => item.sequence)).toEqual([1])
    expect(feedForSession(previous, session(1000), session(1000))).toBe(previous)
  })
  it('does not confuse a connection transition with a new session', () => {
    const previous = { events: [event(10)], cursor: 10, truncated: false }
    expect(feedForSession(previous, session(1000), { ...session(1000), status: 'connected' })).toBe(previous)
  })
})

describe('activity presentation', () => {
  it('reads the snake_case extended text emitted by the Rust protobuf serializer', () => {
    expect(eventDetail(event(1, { data: { info: { id: 'ABC', source: { sender: '123@lid' } }, message: { extended_text_message: { text: 'Resposta de teste' } } } }))).toBe('123@lid · Resposta de teste · ABC')
  })
  it('preserves routing context when an oversized message body was omitted', () => {
    expect(eventDetail(event(1, { data: { info: { id: 'ABC', source: { sender: '123@lid' } }, content_omitted: true } }))).toContain('123@lid · Content omitted because of the size limit. · ABC')
  })
  it('does not display pairing material in an event row', () => {
    expect(eventDetail(event(1, { kind: 'session.qr', data: { code: 'secret', expires_at_ms: 1000 } }))).toBe('')
    expect(eventDetail(event(1, { kind: 'session.pair_code', data: { code: 'secret' } }))).toBe('')
  })
})

describe('configuration drafts remain host owned', () => {
  it('edits the database without dropping Cloud credentials or nested templates', () => {
    const draft = { cloud: { access_token: '${WHATSAPP_CLOUD_ACCESS_TOKEN}' }, database: { name: '${DB_NAME:whatsapp}', namespace: 'one', future: true } }
    expect(withDatabaseField(draft, 'namespace', 'two')).toEqual({ ...draft, database: { ...draft.database, namespace: 'two' } })
    expect(draft.database.namespace).toBe('one')
    expect(withDatabaseField(draft, 'namespace', undefined)).toEqual({ ...draft, database: { name: '${DB_NAME:whatsapp}', future: true } })
  })
  it('preserves an opaque database setting until explicitly replaced', () => {
    const draft = { database: '${DATABASE_CONFIG}' }
    expect(withDatabaseField(draft, 'name', 'other')).toBe(draft)
  })
  it('updates one field without dropping templates or unknown siblings', () => {
    const draft = { max_sessions: '${MAX_SESSIONS}', data_dir: '${DATA_DIR}/wa', future: { enabled: true } }
    const next = withConfigField(draft, 'operation_timeout_secs', 90)
    expect(next).toEqual({ ...draft, operation_timeout_secs: 90 })
    expect(draft).not.toHaveProperty('operation_timeout_secs')
  })
  it('clearing a field deletes the override without materializing other defaults', () => {
    expect(withConfigField({ max_events: 5, future: 1 }, 'max_events', undefined)).toEqual({ future: 1 })
    expect(withConfigField({}, 'max_sessions', undefined)).toEqual({})
    expect(DEFAULTS.max_media_bytes).toBe(16777216)
  })
  it.each(['${WHATSAPP_CONFIG}', null, 3, [1, 2]])('keeps an opaque root unchanged until an explicit replacement: %s', (draft) => {
    expect(withConfigField(draft, 'max_sessions', 32)).toBe(draft)
  })
  it('recognizes environment templates and custom typed values', () => {
    expect(isOpaqueField('data_dir', '${DATA_DIR}/whatsapp')).toBe(true)
    expect(isOpaqueField('max_sessions', '${MAX_SESSIONS}')).toBe(true)
    expect(isOpaqueField('max_sessions', { future: true })).toBe(true)
    expect(isOpaqueField('max_sessions', null)).toBe(true)
    expect(isOpaqueField('max_sessions', 8)).toBe(false)
    expect(isOpaqueField('data_dir', 'data/whatsapp')).toBe(false)
    expect(isOpaqueField('max_events', undefined)).toBe(false)
  })
  it('never writes NaN and keeps invalid finite ranges for host schema validation', () => {
    expect(configNumber('')).toBeUndefined()
    expect(configNumber('NaN')).toBeUndefined()
    expect(configNumber('Infinity')).toBeUndefined()
    expect(configNumber('0')).toBe(0)
    expect(configNumber('1.5')).toBe(1.5)
  })
})

describe('pairing validity and async ownership', () => {
  it('hides a code at expiry, after connecting or when stopped', () => {
    const pair = { code: 'secret', expires_at_ms: 2000 }
    expect(pairingIsCurrent(pair, 'pairing', 1999)).toBe(true)
    expect(pairingIsCurrent(pair, 'pairing', 2000)).toBe(false)
    expect(pairingIsCurrent(pair, 'connected', 1000)).toBe(false)
    expect(pairingIsCurrent(pair, 'stopped', 1000)).toBe(false)
    expect(pairingIsCurrent(null, 'pairing', 1000)).toBe(false)
  })
  it('rejects a late pairing response after phone edits, navigation or unmount', async () => {
    const scope = requestScope()
    let finish!: (value: string) => void
    const response = new Promise<string>((resolve) => { finish = resolve })
    const current = scope.begin()
    let displayed: string | null = null
    const pending = response.then((code) => { if (current()) displayed = code })
    scope.invalidate()
    finish('old-code')
    await pending
    expect(displayed).toBeNull()
  })
  it('a newer operation supersedes an older response, independently per pane', () => {
    const a = requestScope()
    const b = requestScope()
    const oldA = a.begin()
    const currentB = b.begin()
    const currentA = a.begin()
    expect(oldA()).toBe(false)
    expect(currentA()).toBe(true)
    expect(currentB()).toBe(true)
  })
})
