import type { JsonValue } from '@iii-dev/console-ui'

export type SessionStatus = 'stopped' | 'starting' | 'pairing' | 'connected' | 'disconnected' | 'stopping' | 'logged_out' | 'failed'
export interface Session {
  session_id: string
  status: SessionStatus
  created_at_ms: number
}
export interface PairingCode { code: string; expires_at_ms: number }
export interface WorkerEvent {
  session_id: string
  sequence: number
  kind: string
  timestamp_ms: number
  data: Record<string, unknown>
}
export interface EventPage { events: WorkerEvent[]; next_sequence: number; truncated: boolean }
export interface EventFeed { events: WorkerEvent[]; cursor: number; truncated: boolean }
export interface SentMessage { message_id: string; chat_id: string }
export const EMPTY_FEED: EventFeed = { events: [], cursor: 0, truncated: false }
export const ACTIVITY_LIMIT = 100

/** Deletion/recreation reuses a name, not an event cursor or account identity. */
export function feedForSession(feed: EventFeed, previous: Session | null, next: Session): EventFeed {
  return previous && (previous.session_id !== next.session_id || previous.created_at_ms !== next.created_at_ms)
    ? { ...EMPTY_FEED, truncated: true }
    : feed
}

/** A retained, deduplicated window; always advance using the server cursor. */
export function mergeEvents(previous: EventFeed, page: EventPage, sessionId: string): EventFeed {
  // A smaller server cursor means this session was deleted/recreated. Old events cannot belong to it.
  const reset = page.next_sequence < previous.cursor
  const events = new Map((reset ? [] : previous.events).map((event) => [event.sequence, event]))
  for (const event of page.events) {
    if (event.session_id === sessionId) events.set(event.sequence, event)
  }
  return {
    events: [...events.values()].sort((a, b) => a.sequence - b.sequence).slice(-ACTIVITY_LIMIT),
    cursor: page.next_sequence,
    truncated: previous.truncated || page.truncated || reset,
  }
}

/** Mirrors validate::jid: aliases normalize, device IDs and opaque LIDs survive. */
export function normalizeRecipient(raw: string): string {
  const input = raw.trim()
  if (!input || input.length > 128) throw new Error('Enter a valid international phone number or JID.')
  if (!input.includes('@')) {
    const phone = input.replace(/^\+/, '')
    if (!/^[0-9]{7,15}$/.test(phone)) throw new Error('Use 7 to 15 digits, including the country code, without spaces. Example: +5511999999999.')
    return `${phone}@s.whatsapp.net`
  }
  const [user, server, extra] = input.split('@')
  let valid = false
  if (['c.us', 's.whatsapp.net', 'lid'].includes(server)) {
    const [identity, device, excess] = user.split(':')
    valid = /^[0-9]+$/.test(identity) && excess === undefined
      && (device === undefined || (/^[0-9]+$/.test(device) && Number(device) <= 65535))
  } else if (server === 'g.us') valid = /^[0-9]+(?:-[0-9]+)?$/.test(user)
  else if (server === 'newsletter') valid = /^[0-9]+$/.test(user)
  else if (server === 'broadcast') valid = user === 'status' || /^[0-9]+$/.test(user)
  if (!valid || extra !== undefined) throw new Error('Invalid JID. Use an international phone number or a contact, group, LID, channel, or broadcast JID.')
  return `${user}@${server === 'c.us' ? 's.whatsapp.net' : server}`
}

export function pairingPhone(raw: string): string {
  const phone = raw.trim().replace(/^\+/, '')
  if (!/^[0-9]{7,15}$/.test(phone)) throw new Error('Enter the account phone number with its country code and 7 to 15 digits, without spaces.')
  return phone
}

export function validateMessage(text: string): string | null {
  if (!text.trim()) return 'Write a message.'
  if (new TextEncoder().encode(text).length > 65_536) return 'The message exceeds the 65,536-byte limit.'
  return null
}

export function pairingIsCurrent(pair: PairingCode | null, status: SessionStatus, now: number): pair is PairingCode {
  return status === 'pairing' && pair !== null && pair.expires_at_ms > now
}

/** Invalidated on edits, navigation and unmount; late results cannot reveal stale secrets. */
export function requestScope() {
  let epoch = 0
  return {
    begin() { const mine = ++epoch; return () => mine === epoch },
    invalidate() { epoch += 1 },
  }
}

export function eventDetail(event: WorkerEvent): string {
  const data = isObject(event.data) ? event.data : {}
  if (event.kind === 'message.received') {
    const info = isObject(data.info) ? data.info : {}
    const source = isObject(info.source) ? info.source : {}
    const message = isObject(data.message) ? data.message : {}
    const extended = isObject(message.extended_text_message) ? message.extended_text_message : {}
    const body = typeof message.conversation === 'string' ? message.conversation : typeof extended.text === 'string' ? extended.text : null
    return [source.sender, body?.slice(0, 240) ?? (data.content_omitted ? 'Content omitted because of the size limit.' : 'Message without text.'), info.id].filter((item) => typeof item === 'string' && item.length > 0).join(' · ')
  }
  // Pairing events contain expiry metadata only. Never display a QR or code as event data.
  if (event.kind.startsWith('session.')) return typeof data.reason === 'string' ? data.reason : ''
  return JSON.stringify(data).slice(0, 400)
}

export const STATUS_LABEL: Record<SessionStatus, string> = {
  stopped: 'Stopped', starting: 'Starting', pairing: 'Awaiting pairing', connected: 'Connected',
  disconnected: 'Disconnected', stopping: 'Stopping', logged_out: 'Unlinked', failed: 'Failed',
}

export const DEFAULTS = {
  data_dir: 'data/whatsapp-go', max_sessions: 32, max_media_bytes: 16 * 1024 * 1024,
  max_events: 1000, max_inbox_messages: 10000, operation_timeout_secs: 30, shutdown_timeout_secs: 15,
} as const
export const CLOUD_DEFAULTS = {
  enabled: false, api_version: 'v26.0', business_account_id: '', phone_number_id: '',
  access_token: '', app_secret: '', verify_token: '',
} as const
export type CloudConfigKey = keyof typeof CLOUD_DEFAULTS
export type ConfigKey = keyof typeof DEFAULTS
export interface ConfigStatus {
  id: string
  source: 'central' | 'local'
  last_outcome: 'applied' | 'restart_required' | 'rejected'
  last_error: string | null
  rejected_reloads: number
  restart_required_fields: string[]
  active: Record<ConfigKey, string | number>
}
export type JsonObject = { [key: string]: JsonValue }
export function isObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}
export function withConfigField(value: JsonValue, key: ConfigKey, next: JsonValue | undefined): JsonValue {
  if (!isObject(value)) return value
  const copy = { ...value }
  if (next === undefined) delete copy[key]
  else copy[key] = next
  return copy
}
/** Nested edits preserve future fields and never replace an opaque Cloud configuration. */
export function withCloudField(value: JsonValue, key: CloudConfigKey, next: JsonValue | undefined): JsonValue {
  if (!isObject(value) || (value.cloud !== undefined && !isObject(value.cloud))) return value
  const cloud = { ...(isObject(value.cloud) ? value.cloud : {}) }
  if (next === undefined) delete cloud[key]
  else cloud[key] = next
  return { ...value, cloud }
}
export function withCloudValue(value: JsonValue, next: JsonValue): JsonValue {
  return isObject(value) ? { ...value, cloud: next } : value
}
export function isOpaqueCloudField(key: CloudConfigKey, value: JsonValue | undefined): boolean {
  return value !== undefined && (typeof value !== typeof CLOUD_DEFAULTS[key] || (typeof value === 'string' && value.includes('${')))
}
export function isOpaqueField(key: ConfigKey, value: JsonValue | undefined): boolean {
  if (value === undefined) return false
  return typeof value !== typeof DEFAULTS[key] || (typeof value === 'string' && value.includes('${'))
}
export function configNumber(raw: string): number | undefined {
  if (!raw.trim()) return undefined
  const value = Number(raw)
  return Number.isFinite(value) ? value : undefined
}
