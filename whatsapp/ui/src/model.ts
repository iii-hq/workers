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
  if (!input || input.length > 128) throw new Error('Informe um telefone internacional ou JID válido.')
  if (!input.includes('@')) {
    const phone = input.replace(/^\+/, '')
    if (!/^[0-9]{7,15}$/.test(phone)) throw new Error('Use de 7 a 15 dígitos, com código do país, sem espaços. Ex.: +5511999999999.')
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
  if (!valid || extra !== undefined) throw new Error('JID inválido. Use um telefone internacional ou um JID de contato, grupo, LID, canal ou transmissão.')
  return `${user}@${server === 'c.us' ? 's.whatsapp.net' : server}`
}

export function pairingPhone(raw: string): string {
  const phone = raw.trim().replace(/^\+/, '')
  if (!/^[0-9]{7,15}$/.test(phone)) throw new Error('Informe o telefone da conta com código do país e 7 a 15 dígitos, sem espaços.')
  return phone
}

export function validateMessage(text: string): string | null {
  if (!text.trim()) return 'Escreva a mensagem de teste.'
  if (new TextEncoder().encode(text).length > 65_536) return 'A mensagem excede o limite de 65.536 bytes.'
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
    return [source.sender, body?.slice(0, 240) ?? (data.content_omitted ? 'Conteúdo omitido pelo limite de tamanho.' : 'Mensagem sem texto.'), info.id].filter((item) => typeof item === 'string' && item.length > 0).join(' · ')
  }
  // Pairing events contain expiry metadata only. Never display a QR or code as event data.
  if (event.kind.startsWith('session.')) return typeof data.reason === 'string' ? data.reason : ''
  return JSON.stringify(data).slice(0, 400)
}

export const STATUS_LABEL: Record<SessionStatus, string> = {
  stopped: 'Parada', starting: 'Iniciando', pairing: 'Aguardando pareamento', connected: 'Conectada',
  disconnected: 'Desconectada', stopping: 'Parando', logged_out: 'Desvinculada', failed: 'Falha',
}

export const DEFAULTS = {
  data_dir: 'data/whatsapp', max_sessions: 32, max_media_bytes: 16 * 1024 * 1024,
  max_events: 1000, max_inbox_messages: 10000, operation_timeout_secs: 30, shutdown_timeout_secs: 15,
} as const
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
export function isOpaqueField(key: ConfigKey, value: JsonValue | undefined): boolean {
  if (value === undefined) return false
  return typeof value !== typeof DEFAULTS[key] || (typeof value === 'string' && value.includes('${'))
}
export function configNumber(raw: string): number | undefined {
  if (!raw.trim()) return undefined
  const value = Number(raw)
  return Number.isFinite(value) ? value : undefined
}
