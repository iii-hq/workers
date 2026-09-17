import { Tooltip } from '@iii-dev/console-ui'
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
  useEffect,
  useRef,
  useState,
} from 'react'
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
import { useSplitDrag } from './use-split-drag'

interface ResizeState {
  startSize: number
  maxSize: number
}

interface TerminalPanelSharedProps {
  dock: TerminalDock
  size: number
  onDockChange: (dock: TerminalDock) => void
  onSizeChange: (size: number) => void
  onClose: () => void
  /** The page is narrow: a right dock stacks under the editor at full width. */
  narrow?: boolean
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
      <span className="shui-terminal-dock-group">
      <Tooltip label="Dock terminal at bottom">
        <button
          type="button"
          className={`shui-terminal-action${dock === 'bottom' ? ' active' : ''}`}
          onClick={() => onDockChange('bottom')}
          aria-label="Dock terminal at bottom"
          aria-pressed={dock === 'bottom'}
        >
          <PanelBottom aria-hidden />
        </button>
      </Tooltip>
      <Tooltip label="Dock terminal on right">
        <button
          type="button"
          className={`shui-terminal-action${dock === 'right' ? ' active' : ''}`}
          onClick={() => onDockChange('right')}
          aria-label="Dock terminal on right"
          aria-pressed={dock === 'right'}
        >
          <PanelRight aria-hidden />
        </button>
      </Tooltip>
      <Tooltip label="Open terminal as an editor tab">
        <button
          type="button"
          className={`shui-terminal-action${dock === 'editor' ? ' active' : ''}`}
          onClick={() => onDockChange('editor')}
          aria-label="Open terminal as an editor tab"
          aria-pressed={dock === 'editor'}
        >
          <SquareTerminal aria-hidden />
        </button>
      </Tooltip>
      </span>
      <Tooltip label="Hide terminal">
        <button
          type="button"
          className="shui-terminal-action"
          onClick={onClose}
          aria-label="Hide terminal"
        >
          <X aria-hidden />
        </button>
      </Tooltip>
    </>
  )
}

export function TerminalPanel(props: TerminalPanelProps) {
  const { dock, size, onDockChange, onSizeChange, onClose, narrow } = props
  const panelRef = useRef<HTMLElement>(null)
  const workspaceRef = useRef<TerminalWorkspaceHandle>(null)
  const [resizeBounds, setResizeBounds] = useState({ size, max: 1200 })
  const docked = dock !== 'editor'
  const style = narrow && dock === 'right' ? undefined : terminalPanelStyle(dock, size)

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

  const maxSizeOf = (frame: Element | null | undefined) => {
    const rect = frame?.getBoundingClientRect()
    return dock === 'bottom'
      ? Math.max(160, (rect?.height ?? window.innerHeight) - 120)
      : Math.max(160, (rect?.width ?? window.innerWidth) - 240)
  }
  const currentSizeOf = (panel: Element | null) => {
    const rect = panel?.getBoundingClientRect()
    return dock === 'bottom' ? (rect?.height ?? size) : (rect?.width ?? size)
  }
  const resizer = useSplitDrag<ResizeState>({
    horizontal: dock === 'right',
    begin: (event) => {
      if (!docked) return null
      const panel = event.currentTarget.parentElement
      return { startSize: currentSizeOf(panel), maxSize: maxSizeOf(panel?.parentElement) }
    },
    move: (origin, delta) => onSizeChange(clampSize(origin.startSize - delta, origin.maxSize)),
    // Up/Left grow the panel: its free edge faces the start of the axis.
    step: (direction, event) => {
      const panel = event.currentTarget.parentElement
      onSizeChange(clampSize(currentSizeOf(panel) + (direction === -1 ? 16 : -16), maxSizeOf(panel?.parentElement)))
    },
  })

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
          {...resizer}
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
      <TerminalWorkspace
        ref={workspaceRef}
        actions={
          <>
            <Tooltip label="Close disconnected terminals">
              <button
                type="button"
                className="shui-terminal-action"
                onClick={() => void workspaceRef.current?.closeDisconnected()}
                aria-label="Close disconnected terminals"
              >
                <Trash2 aria-hidden />
              </button>
            </Tooltip>
            <DockActions
              dock={dock}
              onDockChange={onDockChange}
              onClose={onClose}
            />
          </>
        }
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
