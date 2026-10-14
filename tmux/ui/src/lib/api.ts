/**
 * The page's calls into the tmux worker. Every function here is one bus
 * call with a bounded timeout; callers own the error presentation.
 */

import type { ExtensionIii } from '@iii-dev/console-ui'
import type {
  CreateSessionResponse,
  CreateWindowResponse,
  PortsSnapshot,
  SessionsSnapshot,
  TerminalOpenResponse,
  TerminalReplayResponse,
} from './types'

const READ_TIMEOUT_MS = 10_000
const MUTATION_TIMEOUT_MS = 15_000
const TERMINAL_TIMEOUT_MS = 10_000

export interface TerminalCredentials {
  terminalId: string
  accessKey: string
}

function credentials(terminal: TerminalCredentials) {
  return { terminal_id: terminal.terminalId, access_key: terminal.accessKey }
}

export function createApi(iii: ExtensionIii) {
  const call = <T>(fn: string, payload: Record<string, unknown>, timeoutMs: number) =>
    iii.trigger<T>(fn, payload, { timeoutMs })

  return {
    listSessions: () => call<SessionsSnapshot>('tmux::sessions::list', {}, READ_TIMEOUT_MS),
    listPorts: () => call<PortsSnapshot>('tmux::ports::list', {}, READ_TIMEOUT_MS),

    createSession: (request: Record<string, unknown>) =>
      call<CreateSessionResponse>('tmux::sessions::create', request, MUTATION_TIMEOUT_MS),
    renameSession: (session: string, name: string) =>
      call<{ session_id: string; name: string }>('tmux::sessions::rename', { session, name }, MUTATION_TIMEOUT_MS),
    killSession: (session: string) => call<{ killed: boolean }>('tmux::sessions::kill', { session }, MUTATION_TIMEOUT_MS),

    createWindow: (request: Record<string, unknown>) =>
      call<CreateWindowResponse>('tmux::windows::create', request, MUTATION_TIMEOUT_MS),
    selectWindow: (session: string, window: string) =>
      call<{ session_id: string; window_id: string }>('tmux::windows::select', { session, window }, MUTATION_TIMEOUT_MS),
    renameWindow: (session: string, window: string, name: string) =>
      call<{ window_id: string; name: string }>('tmux::windows::rename', { session, window, name }, MUTATION_TIMEOUT_MS),
    killWindow: (session: string, window: string) =>
      call<{ killed: boolean }>('tmux::windows::kill', { session, window }, MUTATION_TIMEOUT_MS),

    openTerminal: (request: {
      session: string
      window?: string
      cols: number
      rows: number
      output_function_id: string
    }) => call<TerminalOpenResponse>('tmux::terminal::open', request, TERMINAL_TIMEOUT_MS),
    writeTerminal: (terminal: TerminalCredentials, data: string) =>
      call<{ written: number }>('tmux::terminal::write', { ...credentials(terminal), data }, TERMINAL_TIMEOUT_MS),
    resizeTerminal: (terminal: TerminalCredentials, cols: number, rows: number) =>
      call<{ cols: number; rows: number }>(
        'tmux::terminal::resize',
        { ...credentials(terminal), cols, rows },
        TERMINAL_TIMEOUT_MS,
      ),
    switchTerminal: (terminal: TerminalCredentials, session: string, window?: string) =>
      call<{ session_id: string }>(
        'tmux::terminal::switch',
        window === undefined ? { ...credentials(terminal), session } : { ...credentials(terminal), session, window },
        TERMINAL_TIMEOUT_MS,
      ),
    replayTerminal: (terminal: TerminalCredentials, afterSequence: number) =>
      call<TerminalReplayResponse>(
        'tmux::terminal::replay',
        { ...credentials(terminal), after_sequence: afterSequence },
        TERMINAL_TIMEOUT_MS,
      ),
    redrawTerminal: (terminal: TerminalCredentials) =>
      call<{ ok: boolean }>('tmux::terminal::redraw', credentials(terminal), TERMINAL_TIMEOUT_MS),
    closeTerminal: (terminal: TerminalCredentials) =>
      call<{ closed: boolean }>('tmux::terminal::close', credentials(terminal), TERMINAL_TIMEOUT_MS),
  }
}

export type TmuxApi = ReturnType<typeof createApi>
