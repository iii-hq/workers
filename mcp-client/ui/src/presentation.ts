import { formatBytes, formatDuration } from '@iii-dev/console-ui/format'
import { type AccessClass, serverAccess } from './access'
import type { AuthKind, EngineFunction, ProcessInfo, Selection, Server, ServerState, ServersList, Stage } from './types'
import type { ServersData } from './use-servers'

/** Server state to the words and tones of the page: the spec's "States and
 * ranges" table. Every string is authored copy; nothing is case-transformed. */

export type Tone = 'ok' | 'accent' | 'warn' | 'alert' | 'ghost'
export type RowActionId =
  | 'sign_in'
  | 'update_key'
  | 'share'
  | 'fix'
  | 'retry'
  | 'restart'
  | 'review'
  | 'turn_on'
  | 'review_access'

export interface RowPresentation {
  dot: Tone
  pulse: boolean
  description: string
  action: { id: RowActionId; label: string } | null
  panel: { variant: 'warn' | 'alert'; headline: string; next: string } | null
}

export const STAGES: readonly { id: Stage; label: string }[] = [
  { id: 'config', label: 'Config' },
  { id: 'approval', label: 'Approval' },
  { id: 'secret', label: 'Secret' },
  { id: 'connect', label: 'Connect' },
  { id: 'authorize', label: 'Authorize' },
  { id: 'tools', label: 'Tools' },
]

const AUTH_WORDS: Record<AuthKind, string> = {
  none: '',
  bearer: 'API key',
  header: 'API key',
  client_credentials: 'Client credentials',
  oauth: 'Signed in',
}

const ATTENTION: readonly ServerState[] = ['needs_login', 'auth_failed', 'config_error', 'unavailable', 'unapproved']

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`

/** The host of an http(s) URL; a reference (`secret://NAME`) is shown as written. */
function where(target: string): string {
  try {
    const url = new URL(target)
    if (url.protocol === 'http:' || url.protocol === 'https:') return url.host
  } catch {
    // not a URL at all: shown as written
  }
  return target
}

function readyText(server: Server): string {
  if (server.transport === 'stdio') {
    const pid = server.process?.pid
    return pid == null ? 'stdio' : `stdio · pid ${pid}`
  }
  const auth = AUTH_WORDS[server.auth]
  const host = where(server.target)
  return auth ? `${host} · ${auth}` : host
}

function exitText(server: Server, now: number): string {
  const proc = server.process
  const exit = proc?.last_exit
  const text =
    exit?.code != null
      ? `Exited ${exit.code}`
      : exit?.signal != null
        ? `Killed by signal ${exit.signal}`
        : 'Not running'
  if (proc?.next_restart_at == null) return text
  return `${text} · restarting in ${Math.max(0, Math.ceil((proc.next_restart_at - now) / 1000))} s`
}

export function presentServer(server: Server, now: number): RowPresentation {
  const quiet = { pulse: false, action: null, panel: null }
  // The worker's own text heads the panel; `next` says what happens or what to do.
  const alert = (description: string, id: RowActionId, label: string, next: string): RowPresentation => ({
    dot: 'alert',
    pulse: false,
    description,
    action: { id, label },
    panel: { variant: 'alert', headline: server.error?.message || description, next },
  })
  switch (server.state) {
    case 'ready':
      return { ...quiet, dot: 'ok', description: readyText(server) }
    case 'connecting': {
      const stage = STAGES.find((s) => s.id === server.stage)
      return {
        ...quiet,
        dot: 'accent',
        pulse: true,
        description: stage ? `Connecting… · ${stage.label}` : 'Connecting…',
      }
    }
    case 'needs_login':
      return {
        dot: 'warn',
        pulse: false,
        description: 'Needs sign-in',
        action: { id: 'sign_in', label: 'Sign in' },
        panel: {
          variant: 'warn',
          headline: 'Sign in to connect',
          next: 'Agents get an error from this server until someone signs in.',
        },
      }
    case 'auth_failed':
      // A client-credentials failure is the token endpoint's answer, not a 401 from the server.
      return alert(
        server.auth === 'client_credentials' ? 'Auth failed' : 'Auth failed · 401',
        'update_key',
        'Update key',
        'Store a working key under the same secret name. The worker connects again by itself.',
      )
    case 'config_error':
      if (server.error?.code === 'secret_not_shared') {
        return alert(
          'Config error · secret not shared',
          'share',
          'Share',
          'Share that secret with mcp-client. The worker connects again by itself.',
        )
      }
      return alert(
        server.error?.message ? `Config error · ${server.error.message}` : 'Config error',
        'fix',
        'Fix',
        'Fix the entry, then save it.',
      )
    case 'unavailable':
      // `servers::list` carries a next-retry time only for a stdio process.
      return server.transport === 'stdio'
        ? alert(exitText(server, now), 'restart', 'Restart', 'The worker restarts it by itself.')
        : alert('Unavailable · retrying', 'retry', 'Retry', 'The worker keeps trying by itself.')
    case 'unapproved':
      return {
        dot: 'warn',
        pulse: false,
        description: 'Review needed · changed outside this page',
        action: { id: 'review', label: 'Review' },
        panel: {
          variant: 'warn',
          headline: 'Review needed',
          next: 'This entry was added or changed outside this page. Nothing runs and no secret is sent until someone approves it.',
        },
      }
    case 'disabled':
      return { ...quiet, dot: 'ghost', description: 'Off', action: { id: 'turn_on', label: 'Turn on' } }
  }
}

/** The Connection view's six stages: the ones the connection passed, the one it is at (in the row's dot),
 * and the ones it has not reached. A draft, or a server stopped with no stage, has reached none. */
export function stageTrack(server: Server | null): { id: Stage; label: string; tone: Tone; pulse: boolean }[] {
  const at = server?.state === 'ready' ? STAGES.length : STAGES.findIndex((s) => s.id === server?.stage)
  const here = server ? presentServer(server, 0) : null
  return STAGES.map((stage, i) => ({
    ...stage,
    tone: i < at ? 'ok' : i === at ? (here?.dot ?? 'ghost') : 'ghost',
    pulse: i === at && here?.pulse === true,
  }))
}

/** What a connected server advertises, for the Connection view's facts. */
export function advertisesText(server: Server): string {
  const { tools_listed, capabilities } = server
  const words = [tools_listed && 'tools', capabilities.resources && 'resources', capabilities.prompts && 'prompts']
  return words.filter(Boolean).join(' · ') || 'none'
}

/** `69 tools` while the server holds only tools (and maybe instructions);
 * `74 functions` once it also has resource functions or prompts. */
export function countLabel(server: Server): string {
  const { tools, resource_functions, prompts } = server
  if (resource_functions.length === 0 && prompts.length === 0) return count(tools.length, 'tool', 'tools')
  const n = tools.length + (server.has_instructions ? 1 : 0) + resource_functions.length + prompts.length
  return count(n, 'function', 'functions')
}

export function statusBarText(list: ServersList): string {
  const { servers } = list
  const ready = servers.filter((s) => s.state === 'ready').length
  const registered = servers.reduce(
    (n, s) =>
      n +
      [...s.tools, ...s.resource_functions, ...s.prompts].filter((fn) => fn.registered).length +
      (s.has_instructions ? 1 : 0),
    0,
  )
  const attention = servers.filter((s) => ATTENTION.includes(s.state)).length
  return [
    count(servers.length, 'server', 'servers'),
    `${ready} ready`,
    `${count(registered, 'function', 'functions')} registered`,
    `${attention} ${attention === 1 ? 'needs' : 'need'} attention`,
  ].join(' · ')
}

const PINNED_WORDS = { connection: 'Connection', process: 'Process', access: 'Agent access' } as const

export interface PathHeads {
  first: string
  firstMeta: string
  second: string | null
  secondMeta: string | null
  third: string | null
  thirdIsId: boolean
}

/** A function's id past `mcp::<server>::`: the path row's third head and column 3's name. */
export const functionName = (server: string, functionId: string): string => functionId.slice(`mcp::${server}::`.length)

/** The three heads of the path row, which together spell the selected function id. */
export function pathHeads(selection: Selection, server: Server | null, total: number): PathHeads {
  const { item } = selection
  return {
    first: 'mcp::',
    firstMeta: count(total, 'server', 'servers'),
    second: selection.server === null ? null : `${selection.server}::`,
    secondMeta: server ? countLabel(server) : null,
    third:
      item === null
        ? null
        : item.kind === 'function'
          ? functionName(selection.server ?? '', item.functionId)
          : PINNED_WORDS[item.kind],
    thirdIsId: item?.kind === 'function',
  }
}

export type PageState = 'loading' | 'config_down' | 'worker_down' | 'first_run' | 'ready'

/** The page-level situation, by which read failed: the spec's page-level states. */
export function pageState(data: ServersData): PageState {
  if (data.loading && data.list === null && data.config === null) return 'loading'
  if (data.configDown) return 'config_down'
  if (data.workerDown) return 'worker_down'
  if (data.list?.servers.length === 0) return 'first_run'
  return 'ready'
}

/** Names in code-unit order, the same in every locale: rows never reorder on a state change. */
export const byName = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0)

export const STATE_WORD: Record<ServerState, string> = {
  ready: 'Ready',
  connecting: 'Connecting',
  needs_login: 'Needs sign-in',
  auth_failed: 'Auth failed',
  config_error: 'Config error',
  unavailable: 'Unavailable',
  unapproved: 'Review needed',
  disabled: 'Off',
}

/** The pinned Connection row: transport, auth and protocol, each only when known. */
export function connectionRowText(server: Server): string {
  return [
    server.transport === 'http' ? 'HTTP' : 'stdio',
    AUTH_WORDS[server.auth],
    server.protocol_version && `protocol ${server.protocol_version}`,
  ]
    .filter(Boolean)
    .join(' · ')
}

export function processRowText(process: ProcessInfo | null, now: number): string {
  if (process?.pid == null) return 'not running'
  const up = process.started_at == null ? null : `up ${formatDuration(now - process.started_at)}`
  return ['running', `pid ${process.pid}`, up].filter(Boolean).join(' · ')
}

export interface FunctionRow {
  functionId: string
  group: 'tools' | 'resources' | 'prompts'
  name: string
  description: string
  chip: 'Writes' | 'Destructive' | null
}

const GROUP_OF = { tool: 'tools', instructions: 'tools', resource: 'resources', prompt: 'prompts' } as const
const CHIP = { read: null, write: 'Writes', destructive: 'Destructive' } as const

/** Column 2's rows, from the classification of access.ts; the description is the engine's. */
export function functionRows(server: Server, functions: EngineFunction[]): FunctionRow[] {
  const described = new Map(functions.map((fn) => [fn.function_id, fn.description ?? '']))
  const unregistered = new Set(
    [...server.tools, ...server.resource_functions, ...server.prompts]
      .filter((fn) => !fn.registered)
      .map((fn) => fn.function_id),
  )
  // `instructions` heads the Tools group; the sort is stable, so the rest keep their order.
  const access = serverAccess(server).sort(
    (a, b) => Number(b.kind === 'instructions') - Number(a.kind === 'instructions'),
  )
  return access.map((fn) => ({
    functionId: fn.functionId,
    group: GROUP_OF[fn.kind],
    name: fn.name,
    description: unregistered.has(fn.functionId)
      ? 'Not registered in the engine'
      : (described.get(fn.functionId) ?? ''),
    chip: CHIP[fn.cls],
  }))
}

/** Rows whose name or description holds the query, case-insensitively, and the count for the search field. */
export function filterRows(rows: FunctionRow[], query: string): { rows: FunctionRow[]; countText: string } {
  const q = query.trim().toLowerCase()
  if (!q) return { rows, countText: String(rows.length) }
  const matches = rows.filter((row) => row.name.toLowerCase().includes(q) || row.description.toLowerCase().includes(q))
  return { rows: matches, countText: `${matches.length} of ${rows.length}` }
}

export interface RowGroup {
  id: FunctionRow['group']
  label: string
  /** Tools count tools only: the instructions row is listed but not counted. */
  count: number
  rows: FunctionRow[]
  /** The quiet line of a group with nothing listed. */
  empty: string | null
  /** A list the worker could not read, in its own words. */
  failed: { headline: string; detail: string } | null
}

const GROUPS = ['tools', 'resources', 'prompts'] as const
const GROUP_LABEL = { tools: 'Tools', resources: 'Resources', prompts: 'Prompts' } as const

/** Tools always shows; Resources and Prompts with their capability, a row or a failed list. While
 * filtering, only groups with a match show. */
export function rowGroups(server: Server, rows: FunctionRow[], query: string): RowGroup[] {
  const filtering = query.trim() !== ''
  const instructions = `mcp::${server.name}::instructions`
  const failed = (headline: string, detail: string | null) => (detail === null ? null : { headline, detail })
  const lists = {
    tools: null,
    resources: failed('Could not list resource templates', server.list_errors.templates),
    prompts: failed('Could not list prompts', server.list_errors.prompts),
  }
  const advertised = { tools: true, resources: server.capabilities.resources, prompts: server.capabilities.prompts }
  const empty = {
    tools: server.tools_listed ? 'No tools. The server lists none.' : 'No tools. Not connected yet.',
    resources: null,
    prompts: lists.prompts ? null : 'No prompts',
  }
  return GROUPS.flatMap((id): RowGroup[] => {
    const mine = rows.filter((row) => row.group === id)
    if (mine.length === 0 && (filtering || !(advertised[id] || lists[id]))) return []
    const count = mine.filter((row) => row.functionId !== instructions).length
    return [
      {
        id,
        label: GROUP_LABEL[id],
        count,
        rows: mine,
        empty: !filtering && count === 0 ? empty[id] : null,
        failed: lists[id],
      },
    ]
  })
}

export type Level = 'servers' | 'contents' | 'detail'

/** The one column a narrow page shows. A page-level state other than loading stands in for every
 * column, so it shows in place of the server list. */
export function narrowLevel(state: PageState, level: Level): Level {
  return state === 'ready' || state === 'loading' ? level : 'contents'
}

/** The summary of the skipped names: `3 tools skipped`, or `3 skipped` once a prompt is among them. */
export function skippedText(server: Server): string | null {
  const { skipped } = server
  if (skipped.length === 0) return null
  if (skipped.some((s) => s.kind === 'prompt')) return `${skipped.length} skipped`
  return `${count(skipped.length, 'tool', 'tools')} skipped`
}

/** `abc…hij`: the head and the tail of a long name, which is where names differ. */
export function middleTruncate(text: string, max: number): string {
  if (text.length <= max) return text
  const keep = max - 1
  return `${text.slice(0, Math.ceil(keep / 2))}…${text.slice(text.length - Math.floor(keep / 2))}`
}

/** One sentence per server that became ready since the previous read; nothing on the first read. */
export function announcements(prev: ServersList | null, next: ServersList): string[] {
  if (prev === null) return []
  const wasReady = new Set(prev.servers.filter((s) => s.state === 'ready').map((s) => s.name))
  return next.servers
    .filter((s) => s.state === 'ready' && !wasReady.has(s.name))
    .map((s) => `${s.name} connected, ${countLabel(s)}`)
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value)

const COMBINED = ['oneOf', 'anyOf', 'allOf', '$ref']

/** Column 3's mono line: the id, then the inputs (a prompt's arguments) and how many are required.
 * A schema still `undefined` is one not read yet; one combined from others (`oneOf`, `$ref`…) with no
 * top-level properties cannot be counted. */
export function metaLine(functionId: string, schema: unknown, kind: FunctionRow['group']): string {
  if (schema === undefined) return functionId
  const properties = isRecord(schema) ? schema.properties : undefined
  if (isRecord(schema) && !isRecord(properties) && COMBINED.some((key) => key in schema)) return functionId
  const names = isRecord(properties) ? Object.keys(properties) : []
  const [one, many] = kind === 'prompts' ? ['argument', 'arguments'] : ['input', 'inputs']
  if (names.length === 0) return `${functionId} · no ${many}`
  const listed = isRecord(schema) && Array.isArray(schema.required) ? schema.required : []
  const required = names.filter((name) => listed.includes(name)).length
  return `${functionId} · ${count(names.length, one, many)}, ${required || 'none'} required`
}

/** A content block the Result half can draw: the worker turns MCP images into `mime` and `data`. */
export type Block = { type: 'text'; text: string } | { type: 'image'; mime: string; data: string }

const utf8 = new TextEncoder()

/** `412 ms` under a second, `1.2 s` from there. */
export function elapsedText(ms: number): string {
  const n = Math.round(ms)
  return n < 1000 ? `${n} ms` : `${(n / 1000).toFixed(1)} s`
}

/** The text and image blocks of a tool result, in order; anything else is left to the raw view. */
export function blocksOf(result: unknown): Block[] {
  const content = isRecord(result) && Array.isArray(result.content) ? result.content : []
  return content.filter(
    (block): block is Block =>
      isRecord(block) &&
      ((block.type === 'text' && typeof block.text === 'string') ||
        (block.type === 'image' && typeof block.mime === 'string' && typeof block.data === 'string')),
  )
}

export interface MessageView {
  role: 'User' | 'Assistant'
  blocks: Block[]
}

/** A prompt result as the Messages view draws it: each message's blocks kept as a tool result's are. */
export function messagesOf(result: unknown): { description: string | null; messages: MessageView[] } {
  const record = isRecord(result) ? result : {}
  const messages = Array.isArray(record.messages) ? record.messages : []
  return {
    description: typeof record.description === 'string' ? record.description : null,
    messages: messages.map((message) => ({
      role: isRecord(message) && message.role === 'assistant' ? 'Assistant' : 'User',
      blocks: blocksOf(message),
    })),
  }
}

/** Where the cursor goes in a resource template: the first `{variable}`, braces included. */
export function firstVariable(template: string): { start: number; end: number } | null {
  const match = /\{[^}]+\}/.exec(template)
  return match ? { start: match.index, end: match.index + match[0].length } : null
}

/** The picker's search filters loaded rows only; it says so while the server holds more pages. */
export function searchNote(loaded: number, more: boolean): string | null {
  return more ? `Searching ${count(loaded, 'loaded resource', 'loaded resources')}` : null
}

/** Whether any of a picker row's texts (its name, its uri) holds the query, case-insensitively. */
export function matchesQuery(query: string, ...texts: string[]): boolean {
  const q = query.trim().toLowerCase()
  return !q || texts.some((text) => text.toLowerCase().includes(q))
}

/** Rows with one per key, the first in place: a list that moved between two pages repeats a row. */
export function uniqueBy<T>(rows: T[], key: (row: T) => string): T[] {
  const seen = new Set<string>()
  return rows.filter((row) => {
    const k = key(row)
    if (seen.has(k)) return false
    seen.add(k)
    return true
  })
}

/** A picker row's trailing meta: the MIME type and the size, each when the server sent it. */
export function resourceMeta(mime: string | undefined, size: number | undefined): string {
  return [mime, size === undefined ? null : formatBytes(size)].filter(Boolean).join(' · ')
}

/** The Result status line: messages for a prompt, blocks and their size for a tool, else the JSON size. */
export function resultSummary(result: unknown, ms: number): string {
  const head = `OK · ${elapsedText(ms)}`
  if (isRecord(result) && Array.isArray(result.messages)) {
    return `${head} · ${count(result.messages.length, 'message', 'messages')}`
  }
  if (isRecord(result) && Array.isArray(result.content)) {
    const blocks = blocksOf(result)
    if (blocks.length === 0) return `${head} · empty`
    const texts = blocks.filter((block) => block.type === 'text').length
    const images = blocks.length - texts
    // An image's size is its decoded bytes, from the base64 length.
    const bytes = blocks.reduce(
      (n, block) =>
        n + (block.type === 'text' ? utf8.encode(block.text).length : Math.floor((block.data.length * 3) / 4)),
      0,
    )
    return [
      head,
      texts > 0 && count(texts, 'text block', 'text blocks'),
      images > 0 && count(images, 'image', 'images'),
      formatBytes(bytes),
    ]
      .filter(Boolean)
      .join(' · ')
  }
  return `${head} · ${formatBytes(utf8.encode(JSON.stringify(result) ?? '').length)}`
}

/** A result drawn whole, as one JSON view: no block to draw and no messages (a resource list page). */
export function drawsWhole(result: unknown): boolean {
  return blocksOf(result).length === 0 && !(isRecord(result) && Array.isArray(result.messages))
}

// shortcut: highlighting 120 KiB freezes the pane; window the block if people read results this long here
const HIGHLIGHT_MAX = 32_768

/** Whether a code view is short enough to highlight; a longer one is drawn as plain text. */
export const highlightable = (code: string): boolean => code.length <= HIGHLIGHT_MAX

/** A text block as pretty JSON when it parses to an object or an array and can be highlighted; else null. */
export function jsonText(text: string): string | null {
  if (!highlightable(text)) return null
  try {
    const parsed: unknown = JSON.parse(text)
    return typeof parsed === 'object' && parsed !== null ? JSON.stringify(parsed, null, 2) : null
  } catch {
    return null
  }
}

/** The question Run asks before a call that can change data; `null` for a read, which runs at once. */
export function runConfirm(
  server: string,
  name: string,
  cls: AccessClass,
): { title: string; tone: 'default' | 'danger' } | null {
  if (cls === 'read') return null
  const effect = cls === 'destructive' ? 'is destructive and can change data' : 'can change data'
  return {
    title: `Run \`${name}\` on ${server}? This tool ${effect}.`,
    tone: cls === 'destructive' ? 'danger' : 'default',
  }
}
