import { readFontSize, stepFontSize, writeFontSize } from '@iii-workers/terminal-font'
import { StatusBar, Tooltip } from '@iii-dev/console-ui'
import { ArrowDown, RefreshCw } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useState } from 'react'
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
  connecting: 'Starting',
  reconnecting: 'Reconnecting',
  disconnected: 'Disconnected',
  exited: 'Session ended',
  error: 'Session failed',
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
  // The xterm host element, kept here as well so the wheel gesture can bind to
  // it; `setContainer` is the session's own callback ref.
  const [container, setContainerNode] = useState<HTMLDivElement | null>(null)
  const attach = useCallback(
    (node: HTMLDivElement | null) => {
      setContainerNode(node)
      setContainer(node)
    },
    [setContainer],
  )

  // Ctrl/⌘ + scroll, the gesture every terminal emulator already answers to.
  //
  // A native listener with `{ passive: false }`, not React's `onWheel`: React
  // registers wheel handlers as passive, so `preventDefault()` inside one is
  // ignored and the browser zooms the whole page underneath the terminal.
  // The size is read at the gesture, not subscribed to: the pane itself never
  // renders it.
  useEffect(() => {
    if (!container) return
    const zoom = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return
      event.preventDefault()
      writeFontSize(stepFontSize(readFontSize(), event.deltaY < 0 ? 1 : -1))
    }
    container.addEventListener('wheel', zoom, { passive: false })
    return () => container.removeEventListener('wheel', zoom)
  }, [container])

  return (
    <div className="shui-terminal">
      {docked ? (
        <StatusBar
          className="shui-terminal-pane-bar"
          end={actions}
        >
          <span className="shui-terminal-pane-cwd" title={cwd}>
            {cwd.split('/').filter(Boolean).slice(-1)[0] ?? cwd}
          </span>
        </StatusBar>
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
              {status === 'disconnected' ? 'Start fresh' : 'Restart'}
            </button>
          ) : null}
        </div>
      ) : null}
      {error ? <div className="shui-terminal-error">{error}</div> : null}
      <div
        ref={attach}
        className="shui-xterm"
        role="application"
        aria-label="Interactive zsh terminal"
      />
      {docked ? null : (
        <div className="shui-terminal-hud">{actions}</div>
      )}
      {!atBottom ? (
        <Tooltip label="Jump to latest output">
          <button
            type="button"
            className="shui-terminal-jump"
            onClick={jumpToLatest}
            aria-label="Jump to latest output"
          >
            <ArrowDown aria-hidden />
          </button>
        </Tooltip>
      ) : null}
    </div>
  )
}
