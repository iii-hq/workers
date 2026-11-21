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

const isObject = (value: unknown): value is Json => typeof value === 'object' && value !== null && !Array.isArray(value)
const str = (value: unknown): string => (typeof value === 'string' ? value : '')
const isEmpty = (value: unknown) =>
  value === '' || (Array.isArray(value) && value.length === 0) || (isObject(value) && Object.keys(value).length === 0)
/** The fields with a value: an empty optional field is left out of the entry. */
const filled = (fields: Json): Json => Object.fromEntries(Object.entries(fields).filter(([, value]) => !isEmpty(value)))
const rowsOf = (value: unknown): KV[] =>
  isObject(value) ? Object.entries(value).map(([name, v]) => ({ name, value: str(v) })) : []
const mapOf = (rows: KV[]): Json =>
  Object.fromEntries(rows.filter((row) => row.name !== '').map((row) => [row.name, row.value]))
/** `blank`'s keys read from `from` as strings; `blank` itself when there is nothing to read. */
const read = <T extends Record<string, string>>(blank: T, from: Json | null): T =>
  from ? (Object.fromEntries(Object.keys(blank).map((key) => [key, str(from[key])])) as T) : blank
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

/** The form's draft of a stored entry. A field of the wrong JSON type reads as empty; an `auth` the form
 * cannot show stays in `extra` with every unknown field. */
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

/** One entry as other clients read it. A literal in a secret position is replaced by the vault name the
 * importer would give it, so the copy holds no value even when one was stored from the CLI. */
function portable(server: string, entry: unknown): Json {
  const draft = draftFromEntry(server, entry)
  const ref = (value: string) => (REFERENCE.test(value) ? placeholder(referenceName(value)) : value)
  const mask = (rows: KV[]) =>
    rows.map(({ name, value }) => ({
      name,
      value: REFERENCE.test(value) || !looksSecretName(name) ? ref(value) : placeholder(secretVaultName(server, name)),
    }))
  const headers = mask(draft.headers)
  if (draft.authMode === 'bearer') {
    const { token } = draft.bearer
    const name = REFERENCE.test(token) ? referenceName(token) : secretVaultName(server, 'TOKEN')
    headers.push({ name: 'Authorization', value: `Bearer ${placeholder(name)}` })
  }
  const { auth: _, ...extra } = draft.extra
  const out = entryFromDraft({
    ...draft,
    // shortcut: a literal url is copied as written; mask it here if validation ever lets a secret query through.
    url: ref(draft.url),
    headers,
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
