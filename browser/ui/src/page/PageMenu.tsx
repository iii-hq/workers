/**
 * The ⋮ menu beside the address bar: what a browser keeps behind its own
 * menu, scoped to this session. Every row is also a page command, so ⌘K
 * lists the same verbs with their keys; the menu is the mouse path.
 */

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@iii-dev/console-ui'
import { cn } from '../lib/cn'
import { Minus, MoreVertical, Plus, RefreshCw } from '../lib/icons'

export interface PageMenuActions {
  findInPage: () => void
  takeScreenshot: () => void
  screenshotToChat: () => void
  printToPdf: () => void
  zoomIn: () => void
  zoomOut: () => void
  zoomReset: () => void
  clearData: () => void
  toggleDeviceToolbar: () => void
  importCookies: () => void
  copyCookies: () => void
}

interface PageMenuProps {
  actions: PageMenuActions
  zoom: number
  canSendToChat: boolean
}

export function PageMenu({ actions, zoom, canSendToChat }: PageMenuProps) {
  const row = (
    label: string,
    action: keyof PageMenuActions,
    options: { disabled?: boolean } = {},
  ) => (
    <DropdownMenuItem
      onSelect={() => actions[action]()}
      disabled={options.disabled}
      className="br-ui-menu-item"
    >
      {label}
    </DropdownMenuItem>
  )
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="br-ui-chrome-btn"
          title="page menu"
          aria-label="page menu"
        >
          <MoreVertical size={17} aria-hidden />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" sideOffset={6} className="br-ui-menu">
        {row('Find in page', 'findInPage')}
        {row('Print to PDF', 'printToPdf')}
        <DropdownMenuSeparator />
        <fieldset className="br-ui-menu-zoom">
          <legend className="br-ui-menu-zoom-legend">Zoom</legend>
          <span className="br-ui-menu-zoom-controls">
            <button
              type="button"
              className="br-ui-chrome-btn"
              onClick={actions.zoomOut}
              disabled={zoom <= 50}
              aria-label="zoom out"
              title="zoom out"
            >
              <Minus size={16} aria-hidden />
            </button>
            <span
              className={cn('br-ui-menu-zoom-level', zoom !== 100 && 'is-set')}
              aria-live="polite"
            >
              {zoom}%
            </span>
            <button
              type="button"
              className="br-ui-chrome-btn"
              onClick={actions.zoomIn}
              disabled={zoom >= 200}
              aria-label="zoom in"
              title="zoom in"
            >
              <Plus size={16} aria-hidden />
            </button>
            <button
              type="button"
              className="br-ui-chrome-btn"
              onClick={actions.zoomReset}
              disabled={zoom === 100}
              aria-label="reset zoom"
              title="reset zoom"
            >
              <RefreshCw size={16} aria-hidden />
            </button>
          </span>
        </fieldset>
        <DropdownMenuSeparator />
        {row('Show device toolbar', 'toggleDeviceToolbar')}
        {row('Take a screenshot', 'takeScreenshot')}
        {row('Screenshot to chat', 'screenshotToChat', {
          disabled: !canSendToChat,
        })}
        <DropdownMenuSeparator />
        {row('Import cookies…', 'importCookies')}
        {row('Copy cookies', 'copyCookies')}
        {row('Clear browsing data', 'clearData')}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
