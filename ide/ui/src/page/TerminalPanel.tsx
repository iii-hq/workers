import { Tooltip } from '@iii-dev/console-ui'
import {
  PanelBottom,
  PanelRight,
  SquareTerminal,
  Trash2,
  X,
} from 'lucide-react'
import { type Dispatch, memo, useRef } from 'react'
import { DockPanel } from './DockPanel'
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

interface TerminalPanelSharedProps {
  dock: TerminalDock
  size: number
  onDockChange: (dock: TerminalDock) => void
  onSizeChange: (size: number) => void
  onClose: () => void
  /** The page is narrow: a right dock stacks under the editor at full width. */
  narrow?: boolean
  /**
   * Hidden rather than unmounted: the shells stay attached and every pane
   * keeps its scrollback, where unmounting detached them all and showing the
   * panel again replayed each one's output from the start.
   */
  hidden?: boolean
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

function TerminalPanelView(props: TerminalPanelProps) {
  const { dock, size, onDockChange, onSizeChange, onClose, narrow, hidden } =
    props
  const workspaceRef = useRef<TerminalWorkspaceHandle>(null)

  return (
    <DockPanel dock={dock} size={size} narrow={narrow} hidden={hidden} label="Terminal" noun="terminal" onSizeChange={onSizeChange}>
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
        hidden={hidden}
        narrow={narrow}
        router={props.router}
        leaseStore={props.leaseStore}
        storageKey={props.storageKey}
        connectionCoordinators={props.connectionCoordinators}
      />
    </DockPanel>
  )
}

/** Memoized: the page re-renders often, and this only when its props change. */
export const TerminalPanel = memo(TerminalPanelView)
