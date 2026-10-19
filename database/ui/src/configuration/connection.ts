/**
 * One connection entry (`databases.<name>`) as the form edits it.
 *
 * An entry comes in two shapes. The legacy one is a single `url` whose
 * scheme picks the driver; the structured one names the `driver` and spells
 * the endpoint out in fields (`host`, `port`, `user`, `password`,
 * `database`), with optional `tls`, `ssh`, `display` and `folder` blocks.
 * The worker accepts either and refuses both at once. Mirrors
 * `DatabaseConfig` in database/src/config.rs.
 *
 * Everything here is pure: it reads and returns JSON objects, so the form can
 * keep keys it does not know about and `${VAR}` templates it must not coerce.
 */

import type { JsonValue } from '@iii-dev/console-ui'
import { envRefName, secretRefName } from '../lib/secrets'
import { type Driver, driverOfConfiguredUrl, isEnvironmentValue } from './model'

export type JsonObject = { [key: string]: JsonValue }

export type ConnectionDriver = Exclude<Driver, 'unknown'>

export const CONNECTION_DRIVERS: readonly { value: ConnectionDriver; label: string }[] = [
  { value: 'postgres', label: 'PostgreSQL' },
  { value: 'mysql', label: 'MySQL' },
  { value: 'sqlite', label: 'SQLite' },
]

export const DEFAULT_PORTS: Readonly<Record<'postgres' | 'mysql', number>> = {
  postgres: 5432,
  mysql: 3306,
}

export const DEFAULT_SSH_PORT = 22

/** What `database::getConnection` puts where a stored literal secret was. */
export const REDACTED = '__redacted__'

function isObject(v: JsonValue | undefined): v is JsonObject {
  return v !== null && typeof v === 'object' && !Array.isArray(v)
}

function str(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

export function isConnectionDriver(v: unknown): v is ConnectionDriver {
  return v === 'postgres' || v === 'mysql' || v === 'sqlite'
}

/* ---------------- shape ---------------- */

/** `structured` once the entry names a driver; a bare `url` is the legacy shape. */
export function connectionShape(db: JsonObject): 'url' | 'structured' {
  if (db.driver !== undefined) return 'structured'
  return db.url !== undefined ? 'url' : 'structured'
}

/** The driver an entry connects with, whichever shape it has. */
export function connectionDriver(db: JsonObject): Driver {
  if (connectionShape(db) === 'url') return driverOfConfiguredUrl(str(db.url))
  return isConnectionDriver(db.driver) ? db.driver : 'unknown'
}

/** Whether the TLS and SSH blocks apply: network drivers, or an unknown one. */
export function isNetworkDriver(driver: Driver): boolean {
  return driver !== 'sqlite'
}

/**
 * Whether this worker's configuration schema knows the structured shape.
 * Detected from the JSON Schema the worker registered rather than assumed:
 * writing `driver:` to an older worker would fail its validation on save.
 * No schema at all is not evidence against it.
 */
export function supportsStructuredConnections(schema: Record<string, unknown> | null | undefined): boolean {
  if (!schema) return true
  let found = false
  const walk = (node: unknown, depth: number) => {
    if (found || depth > 12 || node === null || typeof node !== 'object') return
    if (Array.isArray(node)) {
      for (const item of node) walk(item, depth + 1)
      return
    }
    const rec = node as Record<string, unknown>
    const props = rec.properties
    if (props && typeof props === 'object' && !Array.isArray(props)) {
      const keys = Object.keys(props)
      if (keys.includes('driver') && keys.includes('ssh')) {
        found = true
        return
      }
    }
    for (const value of Object.values(rec)) walk(value, depth + 1)
  }
  walk(schema, 0)
  return found
}

/** A fresh entry for the driver picked in "Add database". */
export function newConnectionEntry(driver: ConnectionDriver, name: string): JsonObject {
  if (driver === 'sqlite') return { driver, database: `./data/${name}.db` }
  return { driver, host: 'localhost', database: name }
}

/** Keys that mean nothing to a file database. */
const NETWORK_KEYS = ['host', 'port', 'user', 'password', 'tls', 'ssh'] as const

/**
 * Switch an entry's driver. Moving to SQLite sets the network keys aside in
 * `stash` rather than deleting them, so flipping back within the same edit
 * restores what was typed; a port equal to the old driver's default is
 * dropped so it does not pin the new driver to the wrong one.
 */
export function switchDriver(
  db: JsonObject,
  next: ConnectionDriver,
  stash: JsonObject = {},
): { entry: JsonObject; stash: JsonObject } {
  const entry: JsonObject = { ...db, driver: next }
  const prev = connectionDriver(db)
  let kept: JsonObject = { ...stash }
  if (next === 'sqlite') {
    for (const key of NETWORK_KEYS) {
      if (entry[key] !== undefined) {
        kept[key] = entry[key]
        delete entry[key]
      }
    }
    return { entry, stash: kept }
  }
  if (prev === 'sqlite') {
    for (const key of NETWORK_KEYS) {
      if (kept[key] !== undefined && entry[key] === undefined) entry[key] = kept[key]
    }
    kept = {}
    if (entry.host === undefined) entry.host = 'localhost'
  }
  if (prev === 'postgres' || prev === 'mysql') {
    if (entry.port === DEFAULT_PORTS[prev]) delete entry.port
  }
  return { entry, stash: kept }
}

/* ---------------- url ↔ fields ---------------- */

export interface UrlFields {
  driver: ConnectionDriver
  host?: string
  port?: number
  user?: string
  password?: string
  database?: string
}

export type UrlParse = { ok: true; fields: UrlFields } | { ok: false; reason: string }

/**
 * Take a legacy url apart into the structured fields, or say why it cannot
 * be: a template (`${…}`), query parameters (the fields have nowhere to keep
 * them), several hosts, or an unknown scheme. Conversion never guesses —
 * a url that would lose something stays a url.
 */
export function parseConnectionUrl(url: string): UrlParse {
  const trimmed = url.trim()
  if (!trimmed) return { ok: false, reason: 'Enter a URL first.' }
  if (trimmed.includes('${')) return { ok: false, reason: 'The URL is filled from the environment.' }
  const driver = driverOfConfiguredUrl(trimmed)
  if (driver === 'unknown') return { ok: false, reason: 'Unknown URL scheme.' }

  if (driver === 'sqlite') {
    let path = trimmed.slice('sqlite:'.length)
    if (path.startsWith('//')) path = path.slice(2)
    if (path.includes('?')) return { ok: false, reason: 'The URL carries query parameters.' }
    if (/^file:/i.test(path)) return { ok: false, reason: 'file: URLs stay as a URL.' }
    if (!path) return { ok: false, reason: 'The URL names no file.' }
    return { ok: true, fields: { driver, database: path } }
  }

  let parsed: URL
  try {
    parsed = new URL(trimmed)
  } catch {
    return { ok: false, reason: 'The URL cannot be parsed.' }
  }
  if (parsed.search) return { ok: false, reason: 'The URL carries query parameters.' }
  if (parsed.hash) return { ok: false, reason: 'The URL carries a fragment.' }
  const host = parsed.hostname.replace(/^\[(.*)\]$/, '$1')
  if (!host) return { ok: false, reason: 'The URL names no host.' }
  if (host.includes(',')) return { ok: false, reason: 'The URL names several hosts.' }

  const decode = (part: string) => {
    try {
      return decodeURIComponent(part)
    } catch {
      return part
    }
  }
  const fields: UrlFields = { driver, host }
  if (parsed.port) {
    const port = Number(parsed.port)
    if (port !== DEFAULT_PORTS[driver]) fields.port = port
  }
  if (parsed.username) fields.user = decode(parsed.username)
  if (parsed.password) fields.password = decode(parsed.password)
  const database = decode(parsed.pathname.replace(/^\//, ''))
  if (database.includes('/')) return { ok: false, reason: 'The URL path is not a database name.' }
  if (database) fields.database = database
  return { ok: true, fields }
}

/**
 * The structured entry for a legacy url entry, keeping every other key
 * (`tls`, `pool`, `capture`, `display`, …). `null` when the url cannot be
 * converted without loss.
 */
export function urlEntryToFields(db: JsonObject): JsonObject | null {
  const parsed = parseConnectionUrl(str(db.url))
  if (!parsed.ok) return null
  const entry: JsonObject = {}
  for (const [key, value] of Object.entries(db)) {
    if (key === 'url') {
      // The fields take the url's place, so the order of keys stays familiar.
      for (const [field, fieldValue] of Object.entries(parsed.fields)) {
        if (fieldValue !== undefined) entry[field] = fieldValue
      }
    } else {
      entry[key] = value
    }
  }
  return entry
}

/* ---------------- sensitive values ---------------- */

/** Every value that is a credential: stored in the secrets worker, never inline. */
export type SensitiveField = 'password' | 'tls.client_key' | 'ssh.password' | 'ssh.private_key' | 'ssh.passphrase'

export const SENSITIVE_FIELDS: readonly SensitiveField[] = [
  'password',
  'tls.client_key',
  'ssh.password',
  'ssh.private_key',
  'ssh.passphrase',
]

const FIELD_SUFFIX: Readonly<Record<SensitiveField, string>> = {
  password: 'PASSWORD',
  'tls.client_key': 'TLS_CLIENT_KEY',
  'ssh.password': 'SSH_PASSWORD',
  'ssh.private_key': 'SSH_PRIVATE_KEY',
  'ssh.passphrase': 'SSH_PASSPHRASE',
}

const MAX_SECRET_NAME = 128

/**
 * The secret a connection's credential is stored under:
 * `DATABASE_<CONNECTION>_<FIELD>`, uppercased, every character outside
 * `[A-Za-z0-9_]` replaced by `_`. Long handles are cut so the name stays
 * within the secrets worker's 128-character limit with the field intact.
 */
export function secretNameFor(connection: string, field: SensitiveField): string {
  const suffix = `_${FIELD_SUFFIX[field]}`
  const handle = connection.toUpperCase().replace(/[^A-Z0-9_]/g, '_')
  const room = MAX_SECRET_NAME - 'DATABASE_'.length - suffix.length
  return `DATABASE_${handle.slice(0, room)}${suffix}`
}

/** `base`, or `base_2`, `base_3`, … — the first one no other field references. */
export function uniqueSecretName(base: string, taken: ReadonlySet<string>): string {
  if (!taken.has(base)) return base
  for (let n = 2; ; n++) {
    const suffix = `_${n}`
    const candidate = `${base.slice(0, MAX_SECRET_NAME - suffix.length)}${suffix}`
    if (!taken.has(candidate)) return candidate
  }
}

/** The value at a dotted field path (`ssh.private_key`). */
export function fieldValue(db: JsonObject, field: string): JsonValue | undefined {
  const [head, ...rest] = field.split('.')
  if (rest.length === 0) return db[head]
  const block = db[head]
  return isObject(block) ? fieldValue(block, rest.join('.')) : undefined
}

/**
 * Secret names referenced anywhere in the configuration, except by the one
 * field being edited. Two handles can derive the same name (`a-b` and `a_b`);
 * this is how the second one notices and picks `_2` instead of overwriting
 * the first one's password.
 */
export function secretNamesInUse(
  databases: JsonObject,
  except?: { connection: string; field: SensitiveField },
): Set<string> {
  const names = new Set<string>()
  for (const [connection, value] of Object.entries(databases)) {
    if (!isObject(value)) continue
    for (const field of SENSITIVE_FIELDS) {
      if (except && except.connection === connection && except.field === field) continue
      const name = secretRefName(fieldValue(value, field))
      if (name) names.add(name)
    }
  }
  return names
}

export type ValueKind = 'empty' | 'secret' | 'env' | 'template' | 'redacted' | 'literal' | 'opaque'

/** How a credential is held: by reference, as a template, or (wrongly) inline. */
export function valueKind(value: JsonValue | undefined): ValueKind {
  if (value === undefined || value === null) return 'empty'
  if (typeof value !== 'string') return 'opaque'
  if (value.trim() === '') return 'empty'
  if (value === REDACTED) return 'redacted'
  if (secretRefName(value)) return 'secret'
  if (envRefName(value)) return 'env'
  if (isEnvironmentValue(value) || value.includes('${')) return 'template'
  return 'literal'
}

/**
 * Why a handle cannot name a connection, or `undefined`. The worker's own
 * rule (`validate_connection_name`): names travel in every call, in mention
 * tokens and in secret names, so they stay to 1–64 letters, digits, `_`,
 * `-` and `.`, starting with a letter, digit or `_`.
 */
export function connectionNameError(name: string): string | undefined {
  if (/^[A-Za-z0-9_][A-Za-z0-9_.-]{0,63}$/.test(name)) return undefined
  return 'Use 1–64 letters, digits, “_”, “-” or “.”, starting with a letter, digit or “_”.'
}

/** An OpenSSH SHA-256 fingerprint, as `ssh-keygen -lf` prints it. */
export function isHostKeyPin(value: string): boolean {
  const b64 = value.startsWith('SHA256:') ? value.slice('SHA256:'.length).replace(/=+$/, '') : ''
  return b64.length === 43 && /^[A-Za-z0-9+/]+$/.test(b64)
}

/** An absolute path on the engine host (POSIX, or a Windows drive path). */
export function isAbsolutePath(value: string): boolean {
  return value.startsWith('/') || /^[A-Za-z]:[\\/]/.test(value)
}

/**
 * A key field's value that names a file on the engine host rather than
 * holding the key: one line, no PEM armour, path-shaped. The worker reads
 * the file; a path is not a secret, so it is kept as typed.
 */
export function looksLikeKeyPath(value: string): boolean {
  const v = value.trim()
  return v !== '' && !v.includes('\n') && !isPem(v) && (isAbsolutePath(v) || v.startsWith('~/'))
}

/**
 * What an entry still needs before the worker will accept it — the checks
 * its `validate` makes that a form can see coming. Field paths are relative
 * to the entry.
 */
export function connectionProblems(db: JsonObject): { field: string; message: string }[] {
  const out: { field: string; message: string }[] = []
  const tls = isObject(db.tls) ? db.tls : {}
  if ((tls.client_cert !== undefined) !== (tls.client_key !== undefined)) {
    out.push({ field: 'tls', message: 'Mutual TLS needs both the client certificate and its key.' })
  }
  if (isObject(db.ssh)) {
    const ssh = db.ssh
    if (ssh.password === undefined && ssh.private_key === undefined) {
      out.push({ field: 'ssh', message: 'The tunnel needs a password or a private key.' })
    }
    if (ssh.passphrase !== undefined && ssh.private_key === undefined) {
      out.push({ field: 'ssh.passphrase', message: 'A passphrase only applies to a private key.' })
    }
    const pin = str(ssh.host_key)
    if (pin && !isEnvironmentValue(pin) && !isHostKeyPin(pin)) {
      out.push({ field: 'ssh.host_key', message: 'Expected SHA256: followed by 43 base64 characters.' })
    }
  }
  const folder = str(db.folder)
  if (folder && !folder.includes('${') && !isAbsolutePath(folder)) {
    out.push({ field: 'folder', message: 'Use an absolute path on the engine host.' })
  }
  return out
}

/* ---------------- certificates ---------------- */

const PEM_RE = /-----BEGIN ([A-Z0-9 ]+)-----[\s\S]*?-----END \1-----/

export function isPem(text: string): boolean {
  return PEM_RE.test(text)
}

/** `CERTIFICATE`, `PRIVATE KEY`, … for the first PEM block, or `null`. */
export function pemLabel(text: string): string | null {
  return PEM_RE.exec(text)?.[1] ?? null
}

/** One line describing an inline PEM value without printing it. */
export function pemSummary(text: string): string {
  const blocks = text.match(new RegExp(PEM_RE.source, 'g'))?.length ?? 0
  const label = pemLabel(text)
  if (!label) return 'Not a PEM value'
  return blocks > 1 ? `${blocks} PEM blocks · ${label}` : `PEM · ${label}`
}

/* ---------------- display ---------------- */

/** The environment editor's tones (iii Android `Palette.Tones`). */
export const CONNECTION_COLORS: readonly string[] = [
  '#FAFAFA',
  '#2563EB',
  '#7C3AED',
  '#0891B2',
  '#16A34A',
  '#EA580C',
  '#DB2777',
  '#F59E0B',
  '#DC2626',
  '#64748B',
]

export function isHexColor(value: unknown): value is string {
  return typeof value === 'string' && /^#[0-9A-Fa-f]{6}$/.test(value)
}

export interface ConnectionDisplay {
  icon?: string
  color?: string
}

export function connectionDisplay(db: JsonValue | undefined): ConnectionDisplay {
  if (!isObject(db) || !isObject(db.display)) return {}
  const icon = str(db.display.icon) || undefined
  const color = isHexColor(db.display.color) ? db.display.color : undefined
  return { icon, color }
}

/** `host:port/db` (or the file) — what a list row says about where it points. */
export function connectionSummary(db: JsonObject): string {
  if (connectionShape(db) === 'url') return ''
  const driver = connectionDriver(db)
  const database = str(db.database)
  if (driver === 'sqlite') return database
  const host = str(db.host)
  const port = typeof db.port === 'number' ? db.port : str(db.port)
  const endpoint = port !== '' ? `${host}:${port}` : host
  return database ? `${endpoint}/${database}` : endpoint
}

/* ---------------- test connection ---------------- */

/**
 * The `database::testConnection` request for a draft entry. A worker that
 * knows structured entries takes the whole entry (`connection`); `name` goes
 * along only when the draft still holds the `__redacted__` sentinel, which
 * the worker can fill from the stored entry of that name. An older worker
 * only understands `{url, tls}`.
 */
export function testConnectionPayload(
  name: string,
  db: JsonObject,
  structured: boolean,
  timeoutMs = 8000,
): Record<string, unknown> {
  if (!structured) return { url: str(db.url), tls: db.tls ?? undefined, timeout_ms: timeoutMs }
  const payload: Record<string, unknown> = { connection: db, timeout_ms: timeoutMs }
  if (JSON.stringify(db).includes(`"${REDACTED}"`)) payload.name = name
  return payload
}

export const TEST_STAGE_LABELS: Readonly<Record<string, string>> = {
  config: 'Configuration',
  secrets: 'Resolving secrets',
  ssh: 'SSH tunnel',
  tcp: 'Network',
  tls: 'TLS',
  auth: 'Authentication',
  database: 'Database',
}

export interface TestOutcome {
  ok: boolean
  driver?: string
  latency_ms?: number
  server_version?: string | null
  message?: string | null
  stage?: string | null
}

/** One status line for a probe: `Connected · PostgreSQL 17.2 · 12 ms` or where it failed. */
export function testOutcomeText(resp: TestOutcome): string {
  const latency = typeof resp.latency_ms === 'number' ? `${resp.latency_ms} ms` : null
  if (resp.ok) {
    return ['Connected', resp.server_version || resp.driver, latency].filter(Boolean).join(' · ')
  }
  const stage = resp.stage ? (TEST_STAGE_LABELS[resp.stage] ?? resp.stage) : null
  const message = resp.message || 'Connection failed'
  return stage ? `${stage} failed · ${message}` : message
}
