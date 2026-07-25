import { Columns2, Plus, RectangleHorizontal, X } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { cn } from '@/lib/utils'
import { tabLabel, type WorkspaceTab } from '@/lib/workspace-tabs'

interface TabStripProps {
  tabs: WorkspaceTab[]
  activeTabId: string
  /** ext page id -> title, for tab labels. */
  extPageTitles: ReadonlyMap<string, string>
  onActivate: (id: string) => void
  onClose: (id: string) => void
  /** Create an EMPTY tab with the chosen column count. */
  onCreate: (columns: 1 | 2) => void
  onRename: (id: string, name: string) => void
  /** Move the tab at `from` to position `to` (indexes into `tabs`). */
  onReorder: (from: number, to: number) => void
}

/** Hold a tab still this long to pick it up for reordering. */
const LONG_PRESS_MS = 2000
/**
 * Pointer travel that turns a pending long-press back into a click.
 * Generous because holding a trackpad press still for 2s drifts a little.
 */
const PRESS_SLOP_PX = 12

/** A press that may become a reorder drag once the timer fires. */
interface PendingPress {
  timer: number
  pointerId: number
  pointerType: string
  x: number
  y: number
  index: number
  el: HTMLElement
}

/** Mirror of the `drag` render state, mutable for pointer handlers. */
interface ActiveDrag {
  from: number
  to: number
  dx: number
  startX: number
  pointerId: number
  el: HTMLElement
}

/**
 * The header's workspace tab strip: one pill per saved tab (active carries
 * the selection tint), a close affordance while more than one tab exists,
 * and a `+` dropdown that creates an empty one- or two-column tab (screens
 * are attached from the empty panes). Right-click opens a context menu;
 * its rename action swaps the label for an inline input — Enter commits,
 * Escape (or clicking away) cancels.
 *
 * Pressing a tab for two seconds picks it up for reordering: the tab
 * follows the pointer, neighbours slide out of the way, and releasing
 * commits the new order through `onReorder` (Escape drops it in place).
 * Layout is only reordered on commit, so the rects captured at pickup
 * stay valid for the whole drag.
 */
export function TabStrip({
  tabs,
  activeTabId,
  extPageTitles,
  onActivate,
  onClose,
  onCreate,
  onRename,
  onReorder,
}: TabStripProps) {
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(
    null,
  )
  const [drag, setDrag] = useState<{
    from: number
    to: number
    dx: number
  } | null>(null)
  const stripRef = useRef<HTMLDivElement>(null)
  const pressRef = useRef<PendingPress | null>(null)
  const dragRef = useRef<ActiveDrag | null>(null)
  /** Tab rects (and the flex gap) captured when a drag begins. */
  const geomRef = useRef<{ rects: DOMRect[]; gap: number }>({
    rects: [],
    gap: 0,
  })
  const suppressClickRef = useRef(false)

  const cancelPress = useCallback(() => {
    const press = pressRef.current
    if (!press) return
    clearTimeout(press.timer)
    pressRef.current = null
    try {
      press.el.releasePointerCapture(press.pointerId)
    } catch {
      // Never captured (or already released by a pointercancel).
    }
  }, [])

  const beginDrag = useCallback(() => {
    const press = pressRef.current
    const strip = stripRef.current
    pressRef.current = null
    if (!press || !strip || dragRef.current) return
    const rects = Array.from(
      strip.querySelectorAll<HTMLElement>('[role="tab"]'),
    ).map((el) => el.getBoundingClientRect())
    geomRef.current = {
      rects,
      gap: rects.length > 1 ? rects[1].left - rects[0].right : 0,
    }
    // The pointer was captured at pointerdown, so moves keep flowing to
    // the pressed tab for the whole drag.
    dragRef.current = {
      from: press.index,
      to: press.index,
      dx: 0,
      startX: press.x,
      pointerId: press.pointerId,
      el: press.el,
    }
    setDrag({ from: press.index, to: press.index, dx: 0 })
    navigator.vibrate?.(15)
  }, [])

  const endDrag = useCallback(
    (commit: boolean) => {
      const d = dragRef.current
      if (!d) return
      dragRef.current = null
      try {
        d.el.releasePointerCapture(d.pointerId)
      } catch {
        // Already released (pointercancel / element gone).
      }
      // The pointerup ending a drag still clicks the tab — swallow it so
      // dropping never doubles as activate.
      suppressClickRef.current = true
      setDrag(null)
      if (commit && d.to !== d.from) onReorder(d.from, d.to)
    },
    [onReorder],
  )

  const dragging = drag !== null
  useEffect(() => {
    if (!dragging) return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') endDrag(false)
    }
    // Non-passive so a touch drag pans the tab, not the page.
    const onTouchMove = (e: TouchEvent) => e.preventDefault()
    document.addEventListener('keydown', onKeyDown)
    document.addEventListener('touchmove', onTouchMove, { passive: false })
    return () => {
      document.removeEventListener('keydown', onKeyDown)
      document.removeEventListener('touchmove', onTouchMove)
    }
  }, [dragging, endDrag])

  // A pending long-press timer must not fire into an unmounted strip.
  useEffect(
    () => () => {
      if (pressRef.current) clearTimeout(pressRef.current.timer)
    },
    [],
  )

  return (
    <div
      ref={stripRef}
      role="tablist"
      aria-label="workspace tabs"
      className="flex items-center gap-1 min-w-0 overflow-x-auto"
    >
      {tabs.map((tab, index) => {
        const active = tab.id === activeTabId
        const label = tabLabel(tab, extPageTitles)
        const isDragged = drag !== null && drag.from === index
        // Dragged tab follows the pointer; tabs between its old and
        // prospective slots slide one tab-width toward the vacancy.
        let transform: string | undefined
        if (drag) {
          if (isDragged) {
            transform = `translateX(${drag.dx}px)`
          } else {
            const { rects, gap } = geomRef.current
            const w = (rects[drag.from]?.width ?? 0) + gap
            if (drag.from < index && index <= drag.to) {
              transform = `translateX(${-w}px)`
            } else if (drag.to <= index && index < drag.from) {
              transform = `translateX(${w}px)`
            }
          }
        }
        return (
          <div
            key={tab.id}
            role="tab"
            aria-selected={active}
            tabIndex={0}
            onClick={() => {
              if (suppressClickRef.current) {
                suppressClickRef.current = false
                return
              }
              onActivate(tab.id)
            }}
            onKeyDown={(e) => {
              if (renamingId === tab.id) return
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                onActivate(tab.id)
              }
            }}
            onPointerDown={(e) => {
              if (e.button !== 0 || renamingId === tab.id || dragRef.current)
                return
              // Presses on the close affordance never arm a drag.
              if ((e.target as Element).closest('button')) return
              suppressClickRef.current = false
              cancelPress()
              // Capture up front so slight drift off the tab during the
              // 2s hold neither hides moves from the slop check nor
              // strands the eventual drag without events.
              try {
                e.currentTarget.setPointerCapture(e.pointerId)
              } catch {
                // Best-effort; the press still works in place.
              }
              pressRef.current = {
                pointerId: e.pointerId,
                pointerType: e.pointerType,
                x: e.clientX,
                y: e.clientY,
                index,
                el: e.currentTarget,
                timer: window.setTimeout(beginDrag, LONG_PRESS_MS),
              }
            }}
            onPointerMove={(e) => {
              const press = pressRef.current
              if (
                press &&
                Math.hypot(e.clientX - press.x, e.clientY - press.y) >
                  PRESS_SLOP_PX
              ) {
                cancelPress()
              }
              const d = dragRef.current
              if (!d) return
              const { rects } = geomRef.current
              const own = rects[d.from]
              // Clamp to the tab row so the strip never gains overflow.
              const dx = Math.max(
                rects[0].left - own.left,
                Math.min(
                  rects[rects.length - 1].right - own.right,
                  e.clientX - d.startX,
                ),
              )
              // Drop slot from the POINTER, not the (clamped) tab: the
              // insertion index is how many other tabs' centers sit left
              // of it.
              let to = 0
              for (let i = 0; i < rects.length; i++) {
                if (i === d.from) continue
                if (e.clientX > rects[i].left + rects[i].width / 2) to++
              }
              d.dx = dx
              d.to = to
              setDrag({ from: d.from, to, dx })
            }}
            onPointerUp={() => {
              cancelPress()
              endDrag(true)
            }}
            onPointerCancel={() => {
              cancelPress()
              endDrag(false)
            }}
            onContextMenu={(e) => {
              e.preventDefault()
              if (dragRef.current) return
              // Touch long-press fires the browser context menu well
              // before the 2s pickup — swallow it so a continued hold
              // reaches reorder mode.
              if (pressRef.current?.pointerType === 'touch') return
              cancelPress()
              setMenu({ id: tab.id, x: e.clientX, y: e.clientY })
            }}
            style={transform ? { transform } : undefined}
            className={cn(
              'group/tab flex items-center gap-1 h-7 pl-2.5 rounded-sm font-mono text-[12px] lowercase cursor-pointer select-none whitespace-nowrap transition-colors',
              tabs.length > 1 ? 'pr-1' : 'pr-2.5',
              active
                ? 'bg-accent-muted text-ink'
                : 'text-ink-faint hover:bg-surface-hover hover:text-ink',
              dragging && !isDragged && 'transition-transform duration-150',
              isDragged &&
                'relative z-10 shadow-floating cursor-grabbing transition-none',
              isDragged && !active && 'bg-panel-raised text-ink',
            )}
          >
            {renamingId === tab.id ? (
              <RenameInput
                initial={tab.name ?? label}
                onCommit={(name) => {
                  setRenamingId(null)
                  onRename(tab.id, name)
                }}
                onCancel={() => setRenamingId(null)}
              />
            ) : (
              <span className="truncate max-w-[180px]">{label}</span>
            )}
            {tabs.length > 1 && renamingId !== tab.id ? (
              <button
                type="button"
                aria-label={`close ${label}`}
                onClick={(e) => {
                  e.stopPropagation()
                  onClose(tab.id)
                }}
                className={cn(
                  'flex items-center justify-center size-4.5 rounded-xs text-ink-ghost hover:text-ink hover:bg-surface-hover transition-[color,opacity]',
                  !active && 'opacity-0 group-hover/tab:opacity-100',
                )}
              >
                <X className="size-3" />
              </button>
            ) : null}
          </div>
        )
      })}

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label="new tab"
            title="new tab"
            className="flex items-center justify-center size-7 shrink-0 rounded-sm text-ink-faint hover:text-ink hover:bg-surface-hover data-[state=open]:bg-surface data-[state=open]:text-ink transition-colors"
          >
            <Plus className="size-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" sideOffset={6}>
          <DropdownMenuItem onSelect={() => onCreate(1)}>
            <RectangleHorizontal className="size-3.5 text-ink-faint" />1 column
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => onCreate(2)}>
            <Columns2 className="size-3.5 text-ink-faint" />2 columns
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {menu ? (
        <TabContextMenu
          x={menu.x}
          y={menu.y}
          canClose={tabs.length > 1}
          onRename={() => {
            setRenamingId(menu.id)
            setMenu(null)
          }}
          onClose={() => {
            onClose(menu.id)
            setMenu(null)
          }}
          onDismiss={() => setMenu(null)}
        />
      ) : null}
    </div>
  )
}

interface RenameInputProps {
  initial: string
  onCommit: (name: string) => void
  onCancel: () => void
}

function RenameInput({ initial, onCommit, onCancel }: RenameInputProps) {
  const [draft, setDraft] = useState(initial)
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])
  return (
    <input
      ref={inputRef}
      value={draft}
      onChange={(e) => setDraft(e.currentTarget.value)}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation()
        if (e.key === 'Enter') onCommit(draft)
        else if (e.key === 'Escape') onCancel()
      }}
      onBlur={onCancel}
      aria-label="rename tab"
      className="w-[140px] bg-transparent font-mono text-[12px] text-ink outline-none lowercase"
    />
  )
}

interface TabContextMenuProps {
  x: number
  y: number
  canClose: boolean
  onRename: () => void
  onClose: () => void
  onDismiss: () => void
}

/**
 * Hand-rolled context menu (Radix dropdowns anchor to a trigger element,
 * not a pointer position). Portaled to body; dismissed by any outside
 * pointer-down or Escape.
 */
function TabContextMenu({
  x,
  y,
  canClose,
  onRename,
  onClose,
  onDismiss,
}: TabContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const onPointerDown = (e: PointerEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return
      onDismiss()
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onDismiss()
    }
    document.addEventListener('pointerdown', onPointerDown)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [onDismiss])

  const itemCls =
    'w-full flex items-center gap-2 rounded-xs px-2 py-1.5 text-left font-mono text-[12px] lowercase text-ink cursor-pointer hover:bg-surface-hover transition-colors'

  return createPortal(
    <div
      ref={menuRef}
      role="menu"
      style={{
        position: 'fixed',
        top: Math.min(y, window.innerHeight - 96),
        left: Math.min(x, window.innerWidth - 160),
        zIndex: 60,
      }}
      className="min-w-[140px] rounded-md bg-panel-raised p-1 shadow-floating"
    >
      <button
        type="button"
        role="menuitem"
        className={itemCls}
        onClick={onRename}
      >
        rename
      </button>
      {canClose ? (
        <button
          type="button"
          role="menuitem"
          className={itemCls}
          onClick={onClose}
        >
          close tab
        </button>
      ) : null}
    </div>,
    document.body,
  )
}
