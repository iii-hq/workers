import { errorMessage } from '@iii-dev/console-ui/format'
import { entryRefs, REFERENCE, secretVaultName } from './config-model'
import { looksSecretName } from './secret-names'
import type { Issue } from './types'

/** Reads an MCP config pasted from another client into rows the page can add. Pure: it runs nothing,
 * reads no file or helper a paste names, and never puts a pasted value into a message: a pasted value
 * lives only in `secrets[].value`, and the entry holds `''` at every secret and need position. */

export type SourceKind =
  | 'mcpServers'
  | 'vscode-editor'
  | 'zed'
  | 'acp'
  | 'registry'
  | 'single'
  | 'bare-map'
  | 'codex-toml'
  | 'unknown'

export interface ImportSecret {
  path: string
  vaultName: string
  value: string
  masked: string
}

export interface ImportNeed {
  path: string
  label: string
  password: boolean
  original: string
}

export interface ImportRow {
  originalName: string
  name: string
  status: 'new' | 'replaces' | 'skipped'
  skipReason?: string
  transport: 'http' | 'stdio' | null
  target: string
  entry: Record<string, unknown>
  secrets: ImportSecret[]
  needs: ImportNeed[]
  envRefs: string[]
  ignored: { field: string; note?: string }[]
  notes: string[]
}

export interface ImportResult {
  source: SourceKind
  sourceLabel: string
  rows: ImportRow[]
  error?: string
}

type Json = Record<string, unknown>
type Ignored = ImportRow['ignored'][number]

const isObject = (value: unknown): value is Json => typeof value === 'object' && value !== null && !Array.isArray(value)
const str = (value: unknown): string => (typeof value === 'string' ? value : '')
const given = (value: unknown): value is string => typeof value === 'string' && value !== ''
/** A pasted env or header value as text: a number or boolean as written, anything else dropped. */
const asText = (value: unknown): string | null =>
  typeof value === 'string' ? value : typeof value === 'number' || typeof value === 'boolean' ? String(value) : null

const LABELS: Record<SourceKind, string> = {
  mcpServers: 'Claude Code, Cursor and similar',
  'vscode-editor': 'VS Code',
  zed: 'Zed',
  acp: 'ACP',
  registry: 'MCP registry',
  single: 'One server',
  'bare-map': 'Server map',
  'codex-toml': 'Codex TOML',
  unknown: 'Not recognised',
}
const hasTarget = (value: unknown) =>
  isObject(value) && ['command', 'url', 'httpUrl', 'serverUrl'].some((key) => typeof value[key] === 'string')
const GEMINI = ['httpUrl', 'trust', 'includeTools', 'excludeTools']

const CONSUMED = new Set([
  'type',
  'name',
  'command',
  'args',
  'env',
  'cwd',
  'url',
  'httpUrl',
  'serverUrl',
  'headers',
  'http_headers',
  'env_http_headers',
  'bearer_token_env_var',
  'env_vars',
  'disabled',
  'oauthClientId',
])
const OAUTH_KEYS = new Set(['enabled', 'clientId', 'client_id', 'clientSecret', 'client_secret', 'scopes'])
const CURSOR_AUTH_KEYS = new Set(['CLIENT_ID', 'CLIENT_SECRET', 'scopes'])
const AUTH_MODES = new Set(['bearer', 'client_credentials', 'oauth'])
const CALLBACK = new Set(['callbackPort', 'callback_port', 'redirectUri', 'callback_url'])
const NOTES = new Map([
  ['headersHelper', 'not run'],
  ['http_headers_helper', 'not run'],
  ['envFile', 'not read'],
])
/** Headers the transport owns: validation refuses an entry that sets one. */
const RESERVED = /^(host|content-length|content-type|accept|mcp-.*)$/i

const VAR = '[A-Za-z_][A-Za-z0-9_]{0,127}'
/** A client's own path variable: no env value stands behind it. */
const BUILTIN =
  /\$\{(workspaceFolder|workspaceFolderBasename|userHome|pathSeparator|\/|CLAUDE_PROJECT_DIR|CLAUDE_PLUGIN_ROOT)\}/
const INPUT = /^\$\{input:([^}]+)\}$/
const ENV_REF = new RegExp(`^\\$\\{(?:env:)?(${VAR})(:-[^}]*)?\\}$`)
/** Gemini's spellings, read in an env block only. */
const ENV_BARE = new RegExp(`^(?:\\$(${VAR})|%(${VAR})%)$`)
const PLACEHOLDER = /^(<.+>|your[-_ ].+|x{3,}|\*{3,}|\.{3}|changeme|replace[-_ ]?me)$/i
const FLAG = /^--?[A-Za-z][\w-]*$/
const ASSIGNED = /^(--?[A-Za-z][\w-]*)=(.+)$/
/** An argument shaped as a header line: `Authorization: Bearer x`. */
const HEADER_ARG = /^([!#$%&'*+.^_`|~0-9A-Za-z-]+):\s*(.+)$/
/** Userinfo after any scheme or none (`${SCHEME}://` leaves `://`): a user with a character other than `:`,
 * or a non-empty password, which may hold `/`, `?` or `#`. `postgres://:@db` holds none. */
const USERINFO = /:\/\/(?:[^/?#@\s]*[^/?#@\s:][^/?#@\s]*@|(?:(?!:\/\/)\S)*:\S+@)/
const QUERY_NAME = /[?&]([^=&#\s]+)=/g
/** The inside of a plain reference, dropped before a template is read: `NAME`, `env:NAME`, `input:ID`, or
 * VS Code's `/` (its other builtins are names). */
const PLAIN = new RegExp(`^(?:(?:env:)?${VAR}|input:[\\w.-]+|/)$`)
/** The name and operator a reference with a word starts with: `:-`, `-`, `:=`, `=`, `:?`, `?`, `:+`, `+`. */
const OPERATOR = new RegExp(`^(?:env:)?${VAR}:?[-=?+]`)
const BEARER = /^Bearer\s+(.+)$/i
const ABSOLUTE = /^([A-Za-z]:[\\/]|\/)/
const PACKAGES_NOTE = 'packages are not imported: add a stdio server by hand'
const LABEL = new Map([
  ['/url', 'URL'],
  ['/command', 'Command'],
  ['/cwd', 'Working directory (absolute path)'],
  ['/auth/token', 'API key'],
  ['/auth/client_id', 'OAuth client ID'],
  ['/auth/client_secret', 'OAuth client secret'],
])

const fail = (source: SourceKind, error: string): ImportResult => ({
  source,
  sourceLabel: LABELS[source],
  rows: [],
  error,
})
const ig = (field: string, note?: string): Ignored => (note ? { field, note } : { field })
/** A name-to-value map, or an ACP or registry `[{name, value}]` list, as pairs. */
const pairs = (value: unknown): [string, unknown][] =>
  Array.isArray(value)
    ? value.flatMap((item): [string, unknown][] =>
        isObject(item) && typeof item.name === 'string' ? [[item.name, item.value]] : [],
      )
    : isObject(value)
      ? Object.entries(value)
      : []
const isSecretFlag = (arg: unknown) =>
  typeof arg === 'string' && FLAG.test(arg) && looksSecretName(arg.replace(/^-+/, ''))

/** RFC 6901 pointers into an entry. */
const pointer = (...keys: (string | number)[]) =>
  keys.map((key) => `/${String(key).replace(/~/g, '~0').replace(/\//g, '~1')}`).join('')
const keysOf = (path: string) =>
  path
    .slice(1)
    .split('/')
    .map((key) => key.replace(/~1/g, '/').replace(/~0/g, '~'))
const labelOf = (path: string) => {
  const [top, key] = keysOf(path)
  return LABEL.get(path) ?? (top === 'args' ? `Argument ${Number(key) + 1}` : key)
}
/** The field a vault name is made of: the header or variable name, the flag before an argument, else the
 * last key (`TOKEN`, `CLIENT_SECRET`, `URL`). */
function fieldOf(path: string, args: unknown): string {
  const keys = keysOf(path)
  const last = keys[keys.length - 1]
  if (keys[0] !== 'args') return last
  const flag = Array.isArray(args) ? str(args[Number(last) - 1]) : ''
  return FLAG.test(flag) ? flag.replace(/^-+/, '') : `ARG_${Number(last) + 1}`
}
/** Whether a string carries a URL credential: userinfo, or a query parameter whose name looks like one.
 * Read as text and parsed, so a URL inside a longer string or around a `${…}` counts too. */
function holdsCredential(value: string): boolean {
  if (USERINFO.test(value) || [...value.matchAll(QUERY_NAME)].some((m) => looksSecretName(m[1]))) return true
  try {
    const url = new URL(value)
    return url.username !== '' || url.password !== '' || [...url.searchParams.keys()].some(looksSecretName)
  } catch {
    return false
  }
}
/** The value of an argument that names its credential itself, `--token=<value>` or `Authorization: <value>`,
 * without a `Bearer` prefix; undefined for any other argument. */
function secretArgValue(arg: string): string | undefined {
  const m = ASSIGNED.exec(arg) ?? HEADER_ARG.exec(arg)
  return m && looksSecretName(m[1].replace(/^-+/, '')) ? m[2].replace(/^Bearer\s+/i, '') : undefined
}
/** `text` with every plain reference removed and each other one kept as written (`raw`), replaced by its word
 * read the same way (`word`), or replaced by `x`; null when a `${` does not close or parse. Braces are
 * matched by counting, so a default may hold `}` of a nested reference. */
function expand(text: string, world: 'raw' | 'word' | 'x'): string | null {
  let out = ''
  let at = 0
  for (let open = text.indexOf('${'); open >= 0; open = text.indexOf('${', at)) {
    let depth = 1
    let end = open + 2
    for (; depth > 0 && end < text.length; end++) {
      if (text.startsWith('${', end)) {
        depth++
        end++
      } else if (text[end] === '}') depth--
    }
    if (depth > 0) return null
    const body = text.slice(open + 2, end - 1)
    out += text.slice(at, open)
    at = end
    if (PLAIN.test(body)) continue
    const head = OPERATOR.exec(body)?.[0]
    if (head === undefined) return null
    const word = world === 'x' ? 'x' : expand(body.slice(head.length), world)
    if (word === null) return null
    out += world === 'raw' ? `\${${head}${word}}` : word
  }
  return out + text.slice(at)
}
/** Whether a template may hold a literal credential, once its plain references are removed: read with its
 * other references as written, as their words and as `x`, any one carries a credential (for an argument, a
 * secret-looking `--flag=` or `Name:` that keeps a value), or a `${` does not close or parse. In doubt it
 * says yes: a template masked for nothing costs less than a literal shown. */
function literalCredential(value: string, arg: boolean): boolean {
  return (['raw', 'word', 'x'] as const).some((world) => {
    const text = expand(value, world)
    return text === null || holdsCredential(text) || (arg && (secretArgValue(text) ?? '').trim() !== '')
  })
}
/** `base`, or `base<sep>2`, `base<sep>3`… cut to `max`, while `taken` refuses the name. */
function free(base: string, sep: string, max: number, taken: (name: string) => boolean): string {
  let name = base
  for (let n = 2; taken(name); n++) name = `${base.slice(0, max - `${sep}${n}`.length)}${sep}${n}`
  return name
}
const envRefsOf = (entry: Json) =>
  [
    ...new Set(
      entryRefs(entry)
        .filter((ref) => /^env:/i.test(ref))
        .map((ref) => ref.slice('env://'.length)),
    ),
  ].sort()

interface Found {
  source: SourceKind
  servers: [string, unknown][]
  gemini?: boolean
  inputs?: Map<string, Json>
  notes?: string[]
}

/** The source by root shape, first match wins. */
function detect(root: unknown): Found | ImportResult {
  const r = isObject(root) ? root : {}
  if (Array.isArray(r.mcpServers)) {
    return { source: 'acp', servers: r.mcpServers.map((s): [string, unknown] => [str(isObject(s) ? s.name : ''), s]) }
  }
  if (isObject(r.mcpServers)) {
    const servers = Object.entries(r.mcpServers)
    return { source: 'mcpServers', servers, gemini: servers.some(([, s]) => isObject(s) && GEMINI.some((k) => k in s)) }
  }
  if (isObject(r.servers)) {
    const inputs = (Array.isArray(r.inputs) ? r.inputs : []).filter(isObject)
    return {
      source: 'vscode-editor',
      servers: Object.entries(r.servers),
      inputs: new Map(inputs.map((input) => [str(input.id), input])),
    }
  }
  if (isObject(r.context_servers)) return { source: 'zed', servers: Object.entries(r.context_servers) }
  if (Array.isArray(r.remotes) || Array.isArray(r.packages)) {
    const remotes = Array.isArray(r.remotes) ? r.remotes : []
    if (remotes.length === 0)
      return fail('registry', 'This registry entry has no remote server. Add the server by hand')
    const name = str(r.name).slice(str(r.name).lastIndexOf('/') + 1)
    return {
      source: 'registry',
      servers: remotes.map((s): [string, unknown] => [name, s]),
      notes: Array.isArray(r.packages) && r.packages.length > 0 ? [PACKAGES_NOTE] : [],
    }
  }
  if (hasTarget(r)) return { source: 'single', servers: [[str(r.name), r]] }
  const values = Object.values(r)
  if (values.length > 0 && values.every(hasTarget)) return { source: 'bare-map', servers: Object.entries(r) }
  return fail('unknown', 'No MCP servers found in this JSON')
}

function skipReason(value: Json, gemini: boolean): string | undefined {
  if (value.type === 'sse') return 'SSE is not supported'
  if (value.type === 'ws') return 'WebSocket is not supported'
  if (value.type === 'sdk') return 'In-process SDK servers cannot be imported'
  if (gemini && typeof value.url === 'string' && value.httpUrl === undefined && value.type === undefined) {
    return 'Gemini\'s "url" is an SSE endpoint, which is not supported'
  }
  if (!hasTarget(value)) return 'No command and no url'
  return undefined
}

/** The fields mcp-client does not take, by name only: a value is never copied. With a bearer header, an
 * OAuth block is one of them. */
function ignoredOf(e: Json, inputs: Map<string, Json>, bearer: boolean): Ignored[] {
  const out: Ignored[] = []
  const sub = (key: string, value: Json, keep: Set<string>) => {
    for (const k of Object.keys(value)) {
      if (!keep.has(k))
        out.push(ig(`${key}.${k}`, key === 'oauth' && CALLBACK.has(k) ? "the other client's callback" : undefined))
    }
  }
  for (const [key, value] of Object.entries(e)) {
    const cursorAuth = key === 'auth' && isObject(value) && 'CLIENT_ID' in value
    const pageAuth = key === 'auth' && isObject(value) && AUTH_MODES.has(str(value.type))
    if (bearer && (key === 'oauth' || key === 'oauthClientId' || cursorAuth))
      out.push(ig(key, 'an Authorization header is set'))
    else if (key === 'oauth' && isObject(value)) sub(key, value, OAUTH_KEYS)
    else if (cursorAuth) sub(key, value, CURSOR_AUTH_KEYS)
    else if (!CONSUMED.has(key) && !pageAuth) out.push(ig(key, NOTES.get(key)))
  }
  for (const [name] of [...pairs(e.headers), ...pairs(e.http_headers), ...pairs(e.env_http_headers)]) {
    if (RESERVED.test(name)) out.push(ig(`headers.${name}`, 'set by mcp-client'))
  }
  // The helpers and envFile stay unread, even as text.
  const json = JSON.stringify(Object.entries(e).filter(([key]) => !NOTES.has(key)))
  for (const [id, input] of inputs) {
    if (input.type === 'command' && json.includes(`\${input:${id}}`)) out.push(ig(`inputs.${id}`, 'not run'))
  }
  return out
}

type Built = Pick<ImportRow, 'entry' | 'secrets' | 'needs' | 'ignored' | 'notes'>

/** One entry rewritten for mcp-client (5.2) with its secrets and needs found (6.7), or why it is skipped. */
function build(e: Json, server: string, inputs: Map<string, Json>, registry: boolean): Built | string {
  const args = Array.isArray(e.args) ? e.args.filter((arg): arg is string => typeof arg === 'string') : []
  for (const arg of args) {
    const m = ASSIGNED.exec(arg)
    // A literal value: rules 1 to 6 of the table all read a reference, a `${`, or a placeholder.
    if (m && isSecretFlag(m[1]) && !REFERENCE.test(m[2]) && !m[2].includes('${') && !PLACEHOLDER.test(m[2])) {
      return `Split \`${m[1]}=…\` into two arguments, then paste again`
    }
  }
  const secrets: ImportSecret[] = []
  const needs: ImportNeed[] = []
  const notes: string[] = []
  const need = (path: string, password: boolean, original: string, label = labelOf(path)) => {
    needs.push({ path, label, password, original: password ? '' : original })
    return ''
  }
  /** One pasted string through the rewrite table, first match wins: what the entry holds at `path`. */
  const put = (path: string, value: string, secretName: boolean, envBlock = false): string => {
    if (REFERENCE.test(value)) return value
    const arg = path.startsWith('/args/')
    const template = value.includes('${')
    // A template is read three ways, its plain references removed: a whole `${DB_URL:-<credential URL>}` is
    // still a reference (rule 4), and userinfo made only of references a need that shows its template (rule 5).
    const credential = template ? literalCredential(value, arg) : holdsCredential(value)
    if (credential && template && !ENV_REF.test(value)) {
      // Asked for again whole: the credential around the reference is never shown.
      needs.push({ path, label: labelOf(path), password: true, original: maskSecret(value) })
      return ''
    }
    const argValue = arg && !template ? secretArgValue(value) : undefined
    // Any string that carries a credential is a secret position, whatever field it sits in.
    const secret = secretName || credential || argValue !== undefined
    if (BUILTIN.test(value)) return need(path, secret, value)
    const input = INPUT.exec(value)
    if (input) {
      const def = inputs.get(input[1])
      return need(path, secret || def?.password === true, '', str(def?.description) || input[1])
    }
    const ref = ENV_REF.exec(value)
    if (ref) {
      if (ref[2] !== undefined) notes.push(`default for \`${ref[1]}\` dropped`)
      return `env://${ref[1]}`
    }
    const bare = envBlock ? ENV_BARE.exec(value) : null
    if (bare) return `env://${bare[1] ?? bare[2]}`
    if (value.includes('${')) return need(path, secret, value)
    // A `--token=…` or header argument whose value is a reference or a placeholder cannot keep it inside.
    if (
      PLACEHOLDER.test(value) ||
      (argValue !== undefined && (REFERENCE.test(argValue) || PLACEHOLDER.test(argValue)))
    ) {
      return need(path, secret, '')
    }
    if (!secret) return value
    // An empty value in a secret position is missing, not a secret to store.
    if (value === '') return need(path, true, '')
    // A vault name another value holds already gets `_2`, `_3`…: no pasted secret is lost.
    const vaultName = free(secretVaultName(server, fieldOf(path, args)), '_', 128, (name) =>
      secrets.some((s) => s.vaultName === name && s.value !== value),
    )
    secrets.push({ path, vaultName, value, masked: maskSecret(value) })
    return ''
  }

  const rawUrl = [e.httpUrl, e.serverUrl, e.url].find((url): url is string => typeof url === 'string')
  const url = rawUrl === undefined ? undefined : put('/url', rawUrl, false)

  const headers: [string, string][] = []
  let bearer: string | undefined = given(e.bearer_token_env_var) ? `env://${e.bearer_token_env_var}` : undefined
  const pasted = registry
    ? []
    : [
        ...pairs(e.headers),
        ...pairs(e.http_headers),
        ...pairs(e.env_http_headers).flatMap(([name, v]): [string, unknown][] =>
          given(v) ? [[name, `env://${v}`]] : [],
        ),
      ]
  for (const [name, raw] of pasted) {
    const value = asText(raw)
    if (value === null || RESERVED.test(name)) continue
    const m = /^authorization$/i.test(name) ? BEARER.exec(value) : null
    if (m) bearer = m[1]
    else headers.push([name, put(pointer('headers', name), value, looksSecretName(name))])
  }
  // A registry header is a descriptor: the person types its value.
  for (const d of registry && Array.isArray(e.headers) ? e.headers.filter(isObject) : []) {
    const name = str(d.name)
    if (name === '' || RESERVED.test(name)) continue
    need(
      pointer('headers', name),
      looksSecretName(name) || d.isSecret === true,
      str(d.default),
      str(d.description) || name,
    )
    headers.push([name, ''])
  }

  const command = typeof e.command === 'string' ? put('/command', e.command, false) : undefined
  const argv = args.map((arg, i) => put(pointer('args', i), arg, isSecretFlag(args[i - 1])))
  const env = [
    ...pairs(e.env),
    ...(Array.isArray(e.env_vars) ? e.env_vars.filter(given) : []).map((v): [string, unknown] => [v, `env://${v}`]),
  ].flatMap(([name, raw]): [string, string][] => {
    const value = asText(raw)
    return value === null ? [] : [[name, put(pointer('env', name), value, looksSecretName(name), true)]]
  })
  let cwd: string | undefined
  if (given(e.cwd)) {
    const value = put('/cwd', e.cwd, false)
    cwd = value !== '' && !ABSOLUTE.test(value) ? need('/cwd', false, e.cwd) : value
  }

  const page = isObject(e.auth) && !('CLIENT_ID' in e.auth) && AUTH_MODES.has(str(e.auth.type)) ? e.auth : null
  let auth: Json | undefined
  if (page) {
    // The page's own format, kept as written: its secrets still go to the vault.
    auth = { ...page }
    for (const key of ['token', 'client_secret']) {
      const value = asText(page[key])
      if (value === null) delete auth[key]
      else auth[key] = put(pointer('auth', key), value, true)
    }
  } else if (bearer !== undefined) {
    auth = { type: 'bearer', token: put('/auth/token', bearer, true) }
  } else {
    const oauth = isObject(e.oauth) && e.oauth.enabled !== false ? e.oauth : {}
    const cursor = isObject(e.auth) && 'CLIENT_ID' in e.auth ? e.auth : {}
    const id = [oauth.clientId, oauth.client_id, e.oauthClientId, cursor.CLIENT_ID].find(given)
    const secret = [oauth.clientSecret, oauth.client_secret, cursor.CLIENT_SECRET].find(given)
    const rawScopes = oauth.scopes ?? cursor.scopes
    const scopes =
      typeof rawScopes === 'string'
        ? rawScopes.split(/\s+/).filter(Boolean)
        : Array.isArray(rawScopes)
          ? rawScopes.filter(given)
          : []
    if (id !== undefined || scopes.length > 0) {
      auth = { type: 'oauth' }
      // A client id is plain text: a reference there has nothing to resolve it.
      if (id !== undefined)
        auth.client_id =
          REFERENCE.test(id) || ENV_REF.test(id)
            ? need('/auth/client_id', false, id)
            : put('/auth/client_id', id, false)
      if (secret !== undefined) auth.client_secret = put('/auth/client_secret', secret, true)
      if (scopes.length > 0) auth.scopes = scopes
    }
  }

  const entry: Json = { type: typeof e.command === 'string' ? 'stdio' : 'http' }
  if (url !== undefined) entry.url = url
  if (headers.length > 0) entry.headers = Object.fromEntries(headers)
  if (command !== undefined) entry.command = command
  if (argv.length > 0) entry.args = argv
  if (env.length > 0) entry.env = Object.fromEntries(env)
  if (cwd !== undefined) entry.cwd = cwd
  if (auth) entry.auth = auth
  if (e.disabled === true) entry.disabled = true
  return { entry, secrets, needs, notes, ignored: ignoredOf(e, inputs, !page && bearer !== undefined) }
}

/** Where the entry connects or what it runs, never a value: `…` for an empty position. */
function targetOf(entry: Json, secrets: ImportSecret[]): string {
  if (entry.type === 'stdio') {
    return [entry.command, ...(Array.isArray(entry.args) ? entry.args : [])].map((v) => str(v) || '…').join(' ')
  }
  const hidden = secrets.find((s) => s.path === '/url')
  if (!hidden) return str(entry.url) || '…'
  try {
    const url = new URL(hidden.value)
    return `${url.protocol}//${url.host} (stored as a secret)`
  } catch {
    // Found as text only: no host to show.
    return '… (stored as a secret)'
  }
}

/** A server name as validation takes it: `^[a-z0-9][a-z0-9-]{0,23}$`. */
export function slugName(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9-]/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 24)
    .replace(/-$/, '')
  return slug || 'server'
}

export function parseImport(text: string, existing: string[]): ImportResult {
  if (/^\[mcp_servers\./m.test(text)) return fail('codex-toml', 'Paste JSON, or add the server by hand')
  let root: unknown
  try {
    root = JSON.parse(text)
  } catch {
    // Never the parser's message: it quotes the input.
    return fail('unknown', 'Not valid JSON')
  }
  const found = detect(root)
  if ('rows' in found) return found
  const used = new Set<string>()
  const rows = found.servers.map(([originalName, value]): ImportRow => {
    // An earlier row of the paste holding the slug gives `-2`, `-3`…
    const name = originalName === '' ? '' : free(slugName(originalName), '-', 24, (n) => used.has(n))
    used.add(name)
    const built = isObject(value)
      ? (skipReason(value, found.gemini === true) ??
        build(value, name, found.inputs ?? new Map(), found.source === 'registry'))
      : 'Not a server entry'
    if (typeof built === 'string') {
      return {
        originalName,
        name,
        status: 'skipped',
        skipReason: built,
        transport: null,
        target: '',
        entry: {},
        secrets: [],
        needs: [],
        envRefs: [],
        ignored: [],
        notes: [],
      }
    }
    return {
      originalName,
      name,
      status: existing.includes(name) ? 'replaces' : 'new',
      transport: built.entry.type as 'http' | 'stdio',
      target: targetOf(built.entry, built.secrets),
      ...built,
      envRefs: envRefsOf(built.entry),
      notes: [...new Set([...(found.notes ?? []), ...built.notes])],
    }
  })
  return { source: found.source, sourceLabel: found.gemini ? 'Gemini CLI' : LABELS[found.source], rows }
}

/** A secret as the preview shows it: its length only. */
export function maskSecret(value: string): string {
  const length = [...value].length
  return `•••••••• · ${length} character${length === 1 ? '' : 's'}`
}

/** The entry to save: each secret's position holds `secret://<vaultName>`, each need the value typed for
 * its path (a password one stored in the vault unless it is itself a reference). `row` is not changed. */
export function finalEntry(
  row: ImportRow,
  values: Record<string /* need path */, string>,
): { entry: Record<string, unknown>; toStore: { name: string; value: string }[]; toShare: string[] } {
  const entry = structuredClone(row.entry)
  const toStore = new Map<string, string>()
  const set = (path: string, value: string) => {
    const keys = keysOf(path)
    const last = keys.pop() as string
    let at = entry
    for (const key of keys) at = at[key] as Json
    at[last] = value
  }
  const store = (base: string, value: string) => {
    const name = free(base, '_', 128, (n) => toStore.has(n) && toStore.get(n) !== value)
    toStore.set(name, value)
    return `secret://${name}`
  }
  for (const s of row.secrets) set(s.path, store(s.vaultName, s.value))
  for (const n of row.needs) {
    const value = values[n.path] ?? ''
    const vault = n.password && value !== '' && !REFERENCE.test(value)
    set(n.path, vault ? store(secretVaultName(row.name, fieldOf(n.path, row.entry.args)), value) : value)
  }
  return {
    entry,
    toStore: [...toStore].map(([name, value]) => ({ name, value })),
    toShare: envRefsOf(entry),
  }
}

const NAME = /^[a-z0-9][a-z0-9-]{0,23}$/
const VAULT_NAME = /^[A-Za-z_][A-Za-z0-9_.-]{0,127}$/

/** The row under a new name, stored trimmed as typed: the status follows `existing` (a skip stays a skip), and
 * a vault name still at its default for the old name follows the new one. */
export function renameRow(row: ImportRow, name: string, existing: string[]): ImportRow {
  const next = name.trim()
  const secrets = row.secrets.map((s) => {
    const field = fieldOf(s.path, row.entry.args)
    return s.vaultName === secretVaultName(row.name, field) ? { ...s, vaultName: secretVaultName(next, field) } : s
  })
  const status = row.status === 'skipped' ? 'skipped' : existing.includes(next) ? 'replaces' : 'new'
  return { ...row, name: next, status, secrets }
}

/** The text each need's input starts with: a password need starts empty, since its `original` is at most a
 * mask, and a mask stored in the vault would replace the secret; any other need starts from its `original`. */
export function startValues(row: ImportRow): Record<string, string> {
  return Object.fromEntries(row.needs.map((n) => [n.path, n.password ? '' : n.original]))
}

/** What stops one row from being imported, in the order the person fixes it; empty when nothing does. */
export function rowBlockers(row: ImportRow, values: Record<string, string>): string[] {
  if (row.status === 'skipped') return []
  const out: string[] = []
  if (row.name === '') out.push('Give this server a name')
  else if (!NAME.test(row.name)) out.push('Name: 1 to 24 characters, a-z, 0-9 and -, starting with a letter or a digit')
  for (const n of row.needs) if ((values[n.path] ?? '').trim() === '') out.push(`Fill in ${n.label}`)
  for (const s of row.secrets) if (!VAULT_NAME.test(s.vaultName)) out.push(`Vault name \`${s.vaultName}\` is not valid`)
  const clashes = row.secrets.filter((s, i) =>
    row.secrets.some((t, j) => j < i && t.vaultName === s.vaultName && t.value !== s.value),
  )
  for (const name of new Set(clashes.map((s) => s.vaultName))) out.push(`Two secrets share the vault name \`${name}\``)
  return out
}

/** What stops each row because of an earlier one in the same import: a name, or a vault name held with another
 * value. One list per row, by index; a skipped row takes no part. */
export function crossRowBlockers(rows: ImportRow[]): string[][] {
  const names = new Set<string>()
  const vault = new Map<string, string>()
  return rows.map((row) => {
    if (row.status === 'skipped') return []
    const out: string[] = []
    const name = row.name.trim()
    if (name !== '' && names.has(name)) out.push(`Another server in this import is named \`${name}\``)
    names.add(name)
    const mine = new Map(row.secrets.map((s) => [s.vaultName, s.value]))
    for (const [n, value] of mine) {
      if (vault.has(n) && vault.get(n) !== value)
        out.push(`Vault name \`${n}\` is used by another server in this import`)
    }
    for (const [n, value] of mine) if (!vault.has(n)) vault.set(n, value)
    return out
  })
}

/** `message` with every value of 4 characters or more, as written or JSON-escaped, replaced by `[redacted]`. */
export function scrub(message: string, values: string[]): string {
  const hidden = values.flatMap((v) => [v, JSON.stringify(v).slice(1, -1)]).filter((v) => v.length >= 4)
  // Longest first, so a value that holds another is never left half shown.
  return hidden.sort((a, b) => b.length - a.length).reduce((text, v) => text.split(v).join('[redacted]'), message)
}

/** Consumers a store call should leave in place: the existing ones, then the required ones, once each. */
export function mergeConsumers(existing: readonly string[] | undefined, required: readonly string[]): string[] {
  return [...new Set([...(existing ?? []), ...required])]
}

export interface ImportDeps {
  storeSecret(name: string, value: string): Promise<void>
  shareEnv(name: string): Promise<void>
  save(name: string, entry: Record<string, unknown>): Promise<{ saved: boolean; errors: Issue[]; message?: string }>
}

export interface RowOutcome {
  name: string
  imported: boolean
  message: string
}

/** One row: its secrets stored, its env references shared, then the entry saved. The first failure ends the row
 * with nothing more written, and no message holds a pasted or typed value. */
export async function importRow(deps: ImportDeps, row: ImportRow, values: Record<string, string>): Promise<RowOutcome> {
  const hidden = [...row.secrets.map((s) => s.value), ...row.needs.map((n) => values[n.path] ?? '')]
  const no = (what: string, text: string): RowOutcome => ({
    name: row.name,
    imported: false,
    message: `Not imported: ${what}${scrub(text, hidden)}`,
  })
  const { entry, toStore, toShare } = finalEntry(row, values)
  for (const { name, value } of toStore) {
    try {
      await deps.storeSecret(name, value)
    } catch (err) {
      return no(`could not store ${name}: `, errorMessage(err))
    }
  }
  for (const name of toShare) {
    try {
      await deps.shareEnv(name)
    } catch (err) {
      return no(`could not share ${name} with mcp-client: `, errorMessage(err))
    }
  }
  try {
    const out = await deps.save(row.name, entry)
    if (out.saved) return { name: row.name, imported: true, message: 'Imported' }
    const first = out.errors[0]
    if (first) return no('', first.path ? `${first.path}: ${first.message}` : first.message)
    return no('', out.message ?? 'the worker did not validate it')
  } catch (err) {
    return no('', errorMessage(err))
  }
}

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`

/** What "Import n servers" will do, one line each: the secrets stored and the env variables shared over the whole
 * import, then each server's own lines (what validation says it will run or send). */
export function importEffects(
  plans: { name: string; store: string[]; share: string[]; lines: string[] }[],
): { server?: string; text: string }[] {
  const store = [...new Set(plans.flatMap((p) => p.store))]
  const share = [...new Set(plans.flatMap((p) => p.share))]
  return [
    ...(store.length > 0
      ? [
          {
            text: `store ${count(store.length, 'secret', 'secrets')} in the vault: ${store.map((n) => `\`${n}\``).join(', ')}`,
          },
        ]
      : []),
    ...share.map((n) => ({ text: `share \`${n}\` with mcp-client` })),
    ...plans.flatMap((p) => p.lines.map((text) => ({ server: p.name, text }))),
  ]
}

/** The announcement once an import ran: `3 servers imported`, or `2 servers imported, 1 not imported`. */
export function importSummary(outcomes: RowOutcome[]): string {
  const done = outcomes.filter((o) => o.imported).length
  const failed = outcomes.length - done
  return `${count(done, 'server', 'servers')} imported${failed > 0 ? `, ${failed} not imported` : ''}`
}
