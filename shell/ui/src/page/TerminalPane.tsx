import { ArrowDown, RefreshCw, SquareTerminal } from 'lucide-react'
import type { ReactNode } from 'react'
import { HoverTip } from './HoverTip'
import type { TerminalSession } from './terminal-session'

export type { TerminalSession } from './terminal-session'
export { useTerminalSession } from './terminal-session'

interface TerminalPaneProps {
  session: TerminalSession
  actions?: ReactNode
}

export function TerminalPane({ session, actions }: TerminalPaneProps) {
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

  return (
    <div className="shui-terminal">
      <div className="shui-terminal-chrome">
        <SquareTerminal aria-hidden className="shui-terminal-chrome-icon" />
        <span>zsh</span>
        <span className={`shui-terminal-state ${status}`}>{status}</span>
        <span className="shui-terminal-chrome-cwd" title={cwd}>
          {cwd}
        </span>
        <span className="shui-terminal-chrome-spacer" />
        {!atBottom ? (
          <HoverTip label="Jump to latest output">
            <button
              type="button"
              className="shui-terminal-action"
              onClick={jumpToLatest}
              aria-label="Jump to latest output"
            >
              <ArrowDown aria-hidden />
            </button>
          </HoverTip>
        ) : null}
        {status === 'disconnected' ? (
          <HoverTip label="Start fresh shell">
            <button
              type="button"
              className="shui-terminal-action"
              onClick={startFresh}
              aria-label="Start fresh shell"
            >
              <RefreshCw aria-hidden />
            </button>
          </HoverTip>
        ) : status === 'exited' || status === 'error' ? (
          <HoverTip label="Restart terminal session">
            <button
              type="button"
              className="shui-terminal-action"
              onClick={restart}
              aria-label="Restart terminal session"
            >
              <RefreshCw aria-hidden />
            </button>
          </HoverTip>
        ) : null}
        {actions}
      </div>
      {error ? <div className="shui-terminal-error">{error}</div> : null}
      <div
        ref={setContainer}
        className="shui-xterm"
        role="application"
        aria-label="Interactive zsh terminal"
      />
    </div>
  )
}
