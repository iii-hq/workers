import {
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
} from '@iii-dev/console-ui'
import { ChevronDown, ChevronRight, PanelBottom, PanelRight, SquareTerminal } from 'lucide-react'
import { type Dispatch, memo } from 'react'
import { DockPanel } from './DockPanel'
import type { TerminalDock } from './persist'
import { TerminalWorkspace } from './TerminalWorkspace'
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

/** The panel's rows at the end of the terminal menu: where it docks, and hiding it. */
function PanelMenuItems({ dock, onDockChange, onClose }: Pick<TerminalPanelSharedProps, 'dock' | 'onDockChange' | 'onClose'>) {
  const Hide = dock === 'right' ? ChevronRight : ChevronDown
  return (
    <>
      <DropdownMenuSeparator />
      <DropdownMenuLabel>Position</DropdownMenuLabel>
      <DropdownMenuRadioGroup value={dock} onValueChange={(next) => onDockChange(next as TerminalDock)}>
        <DropdownMenuRadioItem value="bottom">
          <PanelBottom aria-hidden />
          Bottom
        </DropdownMenuRadioItem>
        <DropdownMenuRadioItem value="right">
          <PanelRight aria-hidden />
          Right
        </DropdownMenuRadioItem>
        <DropdownMenuRadioItem value="editor">
          <SquareTerminal aria-hidden />
          Editor tab
        </DropdownMenuRadioItem>
      </DropdownMenuRadioGroup>
      <DropdownMenuSeparator />
      <DropdownMenuItem onSelect={onClose}>
        <Hide aria-hidden />
        Hide terminal
      </DropdownMenuItem>
    </>
  )
}

function TerminalPanelView(props: TerminalPanelProps) {
  const { dock, size, onDockChange, onSizeChange, onClose, narrow, hidden } =
    props

  return (
    <DockPanel dock={dock} size={size} narrow={narrow} hidden={hidden} label="Terminal" noun="terminal" onSizeChange={onSizeChange}>
      <TerminalWorkspace
        menuItems={<PanelMenuItems dock={dock} onDockChange={onDockChange} onClose={onClose} />}
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
