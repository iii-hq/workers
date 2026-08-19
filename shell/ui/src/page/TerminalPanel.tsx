import {
  PanelBottom,
  PanelRight,
  SquareTerminal,
  Trash2,
  X,
} from 'lucide-react'
import {
  type CSSProperties,
  type Dispatch,
  type KeyboardEvent,
  type PointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react'
import { HoverTip } from './HoverTip'
import type { TerminalDock } from './persist'
import {
  TerminalWorkspace,
  type TerminalWorkspaceHandle,
} from './TerminalWorkspace'
import type {
  TerminalWorkspaceAction,
  TerminalWorkspaceState,
} from './terminal-layout'
import type { TerminalOutputRouter } from './terminal-output-router'
import type { TerminalConnectionCoordinator } from './terminal-session-state'

interface ResizeState {
  start: number
  startSize: number
  maxSize: number
}

interface TerminalPanelSharedProps {
  dock: TerminalDock
  size: number
  onDockChange: (dock: TerminalDock) => void
  onSizeChange: (size: number) => void
  onClose: () => void
}

interface TerminalPanelProps extends TerminalPanelSharedProps {
  state: TerminalWorkspaceState
  dispatch: Dispatch<TerminalWorkspaceAction>
  root: string
  visible: boolean
  router: TerminalOutputRouter | null
  leaseStore: Storage | null
  storageKey: string
  connectionCoordinators: Map<string, TerminalConnectionCoordinator>
}

function clampSize(size: number, maxSize: number): number {
  return Math.min(Math.max(160, maxSize), Math.max(160, Math.round(size)))
}

function terminalPanelStyle(
  dock: TerminalDock,
  size: number,
): CSSProperties | undefined {
  switch (dock) {
    case 'bottom':
      return { height: size }
    case 'right':
      return { width: size }
    case 'editor':
      return undefined
    default: {
      const exhaustive: never = dock
      return exhaustive
    }
  }
}

function DockActions({
  dock,
  onDockChange,
  onClose,
}: Pick<TerminalPanelSharedProps, 'dock' | 'onDockChange' | 'onClose'>) {
  return (
    <>
      <HoverTip label="Dock terminal at bottom">
        <button
          type="button"
          className={`shui-terminal-action${dock === 'bottom' ? ' active' : ''}`}
          onClick={() => onDockChange('bottom')}
          aria-label="Dock terminal at bottom"
          aria-pressed={dock === 'bottom'}
        >
          <PanelBottom aria-hidden />
        </button>
      </HoverTip>
      <HoverTip label="Dock terminal on right">
        <button
          type="button"
          className={`shui-terminal-action${dock === 'right' ? ' active' : ''}`}
          onClick={() => onDockChange('right')}
          aria-label="Dock terminal on right"
          aria-pressed={dock === 'right'}
        >
          <PanelRight aria-hidden />
        </button>
      </HoverTip>
      <HoverTip label="Open terminal as an editor tab">
        <button
          type="button"
          className={`shui-terminal-action${dock === 'editor' ? ' active' : ''}`}
          onClick={() => onDockChange('editor')}
          aria-label="Open terminal as an editor tab"
          aria-pressed={dock === 'editor'}
        >
          <SquareTerminal aria-hidden />
        </button>
      </HoverTip>
      <HoverTip label="Hide terminal">
        <button
          type="button"
          className="shui-terminal-action"
          onClick={onClose}
          aria-label="Hide terminal"
        >
          <X aria-hidden />
        </button>
      </HoverTip>
    </>
  )
}

export function TerminalPanel(props: TerminalPanelProps) {
  const { dock, size, onDockChange, onSizeChange, onClose } = props
  const resizeRef = useRef<ResizeState | null>(null)
  const panelRef = useRef<HTMLElement>(null)
  const workspaceRef = useRef<TerminalWorkspaceHandle>(null)
  const [resizeBounds, setResizeBounds] = useState({ size, max: 1200 })
  const docked = dock !== 'editor'
  const style = terminalPanelStyle(dock, size)

  useEffect(() => {
    const panel = panelRef.current
    const frame = panel?.parentElement
    if (!panel || !frame || !docked) return
    const update = () => {
      const panelRect = panel.getBoundingClientRect()
      const frameRect = frame.getBoundingClientRect()
      setResizeBounds({
        size: dock === 'bottom' ? panelRect.height : panelRect.width,
        max:
          dock === 'bottom'
            ? Math.max(160, frameRect.height - 120)
            : Math.max(160, frameRect.width - 240),
      })
    }
    update()
    const observer = new ResizeObserver(update)
    observer.observe(panel)
    observer.observe(frame)
    return () => observer.disconnect()
  }, [dock, docked])

  const onPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (!docked) return
      const panel = event.currentTarget.parentElement
      const frame = panel?.parentElement
      const panelRect = panel?.getBoundingClientRect()
      const frameRect = frame?.getBoundingClientRect()
      resizeRef.current = {
        start: dock === 'bottom' ? event.clientY : event.clientX,
        startSize:
          dock === 'bottom'
            ? (panelRect?.height ?? size)
            : (panelRect?.width ?? size),
        maxSize:
          dock === 'bottom'
            ? Math.max(160, (frameRect?.height ?? window.innerHeight) - 120)
            : Math.max(160, (frameRect?.width ?? window.innerWidth) - 240),
      }
      event.preventDefault()
      event.currentTarget.setPointerCapture(event.pointerId)
    },
    [dock, docked, size],
  )

  const onPointerMove = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      const resize = resizeRef.current
      if (!resize) return
      const current = dock === 'bottom' ? event.clientY : event.clientX
      onSizeChange(
        clampSize(resize.startSize - (current - resize.start), resize.maxSize),
      )
    },
    [dock, onSizeChange],
  )

  const onPointerUp = useCallback((event: PointerEvent<HTMLDivElement>) => {
    resizeRef.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }, [])

  const onResizeKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      const grow =
        (dock === 'bottom' && event.key === 'ArrowUp') ||
        (dock === 'right' && event.key === 'ArrowLeft')
      const shrink =
        (dock === 'bottom' && event.key === 'ArrowDown') ||
        (dock === 'right' && event.key === 'ArrowRight')
      if (!grow && !shrink) return
      event.preventDefault()
      const frame = event.currentTarget.parentElement?.parentElement
      const panelRect =
        event.currentTarget.parentElement?.getBoundingClientRect()
      const frameRect = frame?.getBoundingClientRect()
      const maxSize =
        dock === 'bottom'
          ? Math.max(160, (frameRect?.height ?? window.innerHeight) - 120)
          : Math.max(160, (frameRect?.width ?? window.innerWidth) - 240)
      const currentSize =
        dock === 'bottom'
          ? (panelRect?.height ?? size)
          : (panelRect?.width ?? size)
      onSizeChange(clampSize(currentSize + (grow ? 16 : -16), maxSize))
    },
    [dock, onSizeChange, size],
  )

  return (
    <section
      ref={panelRef}
      className="shui-terminal-panel"
      data-terminal-dock={dock}
      style={style}
      aria-label="Terminal"
    >
      {docked ? (
        // biome-ignore lint/a11y/useSemanticElements: this is an interactive range separator, not a static thematic break.
        <div
          role="separator"
          tabIndex={0}
          className="shui-terminal-resize"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onLostPointerCapture={onPointerUp}
          onKeyDown={onResizeKeyDown}
          aria-label={`Resize ${dock} terminal`}
          aria-orientation={dock === 'bottom' ? 'horizontal' : 'vertical'}
          aria-valuemin={160}
          aria-valuemax={Math.round(resizeBounds.max)}
          aria-valuenow={Math.round(resizeBounds.size)}
          title="Drag to resize terminal"
        >
          <span aria-hidden />
        </div>
      ) : null}
      <div className="shui-terminal-panel-chrome">
        <SquareTerminal aria-hidden />
        <span>Terminal workspace</span>
        <span className="shui-terminal-chrome-spacer" />
        <HoverTip label="Close disconnected terminals">
          <button
            type="button"
            className="shui-terminal-action"
            onClick={() => void workspaceRef.current?.closeDisconnected()}
            aria-label="Close disconnected terminals"
          >
            <Trash2 aria-hidden />
          </button>
        </HoverTip>
        <DockActions
          dock={dock}
          onDockChange={onDockChange}
          onClose={onClose}
        />
      </div>
      <TerminalWorkspace
        ref={workspaceRef}
        state={props.state}
        dispatch={props.dispatch}
        root={props.root}
        visible={props.visible}
        router={props.router}
        leaseStore={props.leaseStore}
        storageKey={props.storageKey}
        connectionCoordinators={props.connectionCoordinators}
      />
    </section>
  )
}
