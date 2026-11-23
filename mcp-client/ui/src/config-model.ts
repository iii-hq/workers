import { parsePayload } from './schema-form'
import { looksSecretName } from './secret-names'
import type { Issue } from './types'

/** The draft of one `mcpServers` entry as the Connection form edits it, and the entry's portable copy. */

export interface KV {
  name: string
  value: string
}

export interface Draft {
  name: string
  transport: 'http' | 'stdio'
  url: string
  headers: KV[]
  command: string
  args: string[]
  env: KV[]
  cwd: string
  disabled: boolean
  authMode: 'none' | 'bearer' | 'client_credentials' | 'oauth'
  bearer: { token: string }
  clientCredentials: { token_url: string; client_id: string; client_secret: string; scope: string }
  oauth: { scopes: string; client_id: string; client_secret: string; client_metadata_url: string }
  /** Every field the form does not show, written back as it was. */
  extra: Record<string, unknown>
}

type Json = Record<string, unknown>

/** A whole `secret://NAME` or `env://NAME`: the scheme in any case, `NAME` as the secrets worker spells it. */
export const REFERENCE = /^(secret|env):\/\/[A-Za-z_][A-Za-z0-9_.-]{0,127}$/i

const KNOWN = new Set(['type', 'url', 'headers', 'command', 'args', 'env', 'cwd', 'disabled'])
const AUTH_MODES = new Set(['bearer', 'client_credentials', 'oauth'])
const AUTH_SECRETS = new Set(['token', 'client_secret'])
/** Plain `auth` keys: `token_url` looks like a secret name but is not one. */
const AUTH_PLAIN = new Set(['type', 'client_id', 'token_url', 'scope', 'scopes', 'client_metadata_url'])
/** What the JSON view shows in place of a value in a secret position; itself a value, so an edit that
 * keeps it is refused. */
const HIDDEN = '<hidden: store it as a secret>'

const isObject = (value: unknown): value is Json => typeof value === 'object' && value !== null && !Array.isArray(value)
const str = (value: unknown): string => (typeof value === 'string' ? value : '')
const isEmpty = (value: unknown) =>
  value === '' || (Array.isArray(value) && value.length === 0) || (isObject(value) && Object.keys(value).length === 0)
/** The fields with a value: an empty optional field is left out of the entry. */
const filled = (fields: Json): Json => Object.fromEntries(Object.entries(fields).filter(([, value]) => !isEmpty(value)))
/** A value in a secret position as the page may hold or show it: a whole reference, else `''`. */
export const referenceOnly = (value: string): string => (REFERENCE.test(value) ? value : '')
const literal = (value: string) => value !== '' && !REFERENCE.test(value)
// Each row is read as if just named, so a secret-named row holds a reference or ''.
const rowsOf = (value: unknown): KV[] =>
  isObject(value) ? Object.entries(value).map(([name, v]) => renameRow({ name, value: str(v) }, name)) : []
const mapOf = (rows: KV[]): Json =>
  Object.fromEntries(rows.filter((row) => row.name !== '').map((row) => [row.name, row.value]))
/** `blank`'s keys read from `from` as strings, a secret as a reference only; `blank` itself when there is
 * nothing to read. */
const read = <T extends Record<string, string>>(blank: T, from: Json | null): T =>
  from
    ? (Object.fromEntries(
        Object.keys(blank).map((key) => [key, AUTH_SECRETS.has(key) ? referenceOnly(str(from[key])) : str(from[key])]),
      ) as T)
    : blank
const placeholder = (name: string) => `\${${name}}`
const referenceName = (value: string) => value.slice(value.indexOf('://') + 3)

export function emptyDraft(): Draft {
  return {
    name: '',
    transport: 'http',
    url: '',
    headers: [],
    command: '',
    args: [],
    env: [],
    cwd: '',
    disabled: false,
    authMode: 'none',
    bearer: { token: '' },
    clientCredentials: { token_url: '', client_id: '', client_secret: '', scope: '' },
    oauth: { scopes: '', client_id: '', client_secret: '', client_metadata_url: '' },
    extra: {},
  }
}

/** The form's draft of a stored entry. A field of the wrong JSON type reads as empty, and so does a value in
 * a secret position the form shows (the worker refuses it anyway); an `auth` the form cannot show stays in
 * `extra` with every unknown field. */
export function draftFromEntry(name: string, entry: unknown): Draft {
  const e = isObject(entry) ? entry : {}
  const auth = isObject(e.auth) && AUTH_MODES.has(str(e.auth.type)) ? e.auth : null
  const of = (mode: Draft['authMode']) => (auth?.type === mode ? auth : null)
  const blank = emptyDraft()
  const oauth = of('oauth')
  const http = e.type === 'http' || e.type === 'streamable-http'
  return {
    name,
    transport: e.type === 'stdio' || (!http && e.command !== undefined) ? 'stdio' : 'http',
    url: str(e.url),
    headers: rowsOf(e.headers),
    command: str(e.command),
    args: Array.isArray(e.args) ? e.args.map(str) : [],
    env: rowsOf(e.env),
    cwd: str(e.cwd),
    disabled: e.disabled === true,
    authMode: auth ? (auth.type as Draft['authMode']) : 'none',
    bearer: read(blank.bearer, of('bearer')),
    clientCredentials: read(blank.clientCredentials, of('client_credentials')),
    oauth: {
      ...read(blank.oauth, oauth),
      scopes: oauth && Array.isArray(oauth.scopes) ? oauth.scopes.map(str).join(' ') : '',
    },
    extra: Object.fromEntries(Object.entries(e).filter(([key]) => !KNOWN.has(key) && !(key === 'auth' && auth))),
  }
}

/** The entry a draft stores: `type` always, the chosen transport's fields and only the chosen auth mode's,
 * over the fields the form does not show. */
export function entryFromDraft(draft: Draft): Json {
  const fields =
    draft.transport === 'http'
      ? { url: draft.url, headers: mapOf(draft.headers) }
      : { command: draft.command, args: draft.args.filter(Boolean), env: mapOf(draft.env), cwd: draft.cwd }
  const entry: Json = { type: draft.transport, ...filled(fields) }
  for (const [key, value] of Object.entries(draft.extra)) if (!(key in entry)) entry[key] = value
  if (draft.disabled) entry.disabled = true
  if (draft.transport === 'http' && draft.authMode !== 'none') {
    const { oauth } = draft
    const auth =
      draft.authMode === 'bearer'
        ? draft.bearer
        : draft.authMode === 'client_credentials'
          ? draft.clientCredentials
          : { ...oauth, scopes: oauth.scopes.split(/[\s,]+/).filter(Boolean) }
    entry.auth = { type: draft.authMode, ...filled(auth) }
  }
  return entry
}

/** A header or env row under a new name: a value that is no reference is dropped once the name looks
 * like a credential. */
export const renameRow = (row: KV, name: string): KV => ({
  name,
  value: looksSecretName(name) ? referenceOnly(row.value) : row.value,
})

/** `entry` with `swap` applied to every string in a secret position: a secret-named header or env value,
 * `auth.token`, `auth.client_secret`, any other secret-named key of an `auth` of a known type, and a
 * secret-named field the form does not show. */
function mapSecrets(entry: Json, swap: (value: string, path: string) => string): Json {
  const within = (prefix: string, value: Json, secret: (name: string) => boolean) =>
    Object.fromEntries(
      Object.entries(value).map(([name, v]) => [
        name,
        typeof v === 'string' && secret(name) ? swap(v, `${prefix}/${name}`) : v,
      ]),
    )
  return Object.fromEntries(
    Object.entries(entry).map(([key, value]) => [
      key,
      (key === 'headers' || key === 'env') && isObject(value)
        ? within(key, value, looksSecretName)
        : key === 'auth' && isObject(value)
          ? within(
              key,
              value,
              (name) =>
                AUTH_SECRETS.has(name) ||
                (AUTH_MODES.has(str(value.type)) && !AUTH_PLAIN.has(name) && looksSecretName(name)),
            )
          : typeof value === 'string' && !KNOWN.has(key) && looksSecretName(key)
            ? swap(value, key)
            : value,
    ]),
  )
}

/** The paths (never the values) of every value in a secret position that is not a reference. */
export function literalSecrets(entry: unknown): string[] {
  const paths: string[] = []
  if (!isObject(entry)) return paths
  mapSecrets(entry, (value, path) => {
    if (literal(value)) paths.push(path)
    return value
  })
  return paths
}

/** Why the page refuses `entry`, or null: it holds a value where only a reference may stand. */
export function literalSecretsError(entry: unknown): string | null {
  const paths = literalSecrets(entry)
  if (paths.length === 0) return null
  return `${paths.join(', ')} ${paths.length === 1 ? 'holds' : 'hold'} a value. Use secret://NAME or env://NAME.`
}

/** `entry` as the JSON view shows it: a value in a secret position hidden, a reference as written. */
export const hideLiterals = (entry: Json): Json => mapSecrets(entry, (value) => (literal(value) ? HIDDEN : value))

/** One entry as other clients read it. A literal in a secret position is replaced by the vault name the
 * importer would give it, so the copy holds no value even when one was stored from the CLI. */
function portable(server: string, entry: unknown): Json {
  const draft = draftFromEntry(server, entry)
  const ref = (value: string) => (REFERENCE.test(value) ? placeholder(referenceName(value)) : value)
  const masked = (name: string, value: string) =>
    REFERENCE.test(value) || !looksSecretName(name) ? ref(value) : placeholder(secretVaultName(server, name))
  const mask = (rows: KV[]) => rows.map(({ name, value }) => ({ name, value: masked(name, value) }))
  const headers = mask(draft.headers)
  if (draft.authMode === 'bearer') {
    const { token } = draft.bearer
    const name = REFERENCE.test(token) ? referenceName(token) : secretVaultName(server, 'TOKEN')
    headers.push({ name: 'Authorization', value: `Bearer ${placeholder(name)}` })
  }
  const { auth: _, ...rest } = draft.extra
  const extra = Object.fromEntries(
    Object.entries(rest).map(([key, value]) => [key, typeof value === 'string' ? masked(key, value) : value]),
  )
  const out = entryFromDraft({
    ...draft,
    // shortcut: a literal url is copied as written; mask it here if validation ever lets a secret query through.
    url: ref(draft.url),
    headers,
    // args are no secret position: a reference becomes `${NAME}`, the rest is copied as written.
    args: draft.args.map(ref),
    env: mask(draft.env),
    disabled: false,
    authMode: 'none',
    extra,
  })
  if (draft.transport === 'http' && draft.authMode === 'oauth' && draft.oauth.client_id) {
    out.oauth = { clientId: draft.oauth.client_id }
  }
  return out
}

/** The `{ mcpServers }` other clients read: references as `${NAME}`, a bearer token as an Authorization
 * header, a pre-registered OAuth client as `oauth.clientId`; the rest of `auth` and `disabled` dropped. */
export function copyAsJson(mcpServers: Record<string, unknown>): string {
  const servers = Object.fromEntries(Object.entries(mcpServers).map(([name, entry]) => [name, portable(name, entry)]))
  return JSON.stringify({ mcpServers: servers }, null, 2)
}

/** Whether the form holds an edit: against the stored entry as the form reads it, or, while adding
 * (`stored` undefined), against an empty draft. */
export function draftDirty(draft: Draft, stored: unknown): boolean {
  if (stored === undefined) return !sameJson(draft, emptyDraft())
  return !sameJson(entryFromDraft(draft), entryFromDraft(draftFromEntry(draft.name, stored)))
}

const sorted = (value: unknown): unknown =>
  Array.isArray(value)
    ? value.map(sorted)
    : isObject(value)
      ? Object.fromEntries(
          Object.keys(value)
            .sort()
            .map((key) => [key, sorted(value[key])]),
        )
      : value

/** Deep equality that ignores object key order: the engine stores keys sorted. */
export const sameJson = (a: unknown, b: unknown): boolean => JSON.stringify(sorted(a)) === JSON.stringify(sorted(b))

const target = (entry: unknown) => (isObject(entry) ? [entry.url, entry.command, entry.args] : null)

/** Whether an edit changed where the entry connects or what it runs. */
export const connectionChanged = (before: unknown, after: unknown): boolean => !sameJson(target(before), target(after))

/** The entries a whole-map edit added or changed, and the ones it removed, each in code-unit order. */
export function changedEntries(
  before: Record<string, unknown>,
  after: Record<string, unknown>,
): { changed: string[]; removed: string[] } {
  return {
    changed: Object.keys(after)
      .filter((name) => !(name in before) || !sameJson(before[name], after[name]))
      .sort(),
    removed: Object.keys(before)
      .filter((name) => !(name in after))
      .sort(),
  }
}

/** Every reference an entry names, at any depth, each once and sorted: what Share asks to share. */
export function entryRefs(entry: unknown): string[] {
  const refs = new Set<string>()
  const walk = (value: unknown): void => {
    if (typeof value === 'string') {
      if (REFERENCE.test(value)) refs.add(value)
    } else if (Array.isArray(value)) value.forEach(walk)
    else if (isObject(value)) Object.values(value).forEach(walk)
  }
  walk(entry)
  return [...refs].sort()
}

/** The per-server JSON switch's text read against the draft it shows. The shown text, hidden values and all,
 * is no edit; an edit that keeps a value in a secret position is refused, so the draft never holds one. */
export function readEntryEdit(draft: Draft, text: string): { draft: Draft | null; error: string | null } {
  const parsed = parsePayload(text)
  if ('error' in parsed) return { draft: null, error: parsed.error }
  if (sameJson(parsed.payload, hideLiterals(entryFromDraft(draft)))) return { draft: null, error: null }
  const error = literalSecretsError(parsed.payload)
  return error ? { draft: null, error } : { draft: draftFromEntry(draft.name, parsed.payload), error: null }
}

const hideAll = (mcpServers: Record<string, unknown>) =>
  Object.fromEntries(Object.entries(mcpServers).map(([name, e]) => [name, isObject(e) ? hideLiterals(e) : e]))
const pointer = (name: string) => `/mcpServers/${name.replace(/~/g, '~0').replace(/\//g, '~1')}`

/** The whole map as Edit as JSON shows it: every value in a secret position hidden. */
export const mapAsJson = (mcpServers: Record<string, unknown>): string =>
  JSON.stringify({ mcpServers: hideAll(mcpServers) }, null, 2)

export const READ_MAP_ROOT = 'The text must be a JSON object with an object under mcpServers.'

/** Edit as JSON's text read against the map it showed (`mapAsJson(stored)`). An entry left as shown is no
 * change and goes to validation as stored, so its hidden values never come back from the text; a changed
 * entry that holds a value in a secret position is refused, one issue per entry, before anything is sent. */
export function readMapEdit(
  stored: Record<string, unknown>,
  text: string,
): { issues: Issue[] } | { candidate: Record<string, unknown>; changed: string[]; removed: string[] } {
  let root: unknown
  try {
    root = JSON.parse(text)
  } catch (err) {
    return { issues: [{ path: '', message: `Not valid JSON: ${(err as Error).message}` }] }
  }
  const edited = isObject(root) ? root.mcpServers : undefined
  if (!isObject(edited)) return { issues: [{ path: '', message: READ_MAP_ROOT }] }
  const { changed, removed } = changedEntries(hideAll(stored), edited)
  const issues = changed.flatMap((name) => {
    const message = literalSecretsError(edited[name])
    return message ? [{ path: pointer(name), message }] : []
  })
  if (issues.length > 0) return { issues }
  const candidate = Object.fromEntries(
    Object.keys(edited).map((name) => [name, changed.includes(name) ? edited[name] : stored[name]]),
  )
  return { candidate, changed, removed }
}

/** `<SERVER>_<FIELD>`: upper-case, anything outside `[A-Z0-9_]` as `_`, never a leading digit. */
export function secretVaultName(server: string, field: string): string {
  const name = `${server}_${field}`.toUpperCase().replace(/[^A-Z0-9_]/g, '_')
  return (/^[0-9]/.test(name) ? `_${name}` : name).slice(0, 128)
}

/** The validation messages of one server, keyed by the decoded pointer below its entry; `''` is the
 * entry itself. The first message of a field wins. */
export function fieldErrors(errors: Issue[], server: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const { path, message } of errors) {
    const [root, top, name, ...rest] = path.split('/').map((s) => s.replace(/~1/g, '/').replace(/~0/g, '~'))
    if (root === '' && top === 'mcpServers' && name === server) out[rest.join('/')] ??= message
  }
  return out
}

/** The paths the Connection form shows an error at: each field it renders, and each header, env or argument row. */
const FORM_FIELDS = new Set([
  'name',
  'type',
  'url',
  'headers',
  'command',
  'args',
  'env',
  'cwd',
  'auth',
  'auth/token',
  'auth/token_url',
  'auth/client_id',
  'auth/client_secret',
  'auth/scope',
  'auth/scopes',
  'auth/client_metadata_url',
])
const FORM_ROWS = /^(headers|env|args)\//

/** The errors of `fieldErrors` no field shows, to list with their paths: in JSON mode every one but the entry's
 * own (`''`), in form mode those whose path has no field there. */
export function unplacedErrors(fe: Record<string, string>, mode: 'form' | 'json'): { path: string; message: string }[] {
  return Object.entries(fe)
    .filter(([path]) => path !== '' && (mode === 'json' || !(FORM_FIELDS.has(path) || FORM_ROWS.test(path))))
    .map(([path, message]) => ({ path, message }))
}
