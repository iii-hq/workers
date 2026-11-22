/** Reads an MCP config pasted from another client into rows the page can add. Pure: it runs nothing,
 * reads no file or helper a paste names, and never puts a pasted value into a message. */

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

interface Found {
  source: SourceKind
  servers: [string, unknown][]
  gemini?: boolean
  inputs?: Map<string, Json>
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
    return { source: 'registry', servers: remotes.map((s): [string, unknown] => [name, s]) }
  }
  if (hasTarget(r)) return { source: 'single', servers: [[str(r.name), r]] }
  const values = Object.values(r)
  if (values.length > 0 && values.every(hasTarget)) return { source: 'bare-map', servers: Object.entries(r) }
  return fail('unknown', 'No MCP servers found in this JSON')
}

function skipReason(value: unknown, gemini: boolean): string | undefined {
  if (!isObject(value)) return 'Not a server entry'
  if (value.type === 'sse') return 'SSE is not supported'
  if (value.type === 'ws') return 'WebSocket is not supported'
  if (value.type === 'sdk') return 'In-process SDK servers cannot be imported'
  if (gemini && typeof value.url === 'string' && value.httpUrl === undefined && value.type === undefined) {
    return 'Gemini\'s "url" is an SSE endpoint, which is not supported'
  }
  if (!hasTarget(value)) return 'No command and no url'
  return undefined
}

/** The fields mcp-client does not take, by name only: a value is never copied. */
function ignoredOf(e: Json, inputs: Map<string, Json>): Ignored[] {
  const out: Ignored[] = []
  const sub = (key: string, value: Json, keep: Set<string>) => {
    for (const k of Object.keys(value)) {
      if (!keep.has(k))
        out.push(ig(`${key}.${k}`, key === 'oauth' && CALLBACK.has(k) ? "the other client's callback" : undefined))
    }
  }
  for (const [key, value] of Object.entries(e)) {
    if (CONSUMED.has(key)) continue
    if (key === 'oauth' && isObject(value)) sub(key, value, OAUTH_KEYS)
    else if (key === 'auth' && isObject(value) && 'CLIENT_ID' in value) sub(key, value, CURSOR_AUTH_KEYS)
    else if (!(key === 'auth' && isObject(value) && AUTH_MODES.has(str(value.type)))) out.push(ig(key, NOTES.get(key)))
  }
  for (const [name] of [...pairs(e.headers), ...pairs(e.http_headers), ...pairs(e.env_http_headers)]) {
    if (RESERVED.test(name)) out.push(ig(`headers.${name}`, 'set by mcp-client'))
  }
  const text = JSON.stringify(e)
  for (const [id, input] of inputs) {
    if (input.type === 'command' && text.includes(`\${input:${id}}`)) out.push(ig(`inputs.${id}`, 'not run'))
  }
  return out
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

/** `base`, or `base-2`, `base-3`… cut to stay within 24 when an earlier row of the paste holds it. */
function unique(base: string, used: Set<string>): string {
  let name = base
  for (let n = 2; base !== '' && used.has(name); n++) name = `${base.slice(0, 24 - `-${n}`.length)}-${n}`
  used.add(name)
  return name
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
    const name = unique(originalName === '' ? '' : slugName(originalName), used)
    const blank = { originalName, name, target: '', entry: {}, secrets: [], needs: [], envRefs: [], notes: [] }
    const reason = skipReason(value, found.gemini === true)
    if (reason || !isObject(value)) {
      return { ...blank, status: 'skipped', skipReason: reason, transport: null, ignored: [] }
    }
    return {
      ...blank,
      status: existing.includes(name) ? 'replaces' : 'new',
      transport: typeof value.command === 'string' ? 'stdio' : 'http',
      ignored: ignoredOf(value, found.inputs ?? new Map()),
    }
  })
  return { source: found.source, sourceLabel: found.gemini ? 'Gemini CLI' : LABELS[found.source], rows }
}
