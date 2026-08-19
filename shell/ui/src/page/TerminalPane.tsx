import { ArrowDown, RefreshCw } from 'lucide-react'
import type { ReactNode } from 'react'
import { HoverTip } from './HoverTip'
import type { TerminalSession } from './terminal-session'

export type { TerminalSession } from './terminal-session'
export { useTerminalSession } from './terminal-session'

interface TerminalPaneProps {
  session: TerminalSession
  actions?: ReactNode
  /** Split panes need identity and controls that never sit over output. */
  docked?: boolean
}

const STATUS_LABEL: Record<string, string> = {
  connecting: 'starting',
  reconnecting: 'reconnecting',
  disconnected: 'disconnected',
  exited: 'session ended',
  error: 'session failed',
}

export function TerminalPane({ session, actions, docked }: TerminalPaneProps) {
  const {
    atBottom,
    cwd,
    error,
    jumpToLatest,
    restart,
    setContainer,
    startFresh,
    status,
  } = session
  const settled = status === 'ready'
  const recoverable =
    status === 'disconnected' || status === 'exited' || status === 'error'

  return (
    <div className="shui-terminal">
      {docked ? (
        <div className="shui-terminal-pane-bar">
          <span className="shui-terminal-pane-cwd" title={cwd}>
            {cwd.split('/').filter(Boolean).slice(-1)[0] ?? cwd}
          </span>
          {actions}
        </div>
      ) : null}
      {!settled ? (
        <div className={`shui-terminal-status ${status}`} role="status">
          <span>{STATUS_LABEL[status] ?? status}</span>
          {recoverable ? (
            <button
              type="button"
              className="shui-terminal-status-action"
              onClick={status === 'disconnected' ? startFresh : restart}
            >
              <RefreshCw aria-hidden />
              {status === 'disconnected' ? 'start fresh' : 'restart'}
            </button>
          ) : null}
        </div>
      ) : null}
      {error ? <div className="shui-terminal-error">{error}</div> : null}
      <div
        ref={setContainer}
        className="shui-xterm"
        role="application"
        aria-label="Interactive zsh terminal"
      />
      {docked ? null : <div className="shui-terminal-hud">{actions}</div>}
      {!atBottom ? (
        <HoverTip label="Jump to latest output">
          <button
            type="button"
            className="shui-terminal-jump"
            onClick={jumpToLatest}
            aria-label="Jump to latest output"
          >
            <ArrowDown aria-hidden />
          </button>
        </HoverTip>
      ) : null}
    </div>
  )
}
