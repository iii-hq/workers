import { Columns2, Pencil, Plus, Rows2, X } from 'lucide-react'
import {
  type Dispatch,
  forwardRef,
  type KeyboardEvent,
  type PointerEvent,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react'
import { HoverTip } from './HoverTip'
import { TerminalPane } from './TerminalPane'
import {
  countTerminalPanes,
  MAX_TERMINAL_PANES_PER_TAB,
  MAX_TERMINAL_SESSIONS,
  type TerminalLayoutNode,
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
import {
  createTerminalConnectionCoordinator,
  type TerminalConnectionCoordinator,
} from './terminal-session-state'

export interface TerminalWorkspaceProps {
  state: TerminalWorkspaceState
  dispatch: Dispatch<TerminalWorkspaceAction>
  root: string
  visible: boolean
  router: TerminalOutputRouter | null
  leaseStore: Storage | null
  storageKey: string
  connectionCoordinators: Map<string, TerminalConnectionCoordinator>
}

export interface TerminalWorkspaceHandle {
  closeDisconnected(): Promise<void>
}

interface LayoutContext {
  state: TerminalWorkspaceState
  dispatch: Dispatch<TerminalWorkspaceAction>
  root: string
  visible: boolean
  router: TerminalOutputRouter | null
  leaseStore: Storage | null
  storageKey: string
  tabPaneCount: number
  registerSession: (paneId: string, session: TerminalSession | null) => void
  closePane: (paneId: string) => Promise<void>
  removePane: (paneId: string) => void
  connectionCoordinator: (paneId: string) => TerminalConnectionCoordinator
}

interface SplitDragState {
  start: number
  ratio: number
  size: number
}

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
  const warnings: string[] = []
  for (const lease of leases) {
    if (paneIds.has(lease.paneId)) continue
    try {
      const warning = await reclaim(lease)
      if (warning) warnings.push(warning)
    } catch (error) {
      warnings.push(error instanceof Error ? error.message : String(error))
    }
  }
  return warnings
}

export function pruneTerminalConnectionCoordinators(
  coordinators: Map<string, TerminalConnectionCoordinator>,
  paneIds: ReadonlySet<string>,
): void {
  for (const paneId of coordinators.keys()) {
    if (!paneIds.has(paneId)) coordinators.delete(paneId)
  }
}

function TerminalPaneSlot({
  paneId,
  context,
}: {
  paneId: string
  context: LayoutContext
}) {
  const pane = context.state.panes[paneId]
  const focused = context.state.focusedPaneId === paneId
  const splitDisabled =
    context.tabPaneCount >= MAX_TERMINAL_PANES_PER_TAB ||
    Object.keys(context.state.panes).length >= MAX_TERMINAL_SESSIONS
  const session = useTerminalSession({
    paneId,
    root: pane?.cwd ?? context.root,
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
      onPointerDown={() => context.dispatch({ type: 'pane-focused', paneId })}
    >
      <TerminalPane
        session={session}
        actions={
          <>
            <HoverTip label="Split right">
              <button
                type="button"
                className="shui-terminal-action"
                onClick={() => split('horizontal')}
                aria-label="Split right"
                disabled={splitDisabled}
              >
                <Columns2 aria-hidden />
              </button>
            </HoverTip>
            <HoverTip label="Split down">
              <button
                type="button"
                className="shui-terminal-action"
                onClick={() => split('vertical')}
                aria-label="Split down"
                disabled={splitDisabled}
              >
                <Rows2 aria-hidden />
              </button>
            </HoverTip>
            <HoverTip
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
            </HoverTip>
          </>
        }
      />
    </div>
  )
}

function TerminalSplit({
  node,
  context,
}: {
  node: Extract<TerminalLayoutNode, { type: 'split' }>
  context: LayoutContext
}) {
  const containerRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<SplitDragState | null>(null)
  const horizontal = node.direction === 'horizontal'

  const resize = (ratio: number) => {
    context.dispatch({ type: 'split-resized', splitId: node.id, ratio })
  }

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    const rect = containerRef.current?.getBoundingClientRect()
    const size = horizontal ? rect?.width : rect?.height
    if (!size) return
    dragRef.current = {
      start: horizontal ? event.clientX : event.clientY,
      ratio: node.ratio,
      size,
    }
    event.currentTarget.setPointerCapture(event.pointerId)
    event.preventDefault()
  }

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current
    if (!drag) return
    const current = horizontal ? event.clientX : event.clientY
    resize(drag.ratio + (current - drag.start) / drag.size)
  }

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    dragRef.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const decrease =
      (horizontal && event.key === 'ArrowLeft') ||
      (!horizontal && event.key === 'ArrowUp')
    const increase =
      (horizontal && event.key === 'ArrowRight') ||
      (!horizontal && event.key === 'ArrowDown')
    if (!decrease && !increase) return
    event.preventDefault()
    resize(node.ratio + (increase ? 0.05 : -0.05))
  }

  return (
    <div
      ref={containerRef}
      className={`shui-terminal-split ${node.direction}`}
      data-terminal-split-id={node.id}
    >
      <div
        className="shui-terminal-split-child"
        style={{ flexBasis: `${node.ratio * 100}%` }}
      >
        <TerminalLayoutView node={node.first} context={context} />
      </div>
      {/* biome-ignore lint/a11y/useSemanticElements: this is an interactive range separator, not a static thematic break. */}
      <div
        role="separator"
        tabIndex={0}
        className="shui-terminal-split-separator"
        aria-label={`Resize ${node.direction} terminal split`}
        aria-orientation={horizontal ? 'vertical' : 'horizontal'}
        aria-valuemin={20}
        aria-valuemax={80}
        aria-valuenow={Math.round(node.ratio * 100)}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onLostPointerCapture={onPointerUp}
        onKeyDown={onKeyDown}
      />
      <div className="shui-terminal-split-child">
        <TerminalLayoutView node={node.second} context={context} />
      </div>
    </div>
  )
}

function TerminalLayoutView({
  node,
  context,
}: {
  node: TerminalLayoutNode
  context: LayoutContext
}) {
  switch (node.type) {
    case 'pane':
      return <TerminalPaneSlot paneId={node.paneId} context={context} />
    case 'split':
      return <TerminalSplit node={node} context={context} />
    default: {
      const exhaustive: never = node
      return exhaustive
    }
  }
}

export const TerminalWorkspace = forwardRef<
  TerminalWorkspaceHandle,
  TerminalWorkspaceProps
>(function TerminalWorkspace(
  {
    state,
    dispatch,
    root,
    visible,
    router,
    leaseStore,
    storageKey,
    connectionCoordinators,
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

  const closePane = useCallback(
    async (paneId: string) => {
      setError(null)
      try {
        const warning = await closePty(paneId)
        dispatch({ type: 'pane-closed', paneId })
        if (warning) setError(warning)
      } catch (closeError) {
        setError(
          closeError instanceof Error ? closeError.message : String(closeError),
        )
      }
    },
    [closePty, dispatch],
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
        !window.confirm(
          `Close "${tab.title}" and ${paneIds.length} terminal session${paneIds.length === 1 ? '' : 's'}?`,
        )
      ) {
        return
      }
      setError(null)
      const warnings: string[] = []
      for (const paneId of paneIds) {
        try {
          const warning = await closePty(paneId)
          dispatch({ type: 'pane-closed', paneId })
          if (warning) warnings.push(warning)
        } catch (closeError) {
          setError(
            closeError instanceof Error
              ? closeError.message
              : String(closeError),
          )
          return
        }
      }
      if (warnings.length > 0) setError(warnings.join('; '))
    },
    [closePty, dispatch, state.tabs],
  )

  const closeDisconnected = useCallback(async () => {
    const activePaneIds = new Set(
      selectedTab ? paneIdsInLayout(selectedTab.layout) : [],
    )
    const disconnected = Object.keys(state.panes).filter((paneId) => {
      const session = sessionsRef.current.get(paneId)
      return (
        !activePaneIds.has(paneId) ||
        session?.status === 'disconnected' ||
        session?.status === 'error' ||
        session?.status === 'exited'
      )
    })
    for (const paneId of disconnected) {
      await closePane(paneId)
    }
  }, [closePane, selectedTab, state.panes])

  useImperativeHandle(ref, () => ({ closeDisconnected }), [closeDisconnected])

  const context = useMemo<LayoutContext | null>(() => {
    if (!selectedTab) return null
    return {
      state,
      dispatch,
      root,
      visible,
      router,
      leaseStore,
      storageKey,
      tabPaneCount: countTerminalPanes(selectedTab.layout),
      registerSession,
      closePane,
      removePane,
      connectionCoordinator,
    }
  }, [
    closePane,
    connectionCoordinator,
    dispatch,
    leaseStore,
    registerSession,
    removePane,
    root,
    router,
    selectedTab,
    state,
    storageKey,
    visible,
  ])

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
      <div
        className="shui-terminal-tabs"
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
                  onClick={() =>
                    dispatch({ type: 'tab-selected', tabId: tab.id })
                  }
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
        <HoverTip label="New terminal">
          <button
            type="button"
            className="shui-terminal-tab-new"
            onClick={createTab}
            aria-label="New terminal"
            disabled={totalPaneCount >= MAX_TERMINAL_SESSIONS}
          >
            <Plus aria-hidden />
          </button>
        </HoverTip>
      </div>
      {error ? (
        <div className="shui-terminal-workspace-error">{error}</div>
      ) : null}
      <div className="shui-terminal-layout">
        {selectedTab && context ? (
          <TerminalLayoutView node={selectedTab.layout} context={context} />
        ) : (
          <div className="shui-terminal-empty">No terminal sessions</div>
        )}
      </div>
    </div>
  )
})
