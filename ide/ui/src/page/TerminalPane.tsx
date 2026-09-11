import {
  MAX_FONT_SIZE,
  MIN_FONT_SIZE,
  stepFontSize,
  useTerminalFontSize,
} from '@iii-workers/terminal-font'
import { ArrowDown, Minus, Plus, RefreshCw } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
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
  connecting: 'Starting',
  reconnecting: 'Reconnecting',
  disconnected: 'Disconnected',
  exited: 'Session ended',
  error: 'Session failed',
}

/**
 * The type size, shared with every other terminal in the console: the panes
 * beside this one and the agent pages read the same stored value, so one
 * click here moves all of them.
 */
function FontSizeControl() {
  const [fontSize, setFontSize] = useTerminalFontSize()
  const label =
    `Terminal font size (${MIN_FONT_SIZE}–${MAX_FONT_SIZE} px).` +
    ' Ctrl or ⌘ + scroll works too.'
  return (
    <span className="shui-terminal-font" title={label}>
      <span className="shui-terminal-font-label">Font</span>
      <button
        type="button"
        className="shui-terminal-action"
        onClick={() => setFontSize(stepFontSize(fontSize, -1))}
        disabled={fontSize <= MIN_FONT_SIZE}
        aria-label="Smaller terminal font"
      >
        <Minus aria-hidden />
      </button>
      <output aria-label="Terminal font size in pixels">{fontSize}</output>
      <button
        type="button"
        className="shui-terminal-action"
        onClick={() => setFontSize(stepFontSize(fontSize, 1))}
        disabled={fontSize >= MAX_FONT_SIZE}
        aria-label="Larger terminal font"
      >
        <Plus aria-hidden />
      </button>
    </span>
  )
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
  const [fontSize, setFontSize] = useTerminalFontSize()
  const fontSizeRef = useRef(fontSize)
  fontSizeRef.current = fontSize
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
  useEffect(() => {
    if (!container) return
    const zoom = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return
      event.preventDefault()
      setFontSize(stepFontSize(fontSizeRef.current, event.deltaY < 0 ? 1 : -1))
    }
    container.addEventListener('wheel', zoom, { passive: false })
    return () => container.removeEventListener('wheel', zoom)
  }, [container, setFontSize])

  return (
    <div className="shui-terminal">
      {docked ? (
        <div className="shui-terminal-pane-bar">
          <span className="shui-terminal-pane-cwd" title={cwd}>
            {cwd.split('/').filter(Boolean).slice(-1)[0] ?? cwd}
          </span>
          <FontSizeControl />
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
        <div className="shui-terminal-hud">
          <FontSizeControl />
          {actions}
        </div>
      )}
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
