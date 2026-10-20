import {
  AppWindow,
  Check,
  ChevronDown,
  type LucideIcon,
  Plus,
  X,
} from 'lucide-react'
import {
  Fragment,
  type KeyboardEvent as ReactKeyboardEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { createPortal } from 'react-dom'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { IconButton } from '@/components/ui/IconButton'
import { hoverTitle } from '@/lib/keybindings/registry'
import { cn } from '@/lib/utils'
import {
  CHAT_SCREEN,
  isChatScreen,
  tabLabel,
  type WorkspaceTab,
} from '@/lib/workspace-tabs'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

interface TabStripProps {
  tabs: WorkspaceTab[]
  activeTabId: string
  /** ext page id -> title, for tab labels. */
  extPageTitles: ReadonlyMap<string, string>
  /**
   * screen -> glyph, for the favicon-style mark a tab wears: its first
   * attached screen's icon, the chat mark for any chat session, a window
   * glyph while nothing is attached yet.
   */
  screenIcons?: ReadonlyMap<string, LucideIcon>
  onActivate: (id: string) => void
  onClose: (id: string) => void
  /** Create an EMPTY single-column tab (panes grow via the edge zones). */
  onCreate: () => void
  /** The configured tab limit is reached: the `+` disables and says why. */
  createDisabled?: boolean
  /** Hover title for the disabled `+` (defaults to the limit-reached copy). */
  createDisabledTitle?: string
  onRename: (id: string, name: string) => void
  /** Move the tab at `from` to position `to` (indexes into `tabs`). */
  onReorder: (from: number, to: number) => void
}

/** Touch: hold a tab still this long to pick it up for reordering (a
    mouse/pen drag starts on movement alone — see the slop check). */
const TOUCH_PICKUP_MS = 500
/**
 * Pointer travel that decides the press: with a mouse/pen it BEGINS the
 * reorder drag; on touch it cancels the pickup so the strip can scroll.
 */
const PRESS_SLOP_PX = 6

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

interface Overflow {
  overflowing: boolean
  start: boolean
  end: boolean
}

const NO_OVERFLOW: Overflow = { overflowing: false, start: false, end: false }

/** The glyph a tab wears, from its first attached screen. */
function tabIcon(
  tab: WorkspaceTab,
  screenIcons: ReadonlyMap<string, LucideIcon> | undefined,
): LucideIcon {
  const first = tab.screens.find((screen) => screen !== null)
  if (!first) return AppWindow
  const key = isChatScreen(first) ? CHAT_SCREEN : first
  return screenIcons?.get(key) ?? AppWindow
}

/**
 * The active tab's two flares — the browser-tab silhouette: a square of the
 * canvas colour just outside each bottom corner, with a quarter circle cut
 * out of its far top corner, so the tab appears to curve down into the
 * canvas below the bar.
 */
/**
 * An inactive tab's hover: not the tab silhouette (that is the active tab's
 * shape) but a pill inset a few pixels on every side, drawn behind the
 * label, so the wash floats inside the tab rather than butting against the
 * bar's bottom edge.
 */
const INACTIVE_WASH =
  'after:pointer-events-none after:absolute after:inset-x-0.5 after:inset-y-1 after:-z-10 after:rounded-lg after:transition-colors after:duration-[var(--motion-duration-control)]'

const TAB_FLARES =
  'before:pointer-events-none before:absolute before:bottom-0 before:-left-3 before:size-[12px] before:bg-bg before:[mask-image:radial-gradient(circle_at_0_0,transparent_12px,black_12.5px)] after:pointer-events-none after:absolute after:bottom-0 after:-right-3 after:size-[12px] after:bg-bg after:[mask-image:radial-gradient(circle_at_100%_0,transparent_12px,black_12.5px)]'

/** Where the next focused/activated tab lands for a tablist key. */
export function tabIndexForKey(
  key: string,
  current: number,
  count: number,
): number | null {
  if (count === 0) return null
  switch (key) {
    case 'ArrowRight':
      return (current + 1) % count
    case 'ArrowLeft':
      return (current - 1 + count) % count
    case 'Home':
      return 0
    case 'End':
      return count - 1
    default:
      return null
  }
}

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
}

/**
 * The header's workspace tab strip: one pill per saved tab (active carries
 * the selection tint), a close affordance while more than one tab exists,
 * and a `+` button that creates an empty single-column tab (screens attach
 * from the empty pane; splits grow via the workspace edge zones).
 *
 * Overflow scrolls with edge fades and an "All workspaces" menu; arrow keys,
 * Home, End and Delete work inside the strip; middle click closes.
 * Double-click (or the right-click menu) renames — Enter or clicking away
 * commits, Escape cancels.
 *
 * Dragging a tab reorders it: a mouse/pen picks it up as soon as the
 * press moves, touch picks it up after a short hold (so the strip can
 * still scroll). The tab follows the pointer, neighbours slide out of the
 * way, and releasing commits the new order through `onReorder` (Escape
 * drops it in place). Layout is only reordered on commit, so the rects
 * captured at pickup stay valid for the whole drag.
 */
export function TabStrip({
  tabs,
  activeTabId,
  extPageTitles,
  screenIcons,
  onActivate,
  onClose,
  onCreate,
  createDisabled = false,
  createDisabledTitle,
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
  const [overflow, setOverflow] = useState<Overflow>(NO_OVERFLOW)
  const stripRef = useRef<HTMLDivElement>(null)
  const pressRef = useRef<PendingPress | null>(null)
  const dragRef = useRef<ActiveDrag | null>(null)
  /** Tab rects (and the flex gap) captured when a drag begins. */
  const geomRef = useRef<{ rects: DOMRect[]; gap: number }>({
    rects: [],
    gap: 0,
  })
  const suppressClickRef = useRef(false)
  const focusActiveRef = useRef(false)
  const pendingCloseRef = useRef<string | null>(null)

  const measureOverflow = useCallback(() => {
    const strip = stripRef.current
    if (!strip) return
    const max = strip.scrollWidth - strip.clientWidth
    const overflowing = max > 1
    const next: Overflow = {
      overflowing,
      start: overflowing && strip.scrollLeft > 1,
      end: overflowing && strip.scrollLeft < max - 1,
    }
    setOverflow((current) =>
      current.overflowing === next.overflowing &&
      current.start === next.start &&
      current.end === next.end
        ? current
        : next,
    )
  }, [])

  useLayoutEffect(() => {
    const strip = stripRef.current
    if (!strip) return
    measureOverflow()
    const observer =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(measureOverflow)
    observer?.observe(strip)
    strip.addEventListener('scroll', measureOverflow, { passive: true })
    return () => {
      observer?.disconnect()
      strip.removeEventListener('scroll', measureOverflow)
    }
  }, [measureOverflow])

  // The active tab stays in view whatever selected it; labels remeasure
  // overflow when a rename or a page title changes a tab's width.
  const labels = tabs.map((tab) => tabLabel(tab, extPageTitles)).join('\n')
  // biome-ignore lint/correctness/useExhaustiveDependencies: the labels are the trigger, not a value read here.
  useLayoutEffect(() => {
    const pending = pendingCloseRef.current
    if (pending !== null) {
      pendingCloseRef.current = null
      if (!tabs.some((tab) => tab.id === pending)) focusActiveRef.current = true
    }
    const strip = stripRef.current
    if (!strip || tabs.length === 0) return
    const el = strip.querySelector<HTMLElement>(
      `[data-tab-id="${CSS.escape(activeTabId)}"]`,
    )
    if (!el) return
    const left = el.offsetLeft
    const right = left + el.offsetWidth
    const viewStart = strip.scrollLeft
    const viewEnd = viewStart + strip.clientWidth
    const target =
      left < viewStart
        ? left
        : right > viewEnd
          ? right - strip.clientWidth
          : null
    if (target !== null) {
      strip.scrollTo({
        left: target,
        behavior: prefersReducedMotion() ? 'auto' : 'smooth',
      })
    }
    if (focusActiveRef.current) {
      focusActiveRef.current = false
      el.focus()
    }
    measureOverflow()
  }, [activeTabId, tabs, labels, measureOverflow])

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

  const closeTab = useCallback(
    (id: string) => {
      if (tabs.length <= 1) return
      onClose(id)
    },
    [onClose, tabs.length],
  )

  const onTablistKeyDown = useCallback(
    (e: ReactKeyboardEvent<HTMLDivElement>) => {
      if (renamingId !== null || dragRef.current) return
      const target = (e.target as HTMLElement).closest<HTMLElement>(
        '[data-tab-id]',
      )
      const focusedId = target?.dataset.tabId
      if (!focusedId) return
      const current = tabs.findIndex((tab) => tab.id === focusedId)
      if (current < 0) return
      if (e.key === 'Delete' || e.key === 'Backspace') {
        if (tabs.length <= 1) return
        e.preventDefault()
        pendingCloseRef.current = focusedId
        closeTab(focusedId)
        return
      }
      if (e.key === 'F2') {
        e.preventDefault()
        setRenamingId(focusedId)
        return
      }
      const next = tabIndexForKey(e.key, current, tabs.length)
      if (next === null) return
      e.preventDefault()
      const id = tabs[next].id
      if (id === activeTabId) {
        stripRef.current
          ?.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(id)}"]`)
          ?.focus()
        return
      }
      focusActiveRef.current = true
      onActivate(id)
    },
    [activeTabId, closeTab, onActivate, renamingId, tabs],
  )

  const maskImage = overflow.overflowing
    ? `linear-gradient(to right, ${
        overflow.start ? 'transparent, black 24px' : 'black, black'
      }, ${overflow.end ? 'black calc(100% - 24px), transparent' : 'black, black'})`
    : undefined

  return (
    <div className="flex min-w-0 items-end gap-1">
      <div
        ref={stripRef}
        role="tablist"
        aria-label="Workspace tabs"
        aria-orientation="horizontal"
        onKeyDown={onTablistKeyDown}
        data-overflow-start={overflow.start || undefined}
        data-overflow-end={overflow.end || undefined}
        style={
          maskImage ? { maskImage, WebkitMaskImage: maskImage } : undefined
        }
        /* onboarding-tabs: tour anchor (workers/onboarding). Do not remove. */
        className="onboarding-tabs workspace-tab-strip flex min-w-0 items-end overflow-x-auto px-3"
      >
        {tabs.map((tab, index) => {
          const active = tab.id === activeTabId
          const label = tabLabel(tab, extPageTitles)
          const Icon = tabIcon(tab, screenIcons)
          const isDragged = drag !== null && drag.from === index
          // A hairline between two inactive tabs; none beside the active
          // tab (its flares own that edge) or the one being dragged.
          const divider =
            index > 0 &&
            !active &&
            tabs[index - 1].id !== activeTabId &&
            drag === null
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
            <Fragment key={tab.id}>
              <span
                aria-hidden
                className={cn(
                  'h-4 w-px shrink-0 self-center bg-neutral-400/60 transition-opacity duration-[var(--motion-duration-control)] dark:bg-neutral-700',
                  !divider && 'opacity-0',
                )}
              />
              <div
                role="tab"
                data-tab-id={tab.id}
                aria-selected={active}
                tabIndex={active ? 0 : -1}
                title={index < 9 ? `${label} (${index + 1})` : label}
                onClick={() => {
                  if (suppressClickRef.current) {
                    suppressClickRef.current = false
                    return
                  }
                  onActivate(tab.id)
                }}
                onAuxClick={(e) => {
                  if (e.button !== 1 || renamingId === tab.id) return
                  e.preventDefault()
                  closeTab(tab.id)
                }}
                onKeyDown={(e) => {
                  if (renamingId === tab.id) return
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault()
                    onActivate(tab.id)
                  }
                }}
                onDoubleClick={() => {
                  if (renamingId === tab.id || dragRef.current) return
                  cancelPress()
                  setRenamingId(tab.id)
                }}
                onPointerDown={(e) => {
                  if (e.button === 1) e.preventDefault()
                  if (
                    e.button !== 0 ||
                    renamingId === tab.id ||
                    dragRef.current
                  )
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
                    // Touch keeps a short-hold pickup (movement scrolls the
                    // strip). Mouse/pen picks up on movement — the timer is a
                    // stationary-hold fallback.
                    timer: window.setTimeout(
                      beginDrag,
                      e.pointerType === 'touch' ? TOUCH_PICKUP_MS : 2000,
                    ),
                  }
                }}
                onPointerMove={(e) => {
                  const press = pressRef.current
                  if (
                    press &&
                    Math.hypot(e.clientX - press.x, e.clientY - press.y) >
                      PRESS_SLOP_PX
                  ) {
                    if (press.pointerType === 'touch') {
                      // Let the finger scroll the strip.
                      cancelPress()
                    } else {
                      // Mouse/pen: movement IS the drag gesture.
                      clearTimeout(press.timer)
                      beginDrag()
                    }
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
                  'group/tab relative isolate flex h-12 shrink-0 cursor-pointer select-none items-center gap-2 whitespace-nowrap rounded-t-xl pl-4 font-inter text-sm font-medium transition-colors duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 focus-visible:ring-inset sm:h-9 sm:gap-2 sm:pl-3 dark:focus-visible:ring-neutral-400',
                  tabs.length > 1 ? 'pr-2 sm:pr-1.5' : 'pr-4 sm:pr-3',
                  active
                    ? cn(
                        'z-10 bg-bg text-neutral-900 dark:text-neutral-50',
                        TAB_FLARES,
                      )
                    : cn(
                        'text-neutral-500 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100',
                        INACTIVE_WASH,
                        'hover:after:bg-neutral-300/60 dark:hover:after:bg-neutral-800',
                      ),
                  dragging && !isDragged && 'transition-transform',
                  isDragged && 'z-20 cursor-grabbing transition-none',
                  isDragged &&
                    !active &&
                    'text-neutral-900 after:bg-neutral-300/60 dark:text-neutral-50 dark:after:bg-neutral-800',
                )}
              >
                <Icon
                  className="size-4 shrink-0 text-neutral-500 dark:text-neutral-400"
                  aria-hidden
                />
                {renamingId === tab.id ? (
                  <RenameInput
                    initial={tab.name ?? label}
                    onCommit={(name) => {
                      setRenamingId(null)
                      if (name.trim() !== (tab.name ?? label).trim()) {
                        onRename(tab.id, name)
                      }
                    }}
                    onCancel={() => setRenamingId(null)}
                  />
                ) : (
                  <span className="max-w-[180px] truncate">{label}</span>
                )}
                {tabs.length > 1 && renamingId !== tab.id ? (
                  <button
                    type="button"
                    tabIndex={-1}
                    aria-label={`Close ${label}`}
                    title={
                      tab.id === activeTabId
                        ? hoverTitle(`Close ${label}`, 'workspace.close')
                        : `Close ${label}`
                    }
                    onClick={(e) => {
                      e.stopPropagation()
                      closeTab(tab.id)
                    }}
                    className={cn(
                      'relative flex size-10 items-center justify-center rounded-lg text-neutral-400 transition-colors duration-[var(--motion-duration-control)] hover:bg-neutral-300/60 hover:text-neutral-900 sm:size-6 dark:text-neutral-500 dark:hover:bg-neutral-700 dark:hover:text-neutral-50',
                      !active &&
                        'opacity-100 pointer-fine:opacity-0 pointer-fine:group-hover/tab:opacity-100 pointer-fine:group-focus-within/tab:opacity-100',
                    )}
                  >
                    <span
                      className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                      aria-hidden="true"
                    />
                    <X className="size-4 shrink-0" aria-hidden />
                  </button>
                ) : null}
              </div>
            </Fragment>
          )
        })}
      </div>

      <button
        type="button"
        aria-label="New workspace"
        aria-disabled={createDisabled || undefined}
        disabled={createDisabled}
        title={
          createDisabled
            ? (createDisabledTitle ?? 'Tab limit reached')
            : hoverTitle('New workspace', 'workspace.create')
        }
        onClick={onCreate}
        className="relative flex size-12 shrink-0 items-center justify-center self-center rounded-lg text-neutral-500 transition-colors duration-[var(--motion-duration-control)] hover:bg-neutral-300/60 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 disabled:pointer-events-none disabled:opacity-40 sm:size-8 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100 dark:focus-visible:ring-neutral-400"
      >
        <span
          className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
          aria-hidden="true"
        />
        <Plus className="size-4 shrink-0" aria-hidden />
      </button>

      {overflow.overflowing ? (
        <OverflowMenu
          tabs={tabs}
          activeTabId={activeTabId}
          extPageTitles={extPageTitles}
          onActivate={onActivate}
        />
      ) : null}

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
            closeTab(menu.id)
            setMenu(null)
          }}
          onDismiss={() => setMenu(null)}
        />
      ) : null}
    </div>
  )
}

interface OverflowMenuProps {
  tabs: WorkspaceTab[]
  activeTabId: string
  extPageTitles: ReadonlyMap<string, string>
  onActivate: (id: string) => void
}

/** Every workspace by name when the strip cannot show them all. */
function OverflowMenu({
  tabs,
  activeTabId,
  extPageTitles,
  onActivate,
}: OverflowMenuProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <IconButton
          label="All workspaces"
          tooltip={`All workspaces (${tabs.length})`}
          className="shrink-0"
        >
          <ChevronDown className="size-4" aria-hidden />
        </IconButton>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="max-h-[60vh] overflow-y-auto"
      >
        <DropdownMenuLabel>Workspaces</DropdownMenuLabel>
        {tabs.map((tab, index) => (
          <DropdownMenuCheckboxItem
            key={tab.id}
            checked={tab.id === activeTabId}
            onSelect={() => onActivate(tab.id)}
            indicator={
              <Check aria-hidden className="size-4" strokeWidth={2.5} />
            }
          >
            <span className="min-w-0 flex-1 truncate">
              {tabLabel(tab, extPageTitles)}
            </span>
            {index < 9 ? (
              <span className="ml-4 font-mono text-[11px] text-ink-ghost">
                {index + 1}
              </span>
            ) : null}
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
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
  const settledRef = useRef(false)
  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])
  const settle = (commit: boolean) => {
    if (settledRef.current) return
    settledRef.current = true
    if (commit) onCommit(draft)
    else onCancel()
  }
  return (
    <input
      ref={inputRef}
      value={draft}
      onChange={(e) => setDraft(e.currentTarget.value)}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation()
        if (e.key === 'Enter') settle(true)
        else if (e.key === 'Escape') settle(false)
      }}
      onBlur={() => settle(true)}
      aria-label="Rename workspace"
      name="tab-name"
      className="w-[160px] bg-transparent font-inter text-sm text-neutral-900 outline-none dark:text-neutral-50"
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
    'flex w-full cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-left font-inter text-[13px] text-neutral-700 transition-colors hover:bg-neutral-100 hover:text-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800 dark:hover:text-neutral-50'

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
      className="min-w-[150px] rounded-xl bg-white p-1 font-inter shadow-floating ring-1 ring-neutral-200 dark:bg-neutral-900 dark:ring-neutral-800"
    >
      <button
        type="button"
        role="menuitem"
        className={itemCls}
        onClick={onRename}
      >
        Rename
      </button>
      {canClose ? (
        <button
          type="button"
          role="menuitem"
          className={itemCls}
          onClick={onClose}
        >
          Close workspace
        </button>
      ) : null}
    </div>,
    document.body,
  )
}
