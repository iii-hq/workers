import { Toolbar, Tooltip, useConfirm } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Columns2, Pencil, Plus, Rows2, X } from 'lucide-react'
import {
  type CSSProperties,
  type Dispatch,
  forwardRef,
  type ReactNode,
  type KeyboardEvent,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react'
import { TerminalPane } from './TerminalPane'
import {
  clampSplitRatio,
  countTerminalPanes,
  MAX_TERMINAL_PANES_PER_TAB,
  MAX_TERMINAL_SESSIONS,
  type TerminalLayoutItem,
  type TerminalLayoutNode,
  type TerminalPaneState,
  type TerminalTabState,
  terminalLayoutItems,
  type TerminalWorkspaceAction,
  type TerminalWorkspaceState,
} from './terminal-layout'
import {
  type LocalTerminalLease,
  loadRecoverableTerminalLeases,
  removeRecoverableTerminalLease,
  saveRecoverableTerminalLease,
} from './terminal-leases'
import {
  type TerminalOutputRouter,
  terminalOutputRouterHost,
} from './terminal-output-router'
import {
  reclaimTerminalLease,
  type TerminalSession,
  useTerminalSession,
} from './terminal-session'
import { useSplitDrag } from '@iii-dev/console-ui/hooks'
import {
  createTerminalConnectionCoordinator,
  type TerminalConnectionCoordinator,
} from './terminal-session-state'

export interface TerminalWorkspaceProps {
  state: TerminalWorkspaceState
  dispatch: Dispatch<TerminalWorkspaceAction>
  root: string
  visible: boolean
  /**
   * Out of sight but still mounted: every session stays attached and every
   * xterm keeps its scrollback, so showing it again costs nothing.
   */
  hidden?: boolean
  /** The page is narrow: side-by-side splits stack top to bottom. */
  narrow?: boolean
  router: TerminalOutputRouter | null
  leaseStore: Storage | null
  storageKey: string
  connectionCoordinators: Map<string, TerminalConnectionCoordinator>
  actions?: ReactNode
}

export interface TerminalWorkspaceHandle {
  closeDisconnected(): Promise<void>
}

/**
 * What every pane shares. Nothing in it changes with a tab, a focus or a
 * drag, so a memoized pane re-renders only when its own props do.
 */
interface PaneContext {
  dispatch: Dispatch<TerminalWorkspaceAction>
  visible: boolean
  router: TerminalOutputRouter | null
  leaseStore: Storage | null
  storageKey: string
  registerSession: (paneId: string, session: TerminalSession | null) => void
  closePane: (paneId: string) => Promise<void>
  removePane: (paneId: string) => void
  connectionCoordinator: (paneId: string) => TerminalConnectionCoordinator
}

type SplitDrag = { splitId: string; ratio: number }

/** Half the split handle's 5px: an inner pane edge leaves it to the handle. */
const HALF_HANDLE_PX = 2.5

let generatedId = 0

function createId(prefix: string): string {
  generatedId += 1
  return `${prefix}-${Date.now().toString(36)}-${generatedId.toString(36)}`
}

function paneIdsInLayout(node: TerminalLayoutNode): string[] {
  if (node.type === 'pane') return [node.paneId]
  return [...paneIdsInLayout(node.first), ...paneIdsInLayout(node.second)]
}

function activeTab(state: TerminalWorkspaceState) {
  return state.tabs.find((tab) => tab.id === state.activeTabId) ?? null
}

export async function reconcileTerminalWorkspaceLeases(
  leases: readonly LocalTerminalLease[],
  paneIds: ReadonlySet<string>,
  reclaim: (lease: LocalTerminalLease) => Promise<string | null>,
): Promise<string[]> {
  const orphans = leases.filter((lease) => !paneIds.has(lease.paneId))
  const results = await Promise.allSettled(
    orphans.map(async (lease) => reclaim(lease)),
  )
  return results.flatMap((result) => {
    if (result.status === 'rejected') return [errorMessage(result.reason)]
    return result.value ? [result.value] : []
  })
}

/**
 * Closes the panes' shells all at once, since each close waits for its shell
 * to die, and settles before anything is removed: removing panes one by one
 * re-laid the tab out around the ones still closing. `closed` is what the
 * caller removes; what failed stays. `messages` holds every warning and error.
 */
export async function closeTerminalPanes(
  paneIds: readonly string[],
  closePty: (paneId: string) => Promise<string | null>,
): Promise<{ closed: string[]; messages: string[] }> {
  const results = await Promise.allSettled(
    paneIds.map(async (paneId) => closePty(paneId)),
  )
  const closed: string[] = []
  const messages: string[] = []
  results.forEach((result, index) => {
    if (result.status === 'rejected') {
      messages.push(errorMessage(result.reason))
      return
    }
    closed.push(paneIds[index])
    if (result.value) messages.push(result.value)
  })
  return { closed, messages }
}

export function pruneTerminalConnectionCoordinators(
  coordinators: Map<string, TerminalConnectionCoordinator>,
  paneIds: ReadonlySet<string>,
): void {
  for (const paneId of coordinators.keys()) {
    if (!paneIds.has(paneId)) coordinators.delete(paneId)
  }
}

/** Where a pane sits in its tab, clear of the handles along its inner edges. */
function paneStyle(
  left: number,
  top: number,
  width: number,
  height: number,
): CSSProperties {
  const gaps = (start: number, size: number) => ({
    before: start > 0 ? HALF_HANDLE_PX : 0,
    after: start + size < 1 - 1e-6 ? HALF_HANDLE_PX : 0,
  })
  const x = gaps(left, width)
  const y = gaps(top, height)
  return {
    left: `calc(${left * 100}% + ${x.before}px)`,
    top: `calc(${top * 100}% + ${y.before}px)`,
    width: `calc(${width * 100}% - ${x.before + x.after}px)`,
    height: `calc(${height * 100}% - ${y.before + y.after}px)`,
  }
}

const TerminalPaneSlot = memo(function TerminalPaneSlot({
  paneId,
  cwd,
  focused,
  docked,
  splitDisabled,
  left,
  top,
  width,
  height,
  context,
}: {
  paneId: string
  cwd: string
  focused: boolean
  docked: boolean
  splitDisabled: boolean
  left: number
  top: number
  width: number
  height: number
  context: PaneContext
}) {
  const session = useTerminalSession({
    paneId,
    root: cwd,
    visible: context.visible,
    router: context.router,
    leaseStore: context.leaseStore,
    storageKey: context.storageKey,
    connectionCoordinator: context.connectionCoordinator(paneId),
  })

  useEffect(() => {
    context.registerSession(paneId, session)
    return () => context.registerSession(paneId, null)
  }, [context, paneId, session])

  const split = (direction: 'horizontal' | 'vertical') => {
    context.dispatch({
      type: 'pane-split',
      paneId,
      newPaneId: createId('pane'),
      splitId: createId('split'),
      direction,
    })
  }

  return (
    <div
      className={`shui-terminal-pane-slot${focused ? ' focused' : ''}`}
      data-terminal-pane-id={paneId}
      style={paneStyle(left, top, width, height)}
      onPointerDown={() => context.dispatch({ type: 'pane-focused', paneId })}
    >
      <TerminalPane
        session={session}
        docked={docked}
        actions={
          <>
            <Tooltip label="Split right">
              <button
                type="button"
                className="shui-terminal-action"
                onClick={() => split('horizontal')}
                aria-label="Split right"
                disabled={splitDisabled}
              >
                <Columns2 aria-hidden />
              </button>
            </Tooltip>
            <Tooltip label="Split down">
              <button
                type="button"
                className="shui-terminal-action"
                onClick={() => split('vertical')}
                aria-label="Split down"
                disabled={splitDisabled}
              >
                <Rows2 aria-hidden />
              </button>
            </Tooltip>
            <Tooltip
              label={
                session.status === 'disconnected'
                  ? 'Remove terminal pane'
                  : 'Close terminal pane'
              }
            >
              <button
                type="button"
                className="shui-terminal-action"
                onClick={() => {
                  if (session.status === 'disconnected') {
                    session.forget()
                    context.removePane(paneId)
                    return
                  }
                  void context.closePane(paneId)
                }}
                aria-label={
                  session.status === 'disconnected'
                    ? 'Remove terminal pane'
                    : 'Close terminal pane'
                }
              >
                <X aria-hidden />
              </button>
            </Tooltip>
          </>
        }
      />
    </div>
  )
})

function percent(fraction: number): string {
  return `${fraction * 100}%`
}

function TerminalSplitHandle({
  item,
  dispatch,
  onDrag,
}: {
  item: Extract<TerminalLayoutItem, { type: 'separator' }>
  dispatch: Dispatch<TerminalWorkspaceAction>
  onDrag: (drag: SplitDrag | null) => void
}) {
  const { split, direction, ratio, rect } = item
  const horizontal = direction === 'horizontal'

  const resize = (next: number) => {
    dispatch({ type: 'split-resized', splitId: split.id, ratio: next })
  }
  // A drag moves the split in its tab and tells the page once, on release.
  const dragRatioRef = useRef<number | null>(null)
  const commitDrag = () => {
    const next = dragRatioRef.current
    if (next === null) return
    dragRatioRef.current = null
    resize(next)
    onDrag(null)
  }

  const resizer = useSplitDrag<{ ratio: number; size: number }>({
    horizontal,
    begin: (event) => {
      // The split's share of the tab's area, in pixels along the drag.
      const area = event.currentTarget.parentElement?.getBoundingClientRect()
      const size = horizontal
        ? (area?.width ?? 0) * rect.width
        : (area?.height ?? 0) * rect.height
      return size ? { ratio: split.ratio, size } : null
    },
    move: (origin, delta) => {
      const next = clampSplitRatio(origin.ratio + delta / origin.size)
      dragRatioRef.current = next
      onDrag({ splitId: split.id, ratio: next })
    },
    step: (towards) => resize(split.ratio + towards * 0.05),
  })

  const at = horizontal
    ? rect.left + rect.width * ratio
    : rect.top + rect.height * ratio
  const style: CSSProperties = horizontal
    ? {
        left: percent(at),
        top: percent(rect.top),
        height: percent(rect.height),
      }
    : {
        top: percent(at),
        left: percent(rect.left),
        width: percent(rect.width),
      }

  return (
    // biome-ignore lint/a11y/useSemanticElements: this is an interactive range separator, not a static thematic break.
    <div
      role="separator"
      tabIndex={0}
      className={`shui-terminal-split-separator ${direction}`}
      style={style}
      aria-label={`Resize ${direction} terminal split`}
      aria-orientation={horizontal ? 'vertical' : 'horizontal'}
      aria-valuemin={20}
      aria-valuemax={80}
      aria-valuenow={Math.round(ratio * 100)}
      {...resizer}
      onPointerUp={(event) => {
        resizer.onPointerUp(event)
        commitDrag()
      }}
      onPointerCancel={(event) => {
        resizer.onPointerCancel(event)
        commitDrag()
      }}
      onLostPointerCapture={(event) => {
        resizer.onLostPointerCapture(event)
        commitDrag()
      }}
    />
  )
}

/**
 * One tab's panes and split handles, flat and keyed by id (see
 * `terminalLayoutItems`). Every tab stays mounted, the inactive ones hidden,
 * so switching tabs detaches nothing and replays nothing.
 */
const TerminalTabLayout = memo(function TerminalTabLayout({
  layout,
  hidden,
  focusedPaneId,
  panes,
  root,
  sessionsFull,
  stacked,
  context,
}: {
  layout: TerminalLayoutNode
  hidden: boolean
  focusedPaneId: string | null
  panes: Record<string, TerminalPaneState>
  root: string
  sessionsFull: boolean
  stacked: boolean
  context: PaneContext
}) {
  // Held here, not on the page: a pointer move redraws this tab alone, and
  // of its panes only the ones the move resizes.
  const [drag, setDrag] = useState<SplitDrag | null>(null)
  const paneCount = countTerminalPanes(layout)
  const splitDisabled =
    sessionsFull || paneCount >= MAX_TERMINAL_PANES_PER_TAB

  return (
    <div className="shui-terminal-tab-layout" hidden={hidden}>
      {terminalLayoutItems(layout, { stacked, drag }).map((item) =>
        item.type === 'pane' ? (
          <TerminalPaneSlot
            key={`pane:${item.paneId}`}
            paneId={item.paneId}
            cwd={panes[item.paneId]?.cwd ?? root}
            focused={item.paneId === focusedPaneId}
            docked={paneCount > 1}
            splitDisabled={splitDisabled}
            left={item.rect.left}
            top={item.rect.top}
            width={item.rect.width}
            height={item.rect.height}
            context={context}
          />
        ) : (
          <TerminalSplitHandle
            key={`split:${item.split.id}`}
            item={item}
            dispatch={context.dispatch}
            onDrag={setDrag}
          />
        ),
      )}
    </div>
  )
})

export const TerminalWorkspace = forwardRef<
  TerminalWorkspaceHandle,
  TerminalWorkspaceProps
>(function TerminalWorkspace(
  {
    state,
    dispatch,
    root,
    visible,
    hidden = false,
    narrow = false,
    router,
    leaseStore,
    storageKey,
    connectionCoordinators,
    actions,
  },
  ref,
) {
  const sessionsRef = useRef(new Map<string, TerminalSession>())
  const orphanReclaimsRef = useRef(
    new Map<string, Promise<string | null>>(),
  )
  const [editingTabId, setEditingTabId] = useState<string | null>(null)
  const [editingTitle, setEditingTitle] = useState('')
  const [error, setError] = useState<string | null>(null)
  const { confirm, dialog } = useConfirm()
  const selectedTab = activeTab(state)
  const totalPaneCount = Object.keys(state.panes).length

  const registerSession = useCallback(
    (paneId: string, session: TerminalSession | null) => {
      if (session) {
        sessionsRef.current.set(paneId, session)
      } else {
        sessionsRef.current.delete(paneId)
      }
    },
    [],
  )

  const connectionCoordinator = useCallback(
    (paneId: string) => {
      const existing = connectionCoordinators.get(paneId)
      if (existing) return existing
      const created = createTerminalConnectionCoordinator()
      connectionCoordinators.set(paneId, created)
      return created
    },
    [connectionCoordinators],
  )

  const closePty = useCallback(
    async (paneId: string): Promise<string | null> => {
      const session = sessionsRef.current.get(paneId)
      if (session) {
        return session.close()
      }
      if (!router) return null
      const lease = loadRecoverableTerminalLeases(leaseStore, storageKey).find(
        (entry) => entry.paneId === paneId,
      )
      if (!lease) return null
      return reclaimTerminalLease(terminalOutputRouterHost(router), router, {
        ...lease,
        update: (updated) =>
          saveRecoverableTerminalLease(leaseStore, storageKey, updated),
        remove: () =>
          removeRecoverableTerminalLease(leaseStore, storageKey, paneId),
      })
    },
    [leaseStore, router, storageKey],
  )

  useEffect(() => {
    const paneIds = new Set(Object.keys(state.panes))
    pruneTerminalConnectionCoordinators(connectionCoordinators, paneIds)
    if (!router) return
    let cancelled = false
    const host = terminalOutputRouterHost(router)
    const leases = loadRecoverableTerminalLeases(leaseStore, storageKey)
    void reconcileTerminalWorkspaceLeases(leases, paneIds, (lease) => {
      const existing = orphanReclaimsRef.current.get(lease.sessionId)
      if (existing) return existing
      const reclaim = reclaimTerminalLease(host, router, {
        ...lease,
        update: (updated) =>
          saveRecoverableTerminalLease(leaseStore, storageKey, updated),
        remove: () =>
          removeRecoverableTerminalLease(
            leaseStore,
            storageKey,
            lease.paneId,
          ),
      }).finally(() => {
        orphanReclaimsRef.current.delete(lease.sessionId)
      })
      orphanReclaimsRef.current.set(lease.sessionId, reclaim)
      return reclaim
    }).then((warnings) => {
      if (!cancelled && warnings.length > 0) setError(warnings.join('; '))
    })
    return () => {
      cancelled = true
    }
  }, [
    connectionCoordinators,
    leaseStore,
    router,
    state.panes,
    storageKey,
  ])

  // The closed panes leave in one render: React batches these dispatches.
  const closePanes = useCallback(
    async (paneIds: readonly string[]) => {
      setError(null)
      const { closed, messages } = await closeTerminalPanes(paneIds, closePty)
      for (const paneId of closed) dispatch({ type: 'pane-closed', paneId })
      if (messages.length > 0) setError(messages.join('; '))
    },
    [closePty, dispatch],
  )

  const closePane = useCallback(
    (paneId: string) => closePanes([paneId]),
    [closePanes],
  )

  const removePane = useCallback(
    (paneId: string) => dispatch({ type: 'pane-closed', paneId }),
    [dispatch],
  )

  const closeTab = useCallback(
    async (tabId: string) => {
      const tab = state.tabs.find((entry) => entry.id === tabId)
      if (!tab) return
      const paneIds = paneIdsInLayout(tab.layout)
      if (
        !(await confirm({
          title: `Close "${tab.title}" and ${paneIds.length} terminal session${paneIds.length === 1 ? '' : 's'}?`,
          confirmLabel: 'Close',
        }))
      ) {
        return
      }
      await closePanes(paneIds)
    },
    [closePanes, state.tabs, confirm],
  )

  // Every tab's panes are mounted and attached, so a pane in a background
  // tab is not disconnected: only a pane whose session says so is (or one
  // with no session, which never attached).
  const closeDisconnected = useCallback(async () => {
    const disconnected = Object.keys(state.panes).filter((paneId) => {
      const status = sessionsRef.current.get(paneId)?.status
      return (
        status === undefined ||
        status === 'disconnected' ||
        status === 'error' ||
        status === 'exited'
      )
    })
    await closePanes(disconnected)
  }, [closePanes, state.panes])

  useImperativeHandle(ref, () => ({ closeDisconnected }), [closeDisconnected])

  const context = useMemo<PaneContext>(
    () => ({
      dispatch,
      visible,
      router,
      leaseStore,
      storageKey,
      registerSession,
      closePane,
      removePane,
      connectionCoordinator,
    }),
    [
      closePane,
      connectionCoordinator,
      dispatch,
      leaseStore,
      registerSession,
      removePane,
      router,
      storageKey,
      visible,
    ],
  )

  // Mounting a terminal used to focus it. Panes now stay mounted while the
  // panel is hidden, so showing it hands the keyboard to the focused pane.
  const focusedPaneIdRef = useRef(state.focusedPaneId)
  focusedPaneIdRef.current = state.focusedPaneId
  useEffect(() => {
    if (hidden) return
    const frame = window.requestAnimationFrame(() => {
      const paneId = focusedPaneIdRef.current
      if (paneId) sessionsRef.current.get(paneId)?.focus()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [hidden])

  // The same for a tab: its panes were there all along, hidden, so a click
  // that brings one forward focuses the pane the tab opens on.
  const selectTab = (tab: TerminalTabState) => {
    const switching = tab.id !== state.activeTabId
    dispatch({ type: 'tab-selected', tabId: tab.id })
    // Not on the tab already in front: its double click is a rename.
    if (!switching) return
    const paneId = paneIdsInLayout(tab.layout)[0]
    window.requestAnimationFrame(() => sessionsRef.current.get(paneId)?.focus())
  }

  const createTab = () => {
    dispatch({
      type: 'tab-created',
      tabId: createId('tab'),
      paneId: createId('pane'),
      root,
    })
  }

  const beginRename = (tabId: string, title: string) => {
    setEditingTabId(tabId)
    setEditingTitle(title)
  }

  const commitRename = () => {
    if (editingTabId && editingTitle.trim()) {
      dispatch({
        type: 'tab-renamed',
        tabId: editingTabId,
        title: editingTitle.trim(),
      })
    }
    setEditingTabId(null)
  }

  const selectTabByKey = (
    event: KeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => {
    let targetIndex: number | null = null
    switch (event.key) {
      case 'ArrowLeft':
        targetIndex = (index - 1 + state.tabs.length) % state.tabs.length
        break
      case 'ArrowRight':
        targetIndex = (index + 1) % state.tabs.length
        break
      case 'Home':
        targetIndex = 0
        break
      case 'End':
        targetIndex = state.tabs.length - 1
        break
      default:
        return
    }
    event.preventDefault()
    const target = state.tabs[targetIndex]
    if (!target) return
    dispatch({ type: 'tab-selected', tabId: target.id })
    const tablist = event.currentTarget.closest('[role="tablist"]')
    window.requestAnimationFrame(() => {
      const tabs = tablist?.querySelectorAll<HTMLButtonElement>('[role="tab"]')
      tabs?.[targetIndex]?.focus()
    })
  }

  return (
    <div className="shui-terminal-workspace">
      {dialog}
      <Toolbar className="shui-terminal-tabs" aria-label="Terminals" end={actions}>
        <div
          className="shui-terminal-tab-strip"
          role="tablist"
          aria-label="Terminal tabs"
        >
        {state.tabs.map((tab, index) => {
          const selected = tab.id === state.activeTabId
          return (
            <div
              className={`shui-terminal-tab${selected ? ' active' : ''}`}
              key={tab.id}
            >
              {editingTabId === tab.id ? (
                <input
                  ref={(input) => input?.focus()}
                  className="shui-terminal-tab-input"
                  aria-label="Rename terminal tab"
                  value={editingTitle}
                  onChange={(event) => setEditingTitle(event.target.value)}
                  onBlur={commitRename}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') commitRename()
                    if (event.key === 'Escape') setEditingTabId(null)
                  }}
                />
              ) : (
                <button
                  type="button"
                  role="tab"
                  aria-selected={selected}
                  className="shui-terminal-tab-select"
                  onClick={() => selectTab(tab)}
                  onDoubleClick={() => beginRename(tab.id, tab.title)}
                  onKeyDown={(event) => selectTabByKey(event, index)}
                >
                  {tab.title}
                </button>
              )}
              <button
                type="button"
                className="shui-terminal-tab-action"
                aria-label={`Rename ${tab.title}`}
                onClick={() => beginRename(tab.id, tab.title)}
              >
                <Pencil aria-hidden />
              </button>
              <button
                type="button"
                className="shui-terminal-tab-action"
                aria-label={`Close ${tab.title}`}
                onClick={() => void closeTab(tab.id)}
              >
                <X aria-hidden />
              </button>
            </div>
          )
        })}
        <Tooltip label="New terminal">
          <button
            type="button"
            className="shui-terminal-tab-new"
            onClick={createTab}
            aria-label="New terminal"
            disabled={totalPaneCount >= MAX_TERMINAL_SESSIONS}
          >
            <Plus aria-hidden />
          </button>
        </Tooltip>
        </div>
      </Toolbar>
      {error ? (
        <div className="shui-terminal-workspace-error">{error}</div>
      ) : null}
      <div className="shui-terminal-layout">
        {state.tabs.map((tab) => {
          const active = tab.id === state.activeTabId
          return (
            <TerminalTabLayout
              key={tab.id}
              layout={tab.layout}
              hidden={!active}
              focusedPaneId={active ? state.focusedPaneId : null}
              panes={state.panes}
              root={root}
              sessionsFull={totalPaneCount >= MAX_TERMINAL_SESSIONS}
              stacked={narrow}
              context={context}
            />
          )
        })}
        {selectedTab ? null : (
          <div className="shui-terminal-empty">No terminal sessions</div>
        )}
      </div>
    </div>
  )
})
