/* A right-click menu over the console's DropdownMenu: the menu anchors to
   an invisible fixed-position trigger placed at the pointer, so Radix
   handles placement, collisions, keyboard traversal and dismissal exactly
   as it does for every other console menu. One hook per surface; items
   are computed at open time from whatever was clicked. */

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { useCallback, useState } from 'react'

export type ContextMenuItem =
  | {
      type?: 'item'
      id: string
      label: string
      icon?: ReactNode
      /** Key hint shown at the right edge, like `F2`. Display only. */
      shortcut?: string
      disabled?: boolean
      /** Destructive rows read in the alert colour. */
      danger?: boolean
      onSelect: () => void
    }
  | { type: 'separator'; id: string }
  | { type: 'label'; id: string; label: string }

export interface ContextMenuAnchor {
  x: number
  y: number
}

export interface ContextMenuState {
  anchor: ContextMenuAnchor
  items: readonly ContextMenuItem[]
}

/** Position from a mouse or keyboard event: the pointer, or the target's
    corner for a keyboard-invoked menu (Shift+F10 / the menu key). */
export function anchorFromEvent(event: {
  clientX?: number
  clientY?: number
  currentTarget?: EventTarget | null
  target?: EventTarget | null
}): ContextMenuAnchor {
  if (typeof event.clientX === 'number' && typeof event.clientY === 'number' && (event.clientX !== 0 || event.clientY !== 0)) {
    return { x: event.clientX, y: event.clientY }
  }
  const el = (event.target ?? event.currentTarget) as Element | null
  const rect = el?.getBoundingClientRect?.()
  return rect ? { x: rect.left + 8, y: rect.bottom } : { x: 0, y: 0 }
}

export function useContextMenu() {
  const [state, setState] = useState<ContextMenuState | null>(null)
  const open = useCallback((anchor: ContextMenuAnchor, items: readonly ContextMenuItem[]) => {
    if (items.length === 0) return
    setState({ anchor, items })
  }, [])
  const close = useCallback(() => setState(null), [])
  const element = state ? <ContextMenuSurface state={state} onClose={close} /> : null
  return { open, close, element, isOpen: state !== null }
}

function ContextMenuSurface({ state, onClose }: { state: ContextMenuState; onClose: () => void }) {
  return (
    <DropdownMenu
      open
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
    >
      <DropdownMenuTrigger asChild>
        <span
          aria-hidden
          className="shui-context-anchor"
          style={{ left: state.anchor.x, top: state.anchor.y }}
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="bottom" sideOffset={2} className="shui-context-menu">
        {state.items.map((item) => {
          if (item.type === 'separator') return <DropdownMenuSeparator key={item.id} />
          if (item.type === 'label') return <DropdownMenuLabel key={item.id}>{item.label}</DropdownMenuLabel>
          return (
            <DropdownMenuItem
              key={item.id}
              className={`shui-context-item${item.danger ? ' danger' : ''}`}
              disabled={item.disabled}
              onSelect={() => {
                onClose()
                item.onSelect()
              }}
            >
              <span className="menu-icon" aria-hidden>
                {item.icon}
              </span>
              <span className="menu-label">{item.label}</span>
              {item.shortcut ? <kbd className="menu-shortcut">{item.shortcut}</kbd> : null}
            </DropdownMenuItem>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
