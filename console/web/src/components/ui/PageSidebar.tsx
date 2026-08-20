import {
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
} from 'lucide-react'
import * as React from 'react'
import { cn } from '@/lib/utils'
import { IconButton } from './IconButton'

const COLLAPSED_WIDTH = 36
const DEFAULT_WIDTH = 280
const DEFAULT_MIN_WIDTH = 160
const DEFAULT_MAX_WIDTH = 560
const KEYBOARD_STEP = 16
const MOTION_CLEANUP_MS = 260
const STORAGE_PREFIX = 'iii:page-sidebar:v1:'
const STORAGE_SYNC_EVENT = 'iii:page-sidebar-state'

interface StoredSidebarState {
  collapsed?: boolean
  width?: number
}

interface StoredSidebarStateEvent {
  sourceId: string
  storageKey: string
  state: StoredSidebarState
}

export interface PageSidebarProps
  extends Omit<React.HTMLAttributes<HTMLElement>, 'children'> {
  /** Accessible name used by the aside, toggle, tooltip and resize handle. */
  label?: string
  /** Which outer edge the sidebar hugs. Controls resize direction and icons. */
  side?: 'left' | 'right'
  /** Sidebar content. It stays mounted while the rail is collapsed. */
  children?: React.ReactNode
  /** Optional standard top row rendered beside the stable collapse toggle. */
  header?: React.ReactNode
  /** Compact actions rendered below the toggle in the collapsed rail. */
  collapsedActions?: React.ReactNode

  /** Controlled expanded width. Existing fixed-width usages remain supported. */
  width?: number
  /** Initial expanded width for the host-owned uncontrolled mode. */
  defaultWidth?: number
  minWidth?: number
  maxWidth?: number
  onWidthChange?: (width: number) => void

  collapsible?: boolean
  collapsed?: boolean
  defaultCollapsed?: boolean
  onCollapsedChange?: (collapsed: boolean) => void
  resizable?: boolean

  /**
   * Persists uncontrolled width/collapse state in the Console host. Worker
   * bundles provide only this key; storage and gesture code remain host-owned.
   * Mounted instances with the same key share the same preference.
   */
  storageKey?: string
  /**
   * Full-width responsive presentation. It hides collapse/resize affordances
   * without overwriting the saved wide-layout preference.
   */
  narrow?: boolean
  /**
   * Host-owned container breakpoint for the responsive presentation. The
   * sidebar observes its PageBody, so workers do not need to ship a resize
   * hook solely to coordinate the shared chrome.
   */
  narrowBelow?: number
}

function clampWidth(width: number, minWidth: number, maxWidth: number): number {
  return Math.max(minWidth, Math.min(maxWidth, Math.round(width)))
}

function parseStoredState(raw: string | null): StoredSidebarState {
  try {
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      return {}
    const record = parsed as Record<string, unknown>
    return {
      collapsed:
        typeof record.collapsed === 'boolean' ? record.collapsed : undefined,
      width: typeof record.width === 'number' ? record.width : undefined,
    }
  } catch {
    return {}
  }
}

function readStoredState(storageKey: string | undefined): StoredSidebarState {
  if (!storageKey || typeof window === 'undefined') return {}
  try {
    return parseStoredState(
      window.localStorage.getItem(`${STORAGE_PREFIX}${storageKey}`),
    )
  } catch {
    return {}
  }
}

function writeStoredState(
  storageKey: string | undefined,
  state: StoredSidebarState,
  sourceId: string,
): void {
  if (!storageKey || typeof window === 'undefined') return
  try {
    window.localStorage.setItem(
      `${STORAGE_PREFIX}${storageKey}`,
      JSON.stringify(state),
    )
  } catch {
    // Geometry preferences are best-effort in private/quota-limited contexts.
  }
  window.dispatchEvent(
    new CustomEvent<StoredSidebarStateEvent>(STORAGE_SYNC_EVENT, {
      detail: { sourceId, storageKey, state },
    }),
  )
}

function firstFocusable(root: HTMLElement): HTMLElement | null {
  return root.querySelector<HTMLElement>(
    'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
  )
}

/**
 * The shared navigation column for native and worker-injected Console pages.
 *
 * With no behavior props it remains the original fixed-width visual primitive.
 * `collapsible`/`resizable` opt into the host-owned shell: one stable aside,
 * consistent motion/focus/ARIA, pointer + keyboard resize, and optional host
 * persistence. Worker bundles keep only their domain-specific content.
 */
export function PageSidebar({
  label = 'sidebar',
  side = 'left',
  children,
  header,
  collapsedActions,
  width: controlledWidth,
  defaultWidth = controlledWidth ?? DEFAULT_WIDTH,
  minWidth = DEFAULT_MIN_WIDTH,
  maxWidth = DEFAULT_MAX_WIDTH,
  onWidthChange,
  collapsible = false,
  collapsed: controlledCollapsed,
  defaultCollapsed = false,
  onCollapsedChange,
  resizable = false,
  storageKey,
  narrow = false,
  narrowBelow,
  className,
  style,
  ...rest
}: PageSidebarProps) {
  const managed =
    collapsible ||
    resizable ||
    header !== undefined ||
    collapsedActions !== undefined ||
    storageKey !== undefined ||
    controlledCollapsed !== undefined

  const bounds = React.useMemo(() => {
    const low = Math.min(minWidth, maxWidth)
    const high = Math.max(minWidth, maxWidth)
    return { min: low, max: high }
  }, [minWidth, maxWidth])

  const initialStoredRef = React.useRef<StoredSidebarState | null>(null)
  if (initialStoredRef.current === null) {
    initialStoredRef.current = readStoredState(storageKey)
  }

  const [internalWidth, setInternalWidth] = React.useState(() =>
    clampWidth(
      initialStoredRef.current?.width ?? defaultWidth,
      bounds.min,
      bounds.max,
    ),
  )
  const [internalCollapsed, setInternalCollapsed] = React.useState(
    () => initialStoredRef.current?.collapsed ?? defaultCollapsed,
  )
  const [isResizing, setIsResizing] = React.useState(false)
  // Transitions are enabled only for the component's explicit toggle. Initial
  // hydration, controlled layout updates and wide↔narrow changes stay instant.
  const [animateToggle, setAnimateToggle] = React.useState(false)

  const asideRef = React.useRef<HTMLElement>(null)
  const [containerNarrow, setContainerNarrow] = React.useState(false)

  React.useLayoutEffect(() => {
    if (narrow || narrowBelow === undefined) {
      setContainerNarrow(false)
      return
    }

    const container = asideRef.current?.parentElement
    if (!container) return

    const update = (containerWidth: number) => {
      if (containerWidth <= 0) return
      const next = containerWidth <= narrowBelow
      setContainerNarrow((current) => (current === next ? current : next))
    }

    update(container.getBoundingClientRect().width)
    if (typeof ResizeObserver === 'undefined') return

    const observer = new ResizeObserver(([entry]) => {
      if (entry) update(entry.contentRect.width)
    })
    observer.observe(container)
    return () => observer.disconnect()
  }, [narrow, narrowBelow])

  const effectiveNarrow = narrow || containerNarrow
  const expandedWidth = clampWidth(
    controlledWidth ?? internalWidth,
    bounds.min,
    bounds.max,
  )
  const preferredCollapsed = controlledCollapsed ?? internalCollapsed
  const effectiveCollapsed =
    collapsible && !effectiveNarrow && preferredCollapsed
  const effectiveWidth: number | string = effectiveNarrow
    ? '100%'
    : effectiveCollapsed
      ? COLLAPSED_WIDTH
      : expandedWidth

  const expandedRef = React.useRef<HTMLDivElement>(null)
  const collapsedActionsRef = React.useRef<HTMLDivElement>(null)
  const toggleRef = React.useRef<HTMLButtonElement>(null)
  const resizeRef = React.useRef<HTMLDivElement>(null)
  const chromeFocusRef = React.useRef(false)
  const dragRef = React.useRef<{
    pointerId: number
    startWidth: number
    startX: number
  } | null>(null)
  const widthRef = React.useRef(expandedWidth)
  const collapsedRef = React.useRef(preferredCollapsed)
  const motionTimerRef = React.useRef<number | null>(null)
  const contentId = React.useId()
  const storageSourceId = React.useId()
  const previousStorageKeyRef = React.useRef(storageKey)

  widthRef.current = expandedWidth
  collapsedRef.current = preferredCollapsed

  React.useLayoutEffect(() => {
    if (previousStorageKeyRef.current === storageKey) return
    previousStorageKeyRef.current = storageKey
    const next = readStoredState(storageKey)
    setAnimateToggle(false)
    if (controlledWidth === undefined) {
      const clamped = clampWidth(
        next.width ?? defaultWidth,
        bounds.min,
        bounds.max,
      )
      widthRef.current = clamped
      setInternalWidth(clamped)
    }
    if (controlledCollapsed === undefined) {
      const collapsed = next.collapsed ?? defaultCollapsed
      collapsedRef.current = collapsed
      setInternalCollapsed(collapsed)
    }
  }, [
    bounds.max,
    bounds.min,
    controlledCollapsed,
    controlledWidth,
    defaultCollapsed,
    defaultWidth,
    storageKey,
  ])

  React.useEffect(() => {
    if (!storageKey) return

    const apply = (next: StoredSidebarState) => {
      setAnimateToggle(false)
      if (controlledWidth === undefined && next.width !== undefined) {
        const clamped = clampWidth(next.width, bounds.min, bounds.max)
        widthRef.current = clamped
        setInternalWidth(clamped)
      }
      if (controlledCollapsed === undefined && next.collapsed !== undefined) {
        collapsedRef.current = next.collapsed
        setInternalCollapsed(next.collapsed)
      }
    }

    const onSync = (event: Event) => {
      const detail = (event as CustomEvent<StoredSidebarStateEvent>).detail
      if (
        detail?.storageKey === storageKey &&
        detail.sourceId !== storageSourceId
      ) {
        apply(detail.state)
      }
    }
    const onStorage = (event: StorageEvent) => {
      if (event.key !== `${STORAGE_PREFIX}${storageKey}`) return
      apply(parseStoredState(event.newValue))
    }

    window.addEventListener(STORAGE_SYNC_EVENT, onSync)
    window.addEventListener('storage', onStorage)
    return () => {
      window.removeEventListener(STORAGE_SYNC_EVENT, onSync)
      window.removeEventListener('storage', onStorage)
    }
  }, [
    bounds.max,
    bounds.min,
    controlledCollapsed,
    controlledWidth,
    storageSourceId,
    storageKey,
  ])

  const persist = React.useCallback(
    (next: StoredSidebarState = {}) => {
      if (!storageKey) return
      writeStoredState(
        storageKey,
        {
          width:
            controlledWidth === undefined
              ? (next.width ?? widthRef.current)
              : undefined,
          collapsed:
            controlledCollapsed === undefined
              ? (next.collapsed ?? collapsedRef.current)
              : undefined,
        },
        storageSourceId,
      )
    },
    [controlledCollapsed, controlledWidth, storageKey, storageSourceId],
  )

  const updateWidth = React.useCallback(
    (next: number) => {
      const clamped = clampWidth(next, bounds.min, bounds.max)
      widthRef.current = clamped
      if (controlledWidth === undefined) setInternalWidth(clamped)
      onWidthChange?.(clamped)
    },
    [bounds.max, bounds.min, controlledWidth, onWidthChange],
  )

  const stopDocumentResize = React.useCallback(() => {
    document.body.style.cursor = ''
    document.body.style.userSelect = ''
  }, [])

  const finishResize = React.useCallback(() => {
    if (!dragRef.current) return
    dragRef.current = null
    setIsResizing(false)
    stopDocumentResize()
    persist({ width: widthRef.current })
  }, [persist, stopDocumentResize])

  React.useEffect(
    () => () => {
      dragRef.current = null
      stopDocumentResize()
      if (motionTimerRef.current !== null)
        window.clearTimeout(motionTimerRef.current)
    },
    [stopDocumentResize],
  )

  React.useLayoutEffect(() => {
    const active = document.activeElement
    if (effectiveCollapsed) {
      if (
        active instanceof Node &&
        (expandedRef.current?.contains(active) ||
          resizeRef.current?.contains(active))
      ) {
        toggleRef.current?.focus({ preventScroll: true })
      }
      return
    }
    if (
      active instanceof Node &&
      collapsedActionsRef.current?.contains(active)
    ) {
      const target = expandedRef.current
        ? (firstFocusable(expandedRef.current) ?? expandedRef.current)
        : null
      target?.focus({ preventScroll: true })
      chromeFocusRef.current = false
    }
  }, [effectiveCollapsed])

  const previousNarrowRef = React.useRef(effectiveNarrow)
  React.useLayoutEffect(() => {
    if (previousNarrowRef.current === effectiveNarrow) return
    previousNarrowRef.current = effectiveNarrow
    setAnimateToggle(false)
    if (!effectiveNarrow) return
    const active = document.activeElement
    const chromeHadFocus =
      active === toggleRef.current ||
      (active instanceof Node &&
        collapsedActionsRef.current?.contains(active)) ||
      (chromeFocusRef.current && active === document.body)
    if (!chromeHadFocus) return
    const target = expandedRef.current
      ? (firstFocusable(expandedRef.current) ?? expandedRef.current)
      : null
    target?.focus({ preventScroll: true })
    chromeFocusRef.current = false
  }, [effectiveNarrow])

  const toggle = React.useCallback(() => {
    const next = !preferredCollapsed
    if (next) {
      const active = document.activeElement
      if (
        active instanceof Node &&
        expandedRef.current?.contains(active) &&
        active !== toggleRef.current
      ) {
        toggleRef.current?.focus({ preventScroll: true })
      }
    }
    setAnimateToggle(true)
    if (motionTimerRef.current !== null)
      window.clearTimeout(motionTimerRef.current)
    motionTimerRef.current = window.setTimeout(() => {
      motionTimerRef.current = null
      setAnimateToggle(false)
    }, MOTION_CLEANUP_MS)
    collapsedRef.current = next
    if (controlledCollapsed === undefined) setInternalCollapsed(next)
    onCollapsedChange?.(next)
    if (controlledCollapsed === undefined) persist({ collapsed: next })
  }, [controlledCollapsed, onCollapsedChange, persist, preferredCollapsed])

  // Preserve the original zero-behavior primitive for existing consumers.
  if (!managed) {
    return (
      <aside
        ref={asideRef}
        style={{
          width: effectiveNarrow ? '100%' : (controlledWidth ?? defaultWidth),
          ...style,
        }}
        className={cn(
          'shrink-0 min-h-0 flex flex-col overflow-hidden bg-sidebar',
          className,
        )}
        {...rest}
      >
        {children}
      </aside>
    )
  }

  const showResize = resizable && !effectiveNarrow && !effectiveCollapsed
  const enterFrom = side === 'left' ? '-translate-x-1' : 'translate-x-1'
  const ExpandedIcon = side === 'left' ? PanelLeftClose : PanelRightClose
  const CollapsedIcon = side === 'left' ? PanelLeftOpen : PanelRightOpen

  return (
    <aside
      ref={asideRef}
      aria-label={rest['aria-label'] ?? label}
      data-collapsed={effectiveCollapsed ? '' : undefined}
      data-resizing={isResizing ? '' : undefined}
      style={{ ...style, width: effectiveWidth }}
      className={cn(
        'relative shrink-0 min-h-0 overflow-hidden bg-sidebar',
        effectiveNarrow && 'flex-1 min-w-0',
        animateToggle && !isResizing && 'transition-[width]',
        animateToggle &&
          !isResizing &&
          '[transition-duration:var(--motion-duration-panel)] [transition-timing-function:var(--motion-ease-standard)]',
        isResizing && 'select-none',
        className,
      )}
      {...rest}
      onBlurCapture={(event) => {
        const next = event.relatedTarget
        if (next instanceof Node && !event.currentTarget.contains(next)) {
          chromeFocusRef.current = false
        }
        rest.onBlurCapture?.(event)
      }}
    >
      <div
        ref={expandedRef}
        id={contentId}
        tabIndex={-1}
        inert={effectiveCollapsed || undefined}
        aria-hidden={effectiveCollapsed || undefined}
        style={{ width: effectiveNarrow ? '100%' : expandedWidth }}
        className={cn(
          'absolute inset-y-0 flex min-h-0 flex-col overflow-hidden',
          side === 'left' ? 'left-0' : 'right-0',
          effectiveCollapsed
            ? cn('pointer-events-none opacity-0', enterFrom)
            : 'opacity-100 translate-x-0',
          animateToggle && 'transition-[opacity,transform]',
          animateToggle &&
            (effectiveCollapsed
              ? '[transition-duration:var(--motion-duration-fast)] [transition-timing-function:var(--motion-ease-exit)]'
              : '[transition-duration:var(--motion-duration-fast)] [transition-timing-function:var(--motion-ease-enter)]'),
        )}
        onFocusCapture={() => {
          chromeFocusRef.current = false
        }}
      >
        <div
          className={cn(
            'flex min-h-11 shrink-0 items-center gap-2 px-3',
            collapsible &&
              !effectiveNarrow &&
              (side === 'left' ? 'pr-11' : 'pl-11'),
          )}
        >
          {header ?? (
            <span className="min-w-0 flex-1 truncate font-sans text-sm font-medium text-ink">
              {label}
            </span>
          )}
        </div>
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          {children}
        </div>
      </div>

      <div
        ref={collapsedActionsRef}
        inert={!effectiveCollapsed || undefined}
        aria-hidden={!effectiveCollapsed || undefined}
        className={cn(
          'absolute inset-0 flex flex-col items-center gap-1 pt-11 pb-2',
          effectiveCollapsed ? 'opacity-100' : 'pointer-events-none opacity-0',
          animateToggle && 'transition-opacity',
          animateToggle &&
            (effectiveCollapsed
              ? '[transition-duration:var(--motion-duration-fast)] [transition-timing-function:var(--motion-ease-enter)]'
              : '[transition-duration:var(--motion-duration-fast)] [transition-timing-function:var(--motion-ease-exit)]'),
        )}
        onFocusCapture={() => {
          chromeFocusRef.current = true
        }}
      >
        {collapsedActions}
      </div>

      {collapsible && !effectiveNarrow ? (
        <IconButton
          ref={toggleRef}
          label={`${effectiveCollapsed ? 'expand' : 'collapse'} ${label}`}
          tooltipSide={side === 'left' ? 'right' : 'left'}
          aria-expanded={!effectiveCollapsed}
          aria-controls={contentId}
          onClick={toggle}
          onFocus={() => {
            chromeFocusRef.current = true
          }}
          className={cn(
            'absolute top-2 z-20 size-7',
            side === 'left' ? 'right-1' : 'left-1',
          )}
        >
          <span className="relative block size-4" aria-hidden>
            <ExpandedIcon
              className={cn(
                'absolute inset-0 size-4',
                effectiveCollapsed
                  ? 'scale-90 opacity-0'
                  : 'scale-100 opacity-100',
                animateToggle &&
                  'transition-[opacity,transform] [transition-duration:var(--motion-duration-control)] [transition-timing-function:var(--motion-ease-standard)]',
              )}
            />
            <CollapsedIcon
              className={cn(
                'absolute inset-0 size-4',
                effectiveCollapsed
                  ? 'scale-100 opacity-100'
                  : 'scale-90 opacity-0',
                animateToggle &&
                  'transition-[opacity,transform] [transition-duration:var(--motion-duration-control)] [transition-timing-function:var(--motion-ease-standard)]',
              )}
            />
          </span>
        </IconButton>
      ) : null}

      {showResize ? (
        // biome-ignore lint/a11y/useSemanticElements: a separator with slider behavior is the accurate shape
        <div
          ref={resizeRef}
          role="separator"
          aria-orientation="vertical"
          aria-label={`resize ${label}`}
          aria-valuenow={expandedWidth}
          aria-valuemin={bounds.min}
          aria-valuemax={bounds.max}
          tabIndex={0}
          className={cn(
            'group absolute inset-y-0 z-30 w-2 cursor-col-resize touch-none bg-transparent outline-none',
            side === 'left' ? 'right-0' : 'left-0',
          )}
          onPointerDown={(event) => {
            if (event.button !== 0) return
            event.preventDefault()
            setAnimateToggle(false)
            event.currentTarget.setPointerCapture(event.pointerId)
            dragRef.current = {
              pointerId: event.pointerId,
              startWidth: widthRef.current,
              startX: event.clientX,
            }
            setIsResizing(true)
            document.body.style.cursor = 'col-resize'
            document.body.style.userSelect = 'none'
          }}
          onFocus={() => {
            chromeFocusRef.current = true
          }}
          onPointerMove={(event) => {
            const drag = dragRef.current
            if (!drag || drag.pointerId !== event.pointerId) return
            const delta = event.clientX - drag.startX
            updateWidth(drag.startWidth + (side === 'left' ? delta : -delta))
          }}
          onPointerUp={(event) => {
            const drag = dragRef.current
            if (!drag || drag.pointerId !== event.pointerId) return
            try {
              event.currentTarget.releasePointerCapture(event.pointerId)
            } catch {
              // Pointer capture may already have been released by the browser.
            }
            finishResize()
          }}
          onPointerCancel={finishResize}
          onLostPointerCapture={finishResize}
          onKeyDown={(event) => {
            let next: number | null = null
            if (event.key === 'Home') next = bounds.min
            else if (event.key === 'End') next = bounds.max
            else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
              const grows =
                side === 'left'
                  ? event.key === 'ArrowRight'
                  : event.key === 'ArrowLeft'
              next = widthRef.current + (grows ? KEYBOARD_STEP : -KEYBOARD_STEP)
            }
            if (next === null) return
            event.preventDefault()
            setAnimateToggle(false)
            updateWidth(next)
            persist({ width: clampWidth(next, bounds.min, bounds.max) })
          }}
          onDoubleClick={() => {
            setAnimateToggle(false)
            const next = clampWidth(defaultWidth, bounds.min, bounds.max)
            updateWidth(next)
            persist({ width: next })
          }}
        >
          <span
            aria-hidden
            className={cn(
              'pointer-events-none absolute inset-y-0 w-px bg-transparent group-hover:bg-accent/60 group-focus-visible:bg-accent group-active:bg-accent',
              side === 'left' ? 'right-0' : 'left-0',
              isResizing && 'bg-accent',
            )}
          />
        </div>
      ) : null}
    </aside>
  )
}
