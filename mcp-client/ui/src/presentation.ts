import type { AuthKind, Server, ServerState, ServersList, Stage } from './types'

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
