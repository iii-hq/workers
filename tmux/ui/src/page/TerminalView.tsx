/**
 * The live surface: the tmux client's screen with a toolbar above (where it
 * is: `session › window`) and a status bar below (size, connection, type
 * size). Anything but a running client shows over the screen, never instead
 * of it, so the emulator and its scrollback survive a reconnect.
 */

import {
  Breadcrumb,
  Button,
  EmptyState,
  IconButton,
  StatusBar,
  StatusDot,
  StatusPanel,
  Toolbar,
} from '@iii-dev/console-ui'
import { MAX_FONT_SIZE, MIN_FONT_SIZE, stepFontSize, useTerminalFontSize } from '@iii-workers/terminal-font'
import { Minus, Plus, RotateCcw, SquareTerminal } from 'lucide-react'
import type { ReactNode } from 'react'
import type { TerminalStatus, TmuxTerminal } from './use-terminal'

const STATUS_LABEL: Record<TerminalStatus, string> = {
  loading: 'Loading terminal',
  idle: 'No session',
  connecting: 'Attaching',
  ready: 'Attached',
  ended: 'Detached',
  error: 'Failed',
}

function FontSizeControl() {
  const [fontSize, setFontSize] = useTerminalFontSize()
  return (
    <span
      className="tmux-font"
      title={`Terminal font size (${MIN_FONT_SIZE}–${MAX_FONT_SIZE} px). Ctrl or ⌘ + scroll works too.`}
    >
      <IconButton
        label="Smaller terminal font"
        tooltip={false}
        variant="ghost"
        disabled={fontSize <= MIN_FONT_SIZE}
        onClick={() => setFontSize(stepFontSize(fontSize, -1))}
      >
        <Minus aria-hidden />
      </IconButton>
      <output className="tmux-mono" aria-label="Terminal font size in pixels">
        {fontSize}
      </output>
      <IconButton
        label="Larger terminal font"
        tooltip={false}
        variant="ghost"
        disabled={fontSize >= MAX_FONT_SIZE}
        onClick={() => setFontSize(stepFontSize(fontSize, 1))}
      >
        <Plus aria-hidden />
      </IconButton>
    </span>
  )
}

export interface TerminalViewProps {
  terminal: TmuxTerminal
  sessionName: string | null
  windowLabel: string | null
  /** `tmux::changed` pushes are arriving. */
  live: boolean
  /** tmux is missing or the server has no sessions. */
  empty: { title: string; description: string; action?: { label: string; onClick(): void } } | null
  onReconnect(): void
  toolbarEnd?: ReactNode
}

export function TerminalView({
  terminal,
  sessionName,
  windowLabel,
  live,
  empty,
  onReconnect,
  toolbarEnd,
}: TerminalViewProps) {
  const { status, detail } = terminal
  const crumbs = [
    ...(sessionName ? [{ key: 'session', label: sessionName }] : []),
    ...(windowLabel ? [{ key: 'window', label: windowLabel }] : []),
  ]
  const showEmpty = empty !== null && (status === 'idle' || status === 'error' || status === 'ended')

  return (
    <div className="tmux-terminal">
      <Toolbar aria-label="Terminal" className="tmux-terminal-toolbar" end={toolbarEnd}>
        {crumbs.length > 0 ? (
          <Breadcrumb items={crumbs} emphasizeRoot />
        ) : (
          <span className="tmux-faint">No session attached</span>
        )}
      </Toolbar>
      <div className="tmux-terminal-body">
        <div
          ref={terminal.setContainer}
          className="tmux-xterm"
          data-autofocus="true"
          role="application"
          aria-label={sessionName ? `tmux session ${sessionName}` : 'tmux terminal'}
        />
        {showEmpty && empty ? (
          <div className="tmux-terminal-overlay">
            <EmptyState
              icon={SquareTerminal}
              title={empty.title}
              description={empty.description}
              action={empty.action}
            />
          </div>
        ) : status === 'ended' ? (
          <div className="tmux-terminal-overlay">
            <StatusPanel
              variant="info"
              headline="The console terminal detached"
              detail={detail ?? 'The tmux client is no longer attached.'}
              action={
                <Button size="sm" variant="ghost" onClick={onReconnect}>
                  <RotateCcw aria-hidden />
                  Reconnect
                </Button>
              }
            />
          </div>
        ) : status === 'error' ? (
          <div className="tmux-terminal-overlay">
            <StatusPanel
              variant="alert"
              headline="Could not attach to tmux"
              detail={detail ?? 'The worker refused the terminal.'}
              action={
                <Button size="sm" variant="ghost" onClick={onReconnect}>
                  <RotateCcw aria-hidden />
                  Retry
                </Button>
              }
            />
          </div>
        ) : null}
      </div>
      <StatusBar className="tmux-terminal-statusbar" end={<FontSizeControl />}>
        <span className="tmux-status" role="status">
          <StatusDot
            tone={status === 'ready' ? 'ok' : status === 'error' ? 'alert' : status === 'ended' ? 'warn' : 'ink'}
          />
          {STATUS_LABEL[status]}
        </span>
        {terminal.cols > 0 && terminal.rows > 0 ? (
          <span className="tmux-mono">
            {terminal.cols}×{terminal.rows}
          </span>
        ) : null}
        {status === 'loading' && detail ? <span className="tmux-faint">{detail}</span> : null}
        <span className="tmux-faint">{live ? 'live' : 'snapshot'}</span>
      </StatusBar>
    </div>
  )
}
