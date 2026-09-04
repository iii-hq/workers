/**
 * The shell explorer page (#/ext/shell): an editor-shaped surface over
 * the worker's own functions. One tab strip holds everything the main
 * pane can show: a file (its real content, editable), a diff (one file
 * against one source: the index, a Harness turn, a revision, a recorded
 * change) and, when docked there, the terminal.
 *
 * What a click opens is decided by the sidebar view it comes from:
 * Explorer opens files, Source control opens index diffs (a file can be
 * open twice, staged and unstaged), Timeline opens the diff of one turn.
 *
 * The sidebar hugs the pane's OUTER edge (`panelSide`), and the UI state
 * (browsed root, open tabs per folder, expanded folders, view, diff
 * options, terminal layout) persists per pane (`paneId`, `pane-scope.ts`)
 * in the `shell-ui` configuration entry. The same page can be open
 * several times in one workspace tab, each pane on its own folder with
 * its own terminals and its own live triggers.
 */

import {
  Button,
  ConfirmDialog,
  DirectoryPicker,
  type Host,
  PageBody,
  PageHeader,
  PageMain,
  Panel,
  type PageRenderProps,
  PageShell,
  PageSidebar,
} from '@iii-dev/console-ui'
import type { GitStatusEntry } from '@pierre/trees'
import {
  ArrowLeft,
  ArrowRight,
  CircleAlert,
  Eye,
  EyeOff,
  FolderX,
  PanelLeft,
  PanelRight,
  RefreshCw,
  SquareTerminal,
  Terminal,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { ActivityBar, type SideView } from './ActivityBar'
import {
  type MissingPaths,
  missingAfterChanges,
  missingFromStats,
  NO_MISSING,
  pruneMissing,
  withMissing,
  withMissingPaths,
} from './missing-files'
import { type CoderInfo, coderInfo, coderReadFile, coderStatFiles, joinPath, workspaceValidate } from './coder'
import { createTurnCache, loadDiffContents } from './diff-load'
import { type DiffSource, diffSourceFollowsDisk } from './diff-source'
import { DEFAULT_DIFF_OPTIONS, DiffTab, type DiffOptions, type DiffTabActions, type DiffTabState } from './DiffTab'
import { EditorTabs } from './EditorTabs'
import { type EditorCache, EditorPane } from './EditorPane'
import { refreshCleanEditorCacheEntry } from './editor-cache'
import { copyText, createEntry, deleteEntry, duplicateFile, duplicateName, renameEntry } from './file-actions'
import { createObjectUrlRegistry } from './file-bytes'
import { type ExplorerActions, FilesTab } from './FilesTab'
import { type GitChange, type GitState, gitChanges } from './git'
import { gitDiscard } from './git-actions'
import { HoverTip } from './HoverTip'
import { EDITOR_FULL_READ_BUDGET } from './large-file'
import { useWorkspaceChanges } from './live'
import {
  canGoBack,
  canGoForward,
  EMPTY_HISTORY,
  forgetPath,
  goBack,
  goForward,
  type NavHistory,
  pushLocation,
  recentPaths,
} from './nav-history'
import { paneScopeToken, paneStateKey } from './pane-scope'
import { parseShellPanelContext } from './panel-context'
import { dirname, isUnder } from './paths'
import { createTabUiStateSaver, loadTabUiState, type TabUiState, type TerminalDock } from './persist'
import { useShellReviewSummaryBridge } from './review-summary-store'
import {
  EMPTY_ROOT_MEMORY,
  parseRootMemory,
  recallRoot,
  rememberRoot,
  type RootMemory,
  serializeRootMemory,
} from './root-memory'
import { SearchTab, type SearchRequest } from './SearchTab'
import { ShellLauncher } from './ShellLauncher'
import { SourceControlTab } from './SourceControlTab'
import {
  activateTab,
  activeTab as activeTabOf,
  closeTab,
  cycleTab,
  diffTarget,
  EMPTY_TABS,
  fileTabId,
  fileTarget,
  findTab,
  lastSegments,
  openPinned,
  openPreview,
  persistedTabs,
  pinTab,
  restoreTabs,
  type TabTarget,
  type TabsState,
  tabIdFor,
} from './tabs'
import { createTerminalWorkspace, normalizeTerminalWorkspace, reduceTerminalWorkspace } from './terminal-layout'
import type { TerminalOutputRouter } from './terminal-output-router'
import type { TerminalConnectionCoordinator } from './terminal-session-state'
import { TerminalPanel } from './TerminalPanel'
import { TimelineTab } from './TimelineTab'
import type { TreeChange } from './tree-model'
import { describeRevert, revertTurn } from './turn-revert'
import { useHarnessTurn } from './turn'
import { fetchSessionTurns, relativeToRoot, type SessionTurnSummary, turnTitle } from './turns'
import { useCompareRefs } from './use-compare-refs'
import { useSourceControl } from './use-source-control'
import { useTurnSummary } from './use-turn-summary'
import { useWorkspaceTree } from './use-workspace-tree'
import { WorkspaceBrowser } from './WorkspaceBrowser'

import {
  acknowledgeUnavailableWorkingDirectory,
  acknowledgeValidatedWorkingDirectory,
  deepLinkRootTarget,
  ownsRequestToken,
  ownsScopedRequestToken,
  type RootTargetValidation,
  rebasePathAfterValidation,
  rootValidationRetryDelay,
  type ScopedRequestToken,
  validateRootTarget,
  workingDirectoryFollowRetryDelay,
  workingDirectoryNeedsFollow,
  workingDirectoryRetryMessage,
} from './working-dir-sync'

type RootChangeOutcome = RootTargetValidation['outcome'] | 'declined'

type WorkingDirectoryHost = Host & {
  chat?: {
    requestWorkingDirectoryChange?(request: { sessionId: string; path: string }): boolean
  }
}

const SIDEBAR_DEFAULT_WIDTH = 260
const SIDEBAR_MIN_WIDTH = 200
const SIDEBAR_MAX_WIDTH = 560
const TERMINAL_BOTTOM_DEFAULT_SIZE = 280
const TERMINAL_RIGHT_DEFAULT_SIZE = 420
const LIVE_COALESCE_MS = 400

function clampTerminalSize(size: number | undefined, fallback: number): number {
  if (size === undefined || !Number.isFinite(size)) return fallback
  return Math.min(1200, Math.max(160, Math.round(size)))
}

function isSideView(value: unknown): value is SideView {
  return value === 'files' || value === 'search' || value === 'scm' || value === 'timeline'
}

interface DiffCacheEntry {
  epoch: number
  state: DiffTabState
}

export function ShellExplorerPage({
  host,
  terminalRouter,
  panelSide,
  tabId,
  paneId,
  onRequestClose,
  workingDir,
  panelContext,
  conversationId,
  commands,
  setDirty,
}: { host: Host; terminalRouter: TerminalOutputRouter } & PageRenderProps) {
  const theme = host.useTheme()
  // Everything this instance keeps for itself is keyed by the pane.
  const paneKey = paneStateKey(tabId, paneId)
  const paneScope = paneScopeToken(paneKey)
  const harnessTurn = useHarnessTurn(host, conversationId, paneScope)

  // ── root ──
  const [info, setInfo] = useState<CoderInfo | null>(null)
  const [infoError, setInfoError] = useState<string | null>(null)
  const [restored, setRestored] = useState<TabUiState | null | 'loading'>('loading')
  const [root, setRoot] = useState<string | null>(null)
  // The root the picker is validating right now: the select holds this
  // value so the choice never appears to snap back, and the files pane
  // says what it is opening instead of sitting empty.
  const [pendingRoot, setPendingRoot] = useState<string | null>(null)
  // A folder picked here sticks across reloads, whatever the chat says.
  const [rootPinned, setRootPinned] = useState(false)
  // What was open in the folders this pane browsed before.
  const rootMemoryRef = useRef<RootMemory>(EMPTY_ROOT_MEMORY)
  const rootRef = useRef(root)
  rootRef.current = root
  const workingDirRef = useRef(workingDir ?? null)
  workingDirRef.current = workingDir ?? null
  const acknowledgedWorkingDirRef = useRef<string | null>(null)
  const workingDirFollowRequestSeqRef = useRef(0)
  const workingDirFollowPendingRef = useRef<{ path: string; request: number } | null>(null)
  const workingDirRetryRef = useRef({ path: null as string | null, failures: 0 })
  const workingDirRetryTimerRef = useRef<number | null>(null)
  const manualRootRequestSeqRef = useRef(0)
  const manualRootActiveRequestRef = useRef<number | null>(null)
  const [workingDirError, setWorkingDirError] = useState<string | null>(null)
  const [workingDirRetryEpoch, setWorkingDirRetryEpoch] = useState(0)
  const [rootChangeSettledEpoch, setRootChangeSettledEpoch] = useState(0)
  const rootGenerationRef = useRef(0)
  const rootResolveSeqRef = useRef(0)
  const rootTransitionRef = useRef(false)

  // ── views and panes ──
  const [sideTab, setSideTab] = useState<SideView>('files')
  const [browsePath, setBrowsePath] = useState<string | null>(null)
  const [searchRequest, setSearchRequest] = useState<SearchRequest | null>(null)
  const [goToLineSeq, setGoToLineSeq] = useState(0)
  const [collapsed, setCollapsed] = useState(false)
  const [narrow, setNarrow] = useState(false)
  // A callback ref, not useRef: the page renders a placeholder shell before
  // the workspace frame exists, so an effect that reads a ref once would
  // observe nothing.
  const [frameEl, setFrameEl] = useState<HTMLDivElement | null>(null)

  // ── terminal ──
  const [terminalOpen, setTerminalOpen] = useState(false)
  const [terminalDock, setTerminalDock] = useState<TerminalDock>('bottom')
  const [terminalActive, setTerminalActive] = useState(false)
  const [terminalBottomSize, setTerminalBottomSize] = useState(TERMINAL_BOTTOM_DEFAULT_SIZE)
  const [terminalRightSize, setTerminalRightSize] = useState(TERMINAL_RIGHT_DEFAULT_SIZE)
  const [terminalWorkspace, dispatchTerminalWorkspace] = useReducer(reduceTerminalWorkspace, '/', createTerminalWorkspace)
  const terminalConnectionCoordinators = useRef(new Map<string, TerminalConnectionCoordinator>()).current
  const terminalLeaseStore = useMemo(() => {
    try {
      return window.localStorage
    } catch {
      return null
    }
  }, [])
  const terminalStorageKey = `iii::shell-ui::terminal-leases::${paneKey}`

  // ── workspace data ──
  // Dot entries are filtered by default (Finder/VS Code convention) —
  // in home-shaped folders they otherwise crowd out every visible name.
  const [showHidden, setShowHidden] = useState(false)
  const [git, setGit] = useState<GitState | null>(null)
  const gitRef = useRef(git)
  gitRef.current = git
  const [gitEpoch, setGitEpoch] = useState(0)
  // Bumps whenever the disk (or the index) moved: diff tabs re-read.
  const [diskEpoch, setDiskEpoch] = useState(0)
  const workspaceTree = useWorkspaceTree(host, root, showHidden, rootGenerationRef)
  const tree = workspaceTree.tree
  const applyTreeChanges = workspaceTree.applyChanges
  const ensureDir = workspaceTree.ensureDir
  const [expanded, setExpanded] = useState<string[]>([])
  const expandedRef = useRef(expanded)
  expandedRef.current = expanded
  const [reveal, setReveal] = useState<string | null>(null)

  // ── tabs ──
  const [tabs, setTabs] = useState<TabsState>(EMPTY_TABS)
  const tabsRef = useRef(tabs)
  tabsRef.current = tabs
  const [dirtyPaths, setDirtyPaths] = useState<ReadonlySet<string>>(new Set())
  const [diffOptions, setDiffOptions] = useState<DiffOptions>(DEFAULT_DIFF_OPTIONS)
  const [fileBump, setFileBump] = useState(0)
  // Open file tabs whose file is gone from disk (`missing-files.ts`), and
  // the persisted folder that was gone when the pane came back.
  const [missingPaths, setMissingPaths] = useState<MissingPaths>(NO_MISSING)
  const [missingRoot, setMissingRoot] = useState<string | null>(null)
  const missingRootRef = useRef<string | null>(null)
  const cacheRef = useRef<EditorCache>(new Map())
  const objectUrlsRef = useRef(createObjectUrlRegistry())
  useEffect(() => {
    const registry = objectUrlsRef.current
    return () => registry.releaseAll()
  }, [])
  const diffCacheRef = useRef(new Map<string, DiffCacheEntry>())
  const [diffVersion, setDiffVersion] = useState(0)
  const [revealLineRequest, setRevealLineRequest] = useState<{
    path: string
    line: number
    column?: number
    seq: number
  } | null>(null)
  const historyRef = useRef<NavHistory>(EMPTY_HISTORY)
  const [historyState, setHistoryState] = useState({ back: false, forward: false })
  const navigatingRef = useRef(false)
  const syncHistoryState = useCallback(
    () => setHistoryState({ back: canGoBack(historyRef.current), forward: canGoForward(historyRef.current) }),
    [],
  )

  // ── turns ──
  const [sessionTurns, setSessionTurns] = useState<readonly SessionTurnSummary[]>([])
  const sessionTurnsSeqRef = useRef(0)
  const turnCache = useMemo(() => createTurnCache(host, conversationId), [host, conversationId])
  const [timelineNote, setTimelineNote] = useState<string | null>(null)
  const [reverting, setReverting] = useState<string | null>(null)
  const [pendingDiscard, setPendingDiscard] = useState<GitChange | null>(null)

  const activeTab = activeTabOf(tabs)
  const tabVisible = !terminalActive
  const activeFilePath = tabVisible && activeTab?.target.kind === 'file' ? activeTab.target.path : null
  const activeDiff = tabVisible && activeTab?.target.kind === 'diff' ? activeTab.target : null

  // ── unsaved work ──
  useEffect(() => {
    setDirty?.(dirtyPaths.size === 0 ? false : dirtyPaths.size === 1 ? [...dirtyPaths][0] : true)
  }, [dirtyPaths, setDirty])
  useEffect(() => {
    if (dirtyPaths.size === 0) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener('beforeunload', warn)
    return () => window.removeEventListener('beforeunload', warn)
  }, [dirtyPaths])

  const confirmDiscardAllEdits = useCallback(() => {
    if (dirtyPaths.size === 0) return true
    return window.confirm(`discard unsaved changes in ${dirtyPaths.size} ${dirtyPaths.size === 1 ? 'file' : 'files'}?`)
  }, [dirtyPaths])

  // ── boot: worker info + this workspace tab's persisted state ──
  useEffect(() => {
    let cancelled = false
    coderInfo(host)
      .then((out) => {
        if (!cancelled) setInfo(out)
      })
      .catch((err: unknown) => {
        if (!cancelled) setInfoError(errorMessage(err))
      })
    loadTabUiState(host, paneKey, tabId)
      .then((state) => {
        if (!cancelled) setRestored(state)
      })
      .catch(() => {
        if (!cancelled) setRestored(null)
      })
    return () => {
      cancelled = true
    }
  }, [host, paneKey, tabId])

  // Root resolution waits for BOTH. A folder the user picked in this pane
  // outranks the chat's current one (the chat's next move still re-roots
  // the pane); otherwise the chat's working directory is the live source
  // of truth for a split Shell pane and persisted state only names the
  // folder when there is no chat folder. Any of them may be a subfolder
  // of an allowed base path, not just the base paths themselves.
  useEffect(() => {
    if (!info || restored === 'loading' || root !== null) return
    let cancelled = false
    const seq = ++rootResolveSeqRef.current
    const pinnedRoot = restored?.rootPinned === true && restored.root ? restored.root : null
    const requested = pinnedRoot ?? workingDir ?? restored?.root ?? info.primary_root
    const requestedWorkingDir = workingDir ?? null
    // A persisted folder that could not be opened is reported once the
    // pane has settled somewhere else (the chat's folder reports through
    // its own follow logic, so only a stored one is remembered here).
    const settled = (next: string) => {
      if (missingRootRef.current !== null && missingRootRef.current !== next) setMissingRoot(missingRootRef.current)
      missingRootRef.current = null
    }
    workspaceValidate(host, requested)
      .then(({ path: next }) => {
        if (cancelled || rootResolveSeqRef.current !== seq) return
        settled(next)
        if (requestedWorkingDir !== null) {
          acknowledgedWorkingDirRef.current =
            pinnedRoot !== null
              ? requestedWorkingDir
              : acknowledgeValidatedWorkingDirectory(
                  acknowledgedWorkingDirRef.current,
                  requestedWorkingDir,
                  workingDirRef.current,
                  true,
                )
        }
        setRoot(next)
        setRootPinned(pinnedRoot !== null)
        if (!restored) {
          dispatchTerminalWorkspace({ type: 'workspace-restored', state: createTerminalWorkspace(next) })
          return
        }
        // The pane's own state (view, options, terminal layout) comes back
        // whatever the folder; what was open comes back per folder. Saves
        // older than the per-folder memory hold one folder's tabs at the
        // top level, restored when that is the folder in front.
        rootMemoryRef.current = parseRootMemory(restored.roots)
        const slice =
          recallRoot(rootMemoryRef.current, next) ??
          (restored.root === next || (!restored.root && next === info.primary_root)
            ? { open: restored.open, active: restored.active, expanded: restored.expanded }
            : null)
        if (slice !== null) {
          setTabs(restoreTabs(slice.open, slice.active))
          setExpanded(slice.expanded)
        }
        setShowHidden(restored.showHidden ?? false)
        if (isSideView(restored.sideView)) setSideTab(restored.sideView)
        if (restored.diffOptions) setDiffOptions({ ...DEFAULT_DIFF_OPTIONS, ...restored.diffOptions })
        if (restored.terminalOpen) setTerminalOpen(true)
        if (restored.terminalDock) setTerminalDock(restored.terminalDock)
        if (restored.terminalActive) setTerminalActive(true)
        setTerminalBottomSize(clampTerminalSize(restored.terminalBottomSize, TERMINAL_BOTTOM_DEFAULT_SIZE))
        setTerminalRightSize(clampTerminalSize(restored.terminalRightSize, TERMINAL_RIGHT_DEFAULT_SIZE))
        dispatchTerminalWorkspace({
          type: 'workspace-restored',
          state: restored.terminalWorkspace
            ? normalizeTerminalWorkspace(restored.terminalWorkspace, next)
            : createTerminalWorkspace(next),
        })
      })
      .catch(() => {
        if (cancelled || rootResolveSeqRef.current !== seq) return
        if (pinnedRoot !== null && restored) {
          // The pinned folder is gone: resolve again the ordinary way.
          missingRootRef.current = pinnedRoot
          setRestored({ ...restored, rootPinned: undefined })
          return
        }
        if (requestedWorkingDir === null && requested !== info.primary_root) missingRootRef.current = requested
        settled(info.primary_root)
        setRoot(info.primary_root)
        dispatchTerminalWorkspace({ type: 'workspace-restored', state: createTerminalWorkspace(info.primary_root) })
      })
    return () => {
      cancelled = true
    }
  }, [host, info, restored, root, workingDir])

  // ── git status (gated on the resolved root) ──
  const gitSeqRef = useRef(0)
  const refreshGit = useCallback((): Promise<GitState | null> => {
    if (!root) return Promise.resolve(null)
    const seq = ++gitSeqRef.current
    return gitChanges(host, root)
      .then((state) => {
        if (gitSeqRef.current === seq) {
          setGit(state)
          setGitEpoch((value) => value + 1)
        }
        return state
      })
      .catch((err: unknown) => {
        if (gitSeqRef.current === seq) setGit({ kind: 'error', message: errorMessage(err) })
        return null
      })
  }, [host, root])
  useEffect(() => {
    setGit(null)
    void refreshGit()
  }, [refreshGit])

  // Folders the page expands on its own (restored state, reveals) are
  // listed the same way a click would list them.
  useEffect(() => {
    if (tree === null) return
    for (const dir of expanded) {
      if (tree.kinds.get(dir) === 'dir') void ensureDir(dir)
    }
  }, [expanded, tree, ensureDir])

  const rootLabel = useMemo(() => root?.split('/').filter(Boolean).slice(-1)[0] ?? 'workspace', [root])
  const treeGitStatus = useMemo<readonly GitStatusEntry[]>(
    () => (git?.kind === 'ready' ? git.changes.map((change) => ({ path: change.path, status: change.status })) : []),
    [git],
  )
  const tabGitStatus = useMemo(
    () => new Map(git?.kind === 'ready' ? git.changes.map((change) => [change.path, change.status] as const) : []),
    [git],
  )

  // ── session turns ──
  const refreshSessionTurns = useCallback(() => {
    if (!conversationId) {
      setSessionTurns([])
      return
    }
    const seq = ++sessionTurnsSeqRef.current
    void fetchSessionTurns(host, conversationId)
      .then((turns) => {
        if (sessionTurnsSeqRef.current === seq) setSessionTurns(turns)
      })
      .catch(() => {})
  }, [conversationId, host])
  // biome-ignore lint/correctness/useExhaustiveDependencies: turn boundaries are the refresh triggers
  useEffect(() => {
    refreshSessionTurns()
    if (!harnessTurn.active) return
    const timer = window.setInterval(refreshSessionTurns, 1_500)
    return () => window.clearInterval(timer)
  }, [harnessTurn.active, harnessTurn.completedAtMs, harnessTurn.turnId, refreshSessionTurns])
  // A turn that completed may have become an older turn's "after" side.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the completion stamp is the trigger
  useEffect(() => {
    turnCache.clear()
    setDiskEpoch((value) => value + 1)
  }, [harnessTurn.completedAtMs, turnCache])
  const turnTitles = useMemo(
    () => new Map(sessionTurns.map((turn, index) => [turn.turn_id, turnTitle(turn, sessionTurns.length - index)] as const)),
    [sessionTurns],
  )

  // ── narrow panes ──
  // The panel, not the viewport, decides what fits: a shell page shares the
  // console with other panels. Below the same width the stylesheet treats as
  // narrow, the sidebar becomes an overlay, so it starts out of the way.
  useEffect(() => {
    if (!frameEl) return
    const measure = () => setNarrow(frameEl.getBoundingClientRect().width <= 720)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(frameEl)
    return () => observer.disconnect()
  }, [frameEl])
  const narrowRef = useRef(narrow)
  useEffect(() => {
    if (narrow && !narrowRef.current) setCollapsed(true)
    narrowRef.current = narrow
  }, [narrow])

  // ── tab verbs ──
  const showTab = useCallback((next: (state: TabsState) => TabsState) => {
    setTerminalActive(false)
    setTabs(next)
  }, [])

  const openFileTab = useCallback(
    (relPath: string, options: { pin?: boolean; line?: number; column?: number } = {}) => {
      showTab((s) => (options.pin ? openPinned(s, fileTarget(relPath)) : openPreview(s, fileTarget(relPath))))
      if (options.line !== undefined) {
        const line = options.line
        setRevealLineRequest((previous) => ({
          path: relPath,
          line,
          column: options.column,
          seq: (previous?.seq ?? 0) + 1,
        }))
      }
    },
    [showTab],
  )

  const openDiffTab = useCallback(
    (relPath: string, source: DiffSource, pin = false) => {
      showTab((s) => (pin ? openPinned(s, diffTarget(relPath, source)) : openPreview(s, diffTarget(relPath, source))))
    },
    [showTab],
  )

  const activateTabId = useCallback((id: string) => showTab((s) => activateTab(s, id)), [showTab])
  const pinTabId = useCallback((id: string) => setTabs((s) => pinTab(s, id)), [])

  const dropFileCache = useCallback((path: string) => {
    objectUrlsRef.current.release(cacheRef.current.get(path)?.image)
    cacheRef.current.delete(path)
  }, [])

  const closeTabIds = useCallback(
    (ids: readonly string[]) => {
      if (ids.length === 0) return
      const closing = new Set(ids)
      const current = tabsRef.current
      const filePaths = current.tabs
        .filter((tab) => closing.has(tab.id) && tab.target.kind === 'file')
        .map((tab) => tab.target.path)
      const dirty = filePaths.filter((path) => dirtyPaths.has(path))
      if (
        dirty.length > 0 &&
        !window.confirm(
          dirty.length === 1 ? `discard unsaved changes to ${dirty[0]}?` : `discard unsaved changes in ${dirty.length} files?`,
        )
      ) {
        return
      }
      for (const path of filePaths) dropFileCache(path)
      for (const id of ids) {
        diffCacheRef.current.delete(id)
        historyRef.current = forgetPath(historyRef.current, id)
      }
      if (filePaths.length > 0) {
        setDirtyPaths((prev) => {
          const next = new Set(prev)
          for (const path of filePaths) next.delete(path)
          return next.size === prev.size ? prev : next
        })
      }
      setTabs((s) => {
        let next = s
        for (const id of ids) next = closeTab(next, id)
        return next
      })
      syncHistoryState()
    },
    [dirtyPaths, dropFileCache, syncHistoryState],
  )
  const closeTabId = useCallback((id: string) => closeTabIds([id]), [closeTabIds])

  // ── navigation history (over tab ids) ──
  useEffect(() => {
    if (!tabVisible || tabs.active === null) return
    if (navigatingRef.current) {
      navigatingRef.current = false
      return
    }
    historyRef.current = pushLocation(historyRef.current, { path: tabs.active })
    syncHistoryState()
  }, [tabs.active, tabVisible, syncHistoryState])
  const navigate = useCallback(
    (direction: -1 | 1) => {
      const step = direction === -1 ? goBack(historyRef.current) : goForward(historyRef.current)
      if (step.location === null) return
      const id = step.location.path
      if (!findTab(tabsRef.current, id)) {
        // The tab is gone: skip over it.
        historyRef.current = forgetPath(step.history, id)
        syncHistoryState()
        return
      }
      historyRef.current = step.history
      syncHistoryState()
      navigatingRef.current = true
      activateTabId(id)
    },
    [activateTabId, syncHistoryState],
  )

  // ── explorer verbs ──
  const revealFolder = useCallback(
    (relPath: string) => {
      setSideTab('files')
      setCollapsed(false)
      void workspaceTree.ensurePath(relPath).finally(() => setReveal(relPath))
    },
    [workspaceTree],
  )
  const onRevealed = useCallback(() => setReveal(null), [])

  const onDirtyChange = useCallback((relPath: string, dirty: boolean) => {
    setDirtyPaths((prev) => {
      if (prev.has(relPath) === dirty) return prev
      const next = new Set(prev)
      if (dirty) next.add(relPath)
      else next.delete(relPath)
      return next
    })
    // Editing a preview tab pins it — replacing it would drop the edits.
    if (dirty) setTabs((s) => pinTab(s, fileTabId(relPath)))
  }, [])

  const openTerminalAt = useCallback(
    (dir: string) => {
      const currentRoot = rootRef.current
      if (!currentRoot) return
      const stamp = `${Date.now().toString(36)}`
      dispatchTerminalWorkspace({
        type: 'tab-created',
        tabId: `tab-dir-${stamp}`,
        paneId: `pane-dir-${stamp}`,
        root: joinPath(currentRoot, dir),
      })
      setTerminalOpen(true)
      if (terminalDock === 'editor') setTerminalActive(true)
    },
    [terminalDock],
  )

  const findInFolder = useCallback((dir: string) => {
    setSideTab('search')
    setCollapsed(false)
    setSearchRequest((previous) => ({ seq: (previous?.seq ?? 0) + 1, includeGlob: dir === '' ? '' : `${dir}/**` }))
  }, [])

  const compareFile = useCallback((relPath: string, ref = 'HEAD') => openDiffTab(relPath, { type: 'compare', ref }, true), [openDiffTab])

  const afterDiskChange = useCallback(() => {
    void refreshGit()
    setDiskEpoch((value) => value + 1)
  }, [refreshGit])

  // The explorer's verbs. Paths are root-relative; the tree is patched at
  // once so the row reflects the action before the watcher confirms it.
  const explorerActions = useMemo<ExplorerActions>(
    () => ({
      create: async (kind, rel) => {
        const currentRoot = rootRef.current
        if (!currentRoot) return
        const generation = rootGenerationRef.current
        await createEntry(host, currentRoot, kind, rel)
        // A root switch during the write: the entry landed on disk, but the
        // pane now shows another tree — refreshing or opening would talk to
        // the wrong root.
        if (rootGenerationRef.current !== generation || rootRef.current !== currentRoot) return
        applyTreeChanges([{ rel, kind: 'created', dir: kind === 'folder' }])
        afterDiskChange()
        if (kind === 'file') openFileTab(rel, { pin: true })
      },
      rename: async (from, to, isDir) => {
        const currentRoot = rootRef.current
        if (!currentRoot) return
        const generation = rootGenerationRef.current
        await renameEntry(host, currentRoot, from, to)
        if (rootGenerationRef.current !== generation || rootRef.current !== currentRoot) return
        // Open tabs and their drafts follow the file to its new name.
        const affected = tabsRef.current.tabs.filter((tab) => isUnder(tab.target.path, from))
        for (const tab of affected) {
          const renamed = isDir ? to + tab.target.path.slice(from.length) : to
          const target: TabTarget =
            tab.target.kind === 'file' ? fileTarget(renamed) : diffTarget(renamed, tab.target.source)
          if (tab.target.kind === 'file') {
            const oldPath = tab.target.path
            const cached = cacheRef.current.get(oldPath)
            if (cached) {
              cacheRef.current.delete(oldPath)
              cacheRef.current.set(renamed, cached)
            }
            setDirtyPaths((prev) => {
              if (!prev.has(oldPath)) return prev
              const next = new Set(prev)
              next.delete(oldPath)
              next.add(renamed)
              return next
            })
          }
          diffCacheRef.current.delete(tab.id)
          historyRef.current = forgetPath(historyRef.current, tab.id)
          setTabs((s) => {
            const wasActive = s.active === tab.id
            const closed = closeTab(s, tab.id)
            const reopened = tab.pinned ? openPinned(closed, target) : openPreview(closed, target)
            return wasActive ? reopened : { ...reopened, active: s.active }
          })
        }
        syncHistoryState()
        applyTreeChanges([{ rel: from, kind: 'deleted', dir: isDir }])
        if (isDir) await workspaceTree.reloadDir(dirname(to))
        else applyTreeChanges([{ rel: to, kind: 'created', dir: false }])
        afterDiskChange()
      },
      remove: async (rel, isDir) => {
        const currentRoot = rootRef.current
        if (!currentRoot) return
        const generation = rootGenerationRef.current
        await deleteEntry(host, currentRoot, rel, isDir)
        if (rootGenerationRef.current !== generation || rootRef.current !== currentRoot) return
        const affected = tabsRef.current.tabs.filter((tab) => tab.target.kind === 'file' && isUnder(tab.target.path, rel))
        if (affected.length > 0) closeTabIds(affected.map((tab) => tab.id))
        applyTreeChanges([{ rel, kind: 'deleted', dir: isDir }])
        afterDiskChange()
      },
      duplicate: async (rel) => {
        const currentRoot = rootRef.current
        if (!currentRoot) return
        const kinds = tree?.kinds
        const to = duplicateName(rel, (candidate) => kinds?.has(candidate) ?? false)
        await duplicateFile(host, currentRoot, rel, to)
        applyTreeChanges([{ rel: to, kind: 'created', dir: false }])
        afterDiskChange()
        openFileTab(to, { pin: true })
      },
      openTerminal: openTerminalAt,
      copyPath: (rel, absolute) => {
        const currentRoot = rootRef.current
        void copyText(absolute && currentRoot ? joinPath(currentRoot, rel) : rel)
      },
      compare: (rel) => compareFile(rel),
      findInFolder,
      discard: (rel) => {
        const change = gitRef.current?.kind === 'ready' ? gitRef.current.changes.find((c) => c.path === rel) : undefined
        if (change) setPendingDiscard(change)
      },
      refresh: () => {
        workspaceTree.refresh()
        afterDiskChange()
      },
    }),
    [
      host,
      tree,
      applyTreeChanges,
      afterDiskChange,
      openFileTab,
      openTerminalAt,
      compareFile,
      findInFolder,
      workspaceTree,
      closeTabIds,
      syncHistoryState,
    ],
  )

  const runDiscard = useCallback(
    async (change: GitChange) => {
      const currentRoot = rootRef.current
      if (!currentRoot) return
      const results = await gitDiscard(host, currentRoot, [change])
      const failure = results.find((result) => result.error !== null)
      if (failure) setTimelineNote(`discard failed: ${failure.error}`)
      dropFileCache(change.path)
      setFileBump((value) => value + 1)
      afterDiskChange()
    },
    [host, afterDiskChange, dropFileCache],
  )

  // ── source control ──
  const scmActive = sideTab === 'scm'
  const scm = useSourceControl(host, root, gitEpoch, scmActive, afterDiskChange)
  const compareOpen = tabs.tabs.some((tab) => tab.target.kind === 'diff' && tab.target.source.type === 'compare')
  const compareRefs = useCompareRefs(host, root, compareOpen)

  // ── timeline ──
  const afterRevert = useCallback(() => {
    workspaceTree.refresh()
    refreshSessionTurns()
    turnCache.clear()
    setFileBump((value) => value + 1)
    afterDiskChange()
  }, [workspaceTree, refreshSessionTurns, turnCache, afterDiskChange])

  const revertTurnFiles = useCallback(
    async (turnId: string, paths?: readonly string[]) => {
      if (!conversationId || reverting !== null) return
      if (!confirmDiscardAllEdits()) return
      setReverting(turnId)
      setTimelineNote(null)
      try {
        const result = await revertTurn(host, conversationId, turnId, paths)
        setTimelineNote(describeRevert(result))
      } catch (error: unknown) {
        setTimelineNote(`revert failed: ${errorMessage(error)}`)
      } finally {
        setReverting(null)
        // Reverted files may be open: drop their buffers so the reload
        // shows the restored body instead of the draft.
        if (paths === undefined) {
          objectUrlsRef.current.releaseAll()
          cacheRef.current.clear()
        } else {
          for (const path of paths) {
            const rel = relativeToRoot(path, rootRef.current ?? '')
            if (rel !== null) dropFileCache(rel)
          }
        }
        setDirtyPaths(new Set())
        afterRevert()
      }
    },
    [conversationId, reverting, confirmDiscardAllEdits, host, afterRevert, dropFileCache],
  )

  // ── diff tab loading (the active diff only; others keep what they had) ──
  const activeDiffId = activeDiff !== null ? tabIdFor(activeDiff) : null
  useEffect(() => {
    if (activeDiff === null || activeDiffId === null || root === null) return
    const entry = diffCacheRef.current.get(activeDiffId)
    const stale = entry === undefined || (entry.epoch !== diskEpoch && diffSourceFollowsDisk(activeDiff.source))
    if (!stale) return
    const previous = entry?.state
    diffCacheRef.current.set(activeDiffId, {
      epoch: diskEpoch,
      state: previous?.phase === 'ready' ? previous : { phase: 'loading' },
    })
    setDiffVersion((value) => value + 1)
    const generation = rootGenerationRef.current
    const target = activeDiff
    void loadDiffContents(host, root, target.path, target.source, turnCache)
      .then<DiffTabState>((contents) => ({ phase: 'ready', contents }))
      .catch<DiffTabState>((error: unknown) => ({ phase: 'error', message: errorMessage(error) }))
      .then((state) => {
        if (rootGenerationRef.current !== generation || rootRef.current !== root) return
        const current = diffCacheRef.current.get(activeDiffId)
        if (current === undefined || current.epoch !== diskEpoch) return
        diffCacheRef.current.set(activeDiffId, { epoch: diskEpoch, state })
        setDiffVersion((value) => value + 1)
      })
  }, [activeDiff, activeDiffId, root, diskEpoch, host, turnCache])
  // biome-ignore lint/correctness/useExhaustiveDependencies: diffVersion is the cache's change signal
  const activeDiffState: DiffTabState = useMemo(
    () => (activeDiffId !== null ? diffCacheRef.current.get(activeDiffId)?.state : undefined) ?? { phase: 'loading' },
    [activeDiffId, diffVersion],
  )
  const reloadActiveDiff = useCallback(() => {
    if (activeDiffId === null) return
    diffCacheRef.current.delete(activeDiffId)
    setDiskEpoch((value) => value + 1)
  }, [activeDiffId])

  const diffActions = useMemo<DiffTabActions>(() => {
    if (activeDiff === null) return {}
    const path = activeDiff.path
    const source = activeDiff.source
    const openFile = (rel: string, line?: number) => openFileTab(rel, { pin: true, line })
    switch (source.type) {
      case 'staged':
        return { openFile, unstage: () => void scm.unstage([path]) }
      case 'unstaged': {
        const change = gitRef.current?.kind === 'ready' ? gitRef.current.changes.find((c) => c.path === path) : undefined
        return {
          openFile,
          stage: () => void scm.stage([path]),
          discard: change ? () => setPendingDiscard(change) : undefined,
        }
      }
      case 'turn':
        return {
          openFile,
          revert: () => {
            const abs = rootRef.current ? joinPath(rootRef.current, path) : path
            void revertTurnFiles(source.turnId, [abs])
          },
        }
      case 'compare':
        return {
          openFile,
          changeRef: (ref) => {
            const trimmed = ref.trim()
            if (trimmed === '' || trimmed === source.ref) return
            setTabs((s) => {
              const fromId = tabIdFor(activeDiff)
              const pinned = findTab(s, fromId)?.pinned ?? true
              const closed = closeTab(s, fromId)
              const target = diffTarget(path, { type: 'compare', ref: trimmed })
              return pinned ? openPinned(closed, target) : openPreview(closed, target)
            })
          },
        }
      case 'change':
        return { openFile }
    }
  }, [activeDiff, openFileTab, scm, revertTurnFiles])

  // ── live updates: the watched root streams every change here ──
  // The worker runs a system-level watch on the browsed root for this
  // binding (`shell::changed`): agent writes, shell::exec side effects and
  // outside-the-engine edits all land. A burst patches the tree, reloads
  // the active file when it was the one written (a clean buffer follows
  // the disk, a dirty one keeps the user's edits), and refreshes git and
  // the diff tabs. Bursts coalesce worker-side and again briefly here.
  const liveTimerRef = useRef<number | null>(null)
  const changedAbsRef = useRef<Map<string, string>>(new Map())
  const changedDirsRef = useRef<Set<string>>(new Set())

  const reloadActiveFile = useCallback(() => {
    const currentRoot = rootRef.current
    const generation = rootGenerationRef.current
    const active = activeTabOf(tabsRef.current)
    if (!currentRoot || !active || active.target.kind !== 'file') return
    const path = active.target.path
    const cached = cacheRef.current.get(path)
    // An image preview follows the disk through its own render path; a
    // windowed read has no whole body to refresh.
    if (cached?.image || cached?.window) return
    const absPath = joinPath(currentRoot, path)
    if (!changedAbsRef.current.has(absPath)) return
    coderReadFile(host, absPath, { maxOutputBytes: EDITOR_FULL_READ_BUDGET })
      .then((out) => {
        if (rootGenerationRef.current !== generation || rootRef.current !== currentRoot) return
        const entry = cacheRef.current.get(path)
        if (!entry) return
        if (!refreshCleanEditorCacheEntry(entry, out.content ?? '', out.revision ?? undefined)) return
        setFileBump((n) => n + 1)
      })
      .catch(() => {
        // A deleted-then-read race resolves through the next tree refresh.
      })
  }, [host])

  useWorkspaceChanges(host, root, (event) => {
    if (rootTransitionRef.current) return
    if (event.root !== rootRef.current) return
    const eventAbs = joinPath(event.root, event.path)
    changedAbsRef.current.set(eventAbs, event.kind)
    if (event.dir === true) changedDirsRef.current.add(eventAbs)
    if (liveTimerRef.current !== null) return
    const generation = rootGenerationRef.current
    liveTimerRef.current = window.setTimeout(() => {
      liveTimerRef.current = null
      if (rootGenerationRef.current !== generation) return
      reloadActiveFile()
      const changed = changedAbsRef.current
      changedAbsRef.current = new Map()
      const changedDirs = changedDirsRef.current
      changedDirsRef.current = new Set()
      const currentRoot = rootRef.current
      if (currentRoot === null) return
      const prefix = currentRoot.endsWith('/') ? currentRoot : `${currentRoot}/`
      const treeChanges: TreeChange[] = []
      for (const [abs, rawKind] of changed) {
        if (!abs.startsWith(prefix)) continue
        treeChanges.push({ rel: abs.slice(prefix.length), kind: rawKind, dir: changedDirs.has(abs) })
      }
      applyTreeChanges(treeChanges)
      const openFiles = new Set(tabsRef.current.tabs.filter((tab) => tab.target.kind === 'file').map((tab) => tab.target.path))
      setMissingPaths((prev) => missingAfterChanges(prev, treeChanges, openFiles))
      afterDiskChange()
    }, LIVE_COALESCE_MS)
  }, paneScope)

  // ── files gone from disk ──
  // The tabs that came back with the folder are probed once, so a file
  // deleted while the console was away shows as gone before it is opened;
  // a closed tab drops its mark; the editor reports what its own read found.
  useEffect(() => {
    if (root === null) return
    const generation = rootGenerationRef.current
    const paths = tabsRef.current.tabs.filter((tab) => tab.target.kind === 'file').map((tab) => tab.target.path)
    if (paths.length === 0) return
    let cancelled = false
    coderStatFiles(
      host,
      paths.map((rel) => joinPath(root, rel)),
    )
      .then((results) => {
        if (cancelled || rootGenerationRef.current !== generation || rootRef.current !== root) return
        const gone = missingFromStats(results, root)
        if (gone.length > 0) setMissingPaths((prev) => withMissingPaths(prev, gone))
      })
      .catch(() => {
        // The editor's own read reports on the file when it is opened.
      })
    return () => {
      cancelled = true
    }
  }, [host, root])
  useEffect(() => {
    const openFiles = new Set(tabs.tabs.filter((tab) => tab.target.kind === 'file').map((tab) => tab.target.path))
    setMissingPaths((prev) => pruneMissing(prev, openFiles))
  }, [tabs])
  const onFileMissing = useCallback((relPath: string, gone: boolean) => {
    setMissingPaths((prev) => withMissing(prev, relPath, gone))
  }, [])
  useEffect(
    () => () => {
      if (liveTimerRef.current !== null) window.clearTimeout(liveTimerRef.current)
    },
    [],
  )

  // ── persistence: any state change after boot writes (debounced) ──
  const saver = useMemo(() => createTabUiStateSaver(host, paneKey), [host, paneKey])
  useEffect(() => () => saver.dispose(), [saver])
  const bootedRef = useRef(false)
  useEffect(() => {
    if (root === null) return
    if (!bootedRef.current) {
      // The first pass after restore replays state we just loaded.
      bootedRef.current = true
      return
    }
    const slice = { open: persistedTabs(tabs), active: tabs.active, expanded }
    const memory = rememberRoot(rootMemoryRef.current, root, slice)
    rootMemoryRef.current = memory
    saver.save({
      root,
      rootPinned: rootPinned || undefined,
      roots: serializeRootMemory(memory),
      open: slice.open,
      active: slice.active,
      expanded,
      showHidden,
      sideView: sideTab,
      diffOptions,
      terminalOpen,
      terminalDock,
      terminalActive,
      terminalBottomSize,
      terminalRightSize,
      terminalWorkspace,
    })
  }, [
    saver,
    root,
    rootPinned,
    tabs,
    expanded,
    showHidden,
    sideTab,
    diffOptions,
    terminalOpen,
    terminalDock,
    terminalActive,
    terminalBottomSize,
    terminalRightSize,
    terminalWorkspace,
  ])

  // ── root changes ──
  const changeRoot = useCallback(
    (nextRoot: string, onResolved?: (outcome: RootChangeOutcome, path?: string, error?: unknown) => void): boolean => {
      const resolveSeq = ++rootResolveSeqRef.current
      void validateRootTarget(
        () => workspaceValidate(host, nextRoot),
        () => rootResolveSeqRef.current === resolveSeq,
      ).then((result) => {
        if (result.outcome !== 'validated') {
          onResolved?.(result.outcome, undefined, result.outcome === 'failed' ? result.error : undefined)
          if (result.outcome === 'failed') setRootChangeSettledEpoch((epoch) => epoch + 1)
          return
        }
        if (result.path === rootRef.current) {
          workspaceTree.refresh()
          void refreshGit()
          onResolved?.('validated', result.path)
          setRootChangeSettledEpoch((epoch) => epoch + 1)
          return
        }
        // Validation can take long enough for a draft to begin. Confirm at
        // commit time so the validated transition cannot discard newer work.
        if (!confirmDiscardAllEdits()) {
          onResolved?.('declined')
          setRootChangeSettledEpoch((epoch) => epoch + 1)
          return
        }
        if (rootResolveSeqRef.current !== resolveSeq) {
          onResolved?.('superseded')
          return
        }
        const path = result.path
        rootTransitionRef.current = true
        rootGenerationRef.current += 1
        gitSeqRef.current += 1
        if (liveTimerRef.current !== null) window.clearTimeout(liveTimerRef.current)
        liveTimerRef.current = null
        changedAbsRef.current = new Map()
        changedDirsRef.current = new Set()
        objectUrlsRef.current.releaseAll()
        cacheRef.current.clear()
        diffCacheRef.current.clear()
        historyRef.current = EMPTY_HISTORY
        setHistoryState({ back: false, forward: false })
        // The folder being left keeps what was open in it; the one being
        // entered gets back what it had.
        const previousRoot = rootRef.current
        if (previousRoot !== null) {
          rootMemoryRef.current = rememberRoot(rootMemoryRef.current, previousRoot, {
            open: persistedTabs(tabsRef.current),
            active: tabsRef.current.active,
            expanded: expandedRef.current,
          })
        }
        const recalled = recallRoot(rootMemoryRef.current, path)
        setDirtyPaths(new Set())
        setMissingPaths(NO_MISSING)
        setTabs(recalled !== null ? restoreTabs(recalled.open, recalled.active) : EMPTY_TABS)
        setExpanded(recalled?.expanded ?? [])
        setBrowsePath(null)
        setGit(null)
        // Keep event filtering coherent until React renders the new root.
        rootRef.current = path
        setRoot(path)
        rootTransitionRef.current = false
        onResolved?.('validated', path)
        setRootChangeSettledEpoch((epoch) => epoch + 1)
      })
      return true
    },
    [confirmDiscardAllEdits, host, refreshGit, workspaceTree],
  )

  // ── follow the chat's working directory ──
  // Picking another folder in chat re-roots the explorer, and a folder
  // picked here is handed to the chat (`changeManualRoot`), so a pane beside
  // a chat and that chat stay on one folder. When the chat cannot take the
  // pick (no chat beside, or one that is not mounted), the pick still
  // sticks here until the chat's folder moves again.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the retry epoch re-arms the follow
  useEffect(() => {
    const next = workingDir ?? null
    if (next === null) {
      acknowledgedWorkingDirRef.current = null
      workingDirFollowPendingRef.current = null
      workingDirRetryRef.current = { path: null, failures: 0 }
      setWorkingDirError(null)
      if (workingDirRetryTimerRef.current !== null) {
        window.clearTimeout(workingDirRetryTimerRef.current)
        workingDirRetryTimerRef.current = null
      }
      return
    }
    if (workingDirRetryRef.current.path !== next) {
      workingDirRetryRef.current = { path: next, failures: 0 }
      setWorkingDirError(null)
      if (workingDirRetryTimerRef.current !== null) {
        window.clearTimeout(workingDirRetryTimerRef.current)
        workingDirRetryTimerRef.current = null
      }
    }
    if (
      root === null ||
      manualRootActiveRequestRef.current !== null ||
      !workingDirectoryNeedsFollow(next, acknowledgedWorkingDirRef.current) ||
      workingDirFollowPendingRef.current?.path === next
    ) {
      return
    }
    const request = ++workingDirFollowRequestSeqRef.current
    workingDirFollowPendingRef.current = { path: next, request }
    setPendingRoot(next)
    const accepted = changeRoot(next, (outcome, _path, error) => {
      if (workingDirFollowPendingRef.current?.request !== request) return
      workingDirFollowPendingRef.current = null
      setPendingRoot(null)
      if (outcome === 'validated') {
        setRootPinned(false)
        acknowledgedWorkingDirRef.current = acknowledgeValidatedWorkingDirectory(
          acknowledgedWorkingDirRef.current,
          next,
          workingDirRef.current,
          true,
        )
        workingDirRetryRef.current = { path: next, failures: 0 }
        setWorkingDirError(null)
      } else if (outcome === 'failed' && workingDirRef.current === next) {
        const retry = workingDirRetryRef.current
        const delay = workingDirectoryFollowRetryDelay(retry.failures, error)
        retry.failures += 1
        if (delay !== null && workingDirRetryTimerRef.current === null) {
          workingDirRetryTimerRef.current = window.setTimeout(() => {
            workingDirRetryTimerRef.current = null
            setWorkingDirRetryEpoch((epoch) => epoch + 1)
          }, delay)
        } else if (delay === null) {
          acknowledgedWorkingDirRef.current = acknowledgeUnavailableWorkingDirectory(
            acknowledgedWorkingDirRef.current,
            next,
            workingDirRef.current,
          )
          setWorkingDirError(workingDirectoryRetryMessage(next, 'failed', delay))
        }
      } else if (outcome === 'declined' && workingDirRef.current === next) {
        setWorkingDirError(workingDirectoryRetryMessage(next, 'declined', null))
      }
    })
    if (!accepted && workingDirFollowPendingRef.current?.request === request) {
      workingDirFollowPendingRef.current = null
      setPendingRoot(null)
    }
  }, [workingDir, root, changeRoot, workingDirRetryEpoch])
  useEffect(
    () => () => {
      if (workingDirRetryTimerRef.current !== null) window.clearTimeout(workingDirRetryTimerRef.current)
    },
    [],
  )

  const changeManualRoot = useCallback(
    (nextRoot: string) => {
      const chatDir = workingDirRef.current
      // Suppress both a scheduled retry and an in-flight chat result before the
      // manual validation starts. changeRoot's new sequence supersedes the latter.
      const request = ++manualRootRequestSeqRef.current
      manualRootActiveRequestRef.current = request
      workingDirFollowPendingRef.current = null
      setWorkingDirError(null)
      if (workingDirRetryTimerRef.current !== null) {
        window.clearTimeout(workingDirRetryTimerRef.current)
        workingDirRetryTimerRef.current = null
      }
      setPendingRoot(nextRoot)
      const accepted = changeRoot(nextRoot, (outcome, validatedPath) => {
        if (!ownsRequestToken(manualRootActiveRequestRef.current, request)) return
        manualRootActiveRequestRef.current = null
        setPendingRoot(null)
        if (outcome === 'validated') setRootPinned(true)
        if (outcome === 'validated' && workingDirRef.current === chatDir) {
          // The chat follows the pick: hand it the validated folder, and
          // acknowledge it up front so the chat echoing the same folder
          // back does not re-root the pane a second time. A chat that
          // cannot take it (none beside, or not mounted) leaves the pick
          // pinned here, as before.
          const handedToChat =
            validatedPath !== undefined &&
            validatedPath !== chatDir &&
            conversationId !== null &&
            conversationId !== undefined &&
            ((host as WorkingDirectoryHost).chat?.requestWorkingDirectoryChange?.({
              sessionId: conversationId,
              path: validatedPath,
            }) ??
              false)
          const settled = handedToChat && validatedPath !== undefined ? validatedPath : chatDir
          acknowledgedWorkingDirRef.current = settled
          workingDirRetryRef.current = { path: settled, failures: 0 }
        } else {
          // Validation failure or a declined discard releases the unchanged chat
          // directory to follow again.
          setWorkingDirRetryEpoch((epoch) => epoch + 1)
        }
      })
      if (accepted) return
      setPendingRoot(null)
      if (ownsRequestToken(manualRootActiveRequestRef.current, request)) {
        manualRootActiveRequestRef.current = null
        setWorkingDirRetryEpoch((epoch) => epoch + 1)
      }
    },
    [changeRoot, conversationId, host],
  )

  // ── deep link: #/ext/shell/open/<encoded-abs>[:line] ──
  // The chat's "open in shell" lands here. The request is captured (and
  // stripped from the URL) immediately, then applied once the root has
  // resolved — re-rooting to the file's own folder when it lives outside
  // the browsed one; the effect refires on the new root and opens it.
  const pendingOpenRef = useRef<{ abs: string; line?: number } | null>(null)
  const pendingOpenCaptureSeqRef = useRef(0)
  const pendingOpenRequestSeqRef = useRef(0)
  const pendingOpenRootRequestRef = useRef<{ target: string; token: ScopedRequestToken } | null>(null)
  const pendingOpenWaitingForRetryRef = useRef(false)
  const pendingOpenRetryRef = useRef(0)
  const pendingOpenRetryTimerRef = useRef<number | null>(null)
  const [pendingOpenError, setPendingOpenError] = useState<string | null>(null)
  const [openBump, setOpenBump] = useState(0)
  const requestOpen = useCallback((abs: string, line?: number) => {
    if (rootRef.current !== null) rootResolveSeqRef.current += 1
    pendingOpenCaptureSeqRef.current += 1
    pendingOpenRef.current = { abs, line }
    pendingOpenRootRequestRef.current = null
    pendingOpenWaitingForRetryRef.current = false
    pendingOpenRetryRef.current = 0
    setPendingOpenError(null)
    if (pendingOpenRetryTimerRef.current !== null) {
      window.clearTimeout(pendingOpenRetryTimerRef.current)
      pendingOpenRetryTimerRef.current = null
    }
    setOpenBump((n) => n + 1)
  }, [])
  useEffect(() => {
    const capture = () => {
      const m = window.location.hash.match(/^#\/ext\/shell\/open\/([^/]+)/)
      if (m === null) return
      window.history.replaceState(window.history.state, '', `${window.location.pathname}${window.location.search}#/ext/shell`)
      const raw = m[1]
      const colon = raw.lastIndexOf(':')
      const hasLine = colon !== -1 && /^\d+$/.test(raw.slice(colon + 1))
      const encoded = hasLine ? raw.slice(0, colon) : raw
      let abs: string
      try {
        abs = decodeURIComponent(encoded)
      } catch {
        return // malformed percent escape — not our link
      }
      if (!abs.startsWith('/')) return
      requestOpen(abs, hasLine ? Number.parseInt(raw.slice(colon + 1), 10) : undefined)
    }
    capture()
    window.addEventListener('hashchange', capture)
    return () => window.removeEventListener('hashchange', capture)
  }, [requestOpen])
  // biome-ignore lint/correctness/useExhaustiveDependencies: openBump and rootChangeSettledEpoch re-run the pending open
  useEffect(() => {
    const pending = pendingOpenRef.current
    if (pending === null || root === null) return
    const prefix = root.endsWith('/') ? root : `${root}/`
    if (pending.abs.startsWith(prefix)) {
      pendingOpenCaptureSeqRef.current += 1
      pendingOpenRef.current = null
      pendingOpenRootRequestRef.current = null
      pendingOpenWaitingForRetryRef.current = false
      pendingOpenRetryRef.current = 0
      setPendingOpenError(null)
      openFileTab(pending.abs.slice(prefix.length), { pin: true, line: pending.line })
    } else if (pending.abs !== root) {
      const target = deepLinkRootTarget(pending.abs, workingDirRef.current)
      if (pendingOpenWaitingForRetryRef.current || pendingOpenRootRequestRef.current?.target === target) return
      const requestToken: ScopedRequestToken = {
        scope: pendingOpenCaptureSeqRef.current,
        request: ++pendingOpenRequestSeqRef.current,
      }
      pendingOpenRootRequestRef.current = { target, token: requestToken }
      const accepted = changeRoot(target, (outcome, validatedRoot) => {
        if (
          !ownsScopedRequestToken(pendingOpenCaptureSeqRef.current, pendingOpenRootRequestRef.current?.token ?? null, requestToken)
        ) {
          return
        }
        pendingOpenRootRequestRef.current = null
        if (outcome === 'validated') {
          pendingOpenWaitingForRetryRef.current = false
          if (validatedRoot !== undefined && pendingOpenRef.current) {
            pendingOpenRef.current = {
              ...pendingOpenRef.current,
              abs: rebasePathAfterValidation(pending.abs, target, validatedRoot),
            }
          }
          pendingOpenRetryRef.current = 0
          setPendingOpenError(null)
          return
        }
        if (outcome === 'failed') {
          const delay = rootValidationRetryDelay(pendingOpenRetryRef.current)
          pendingOpenRetryRef.current += 1
          if (delay !== null && pendingOpenRetryTimerRef.current === null) {
            pendingOpenWaitingForRetryRef.current = true
            pendingOpenRetryTimerRef.current = window.setTimeout(() => {
              if (pendingOpenCaptureSeqRef.current !== requestToken.scope) return
              pendingOpenRetryTimerRef.current = null
              pendingOpenWaitingForRetryRef.current = false
              setOpenBump((bump) => bump + 1)
            }, delay)
          } else if (delay === null) {
            pendingOpenWaitingForRetryRef.current = true
            setPendingOpenError(`could not validate the folder for ${pending.abs}`)
          }
        } else if (outcome === 'declined') {
          pendingOpenWaitingForRetryRef.current = true
          setPendingOpenError(`open paused for ${pending.abs}`)
        }
        // A superseding root request will change root or report its own error;
        // the pending absolute path stays intact and is reconsidered afterward.
      })
      if (
        !accepted &&
        ownsScopedRequestToken(pendingOpenCaptureSeqRef.current, pendingOpenRootRequestRef.current?.token ?? null, requestToken)
      ) {
        pendingOpenRootRequestRef.current = null
      }
    }
  }, [root, openBump, changeRoot, rootChangeSettledEpoch, openFileTab])
  useEffect(
    () => () => {
      if (pendingOpenRetryTimerRef.current !== null) window.clearTimeout(pendingOpenRetryTimerRef.current)
    },
    [],
  )

  // ── panel context from other surfaces ──
  const openContextFile = useCallback(
    (path: string): boolean => {
      if (root === null) return false
      setSideTab('files')
      setCollapsed(false)
      if (!path.startsWith('/')) {
        openFileTab(path, { pin: true })
        return true
      }
      // Reuse the validated deep-link pipeline for contextual panel
      // requests. It safely re-roots when the file lives outside the current
      // workspace and preserves the same retry/error behavior.
      requestOpen(path)
      return true
    },
    [root, openFileTab, requestOpen],
  )

  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    const context = parseShellPanelContext(panelContext.context)
    if (!context) return
    // A newly mounted shell page receives the context before its persisted
    // root necessarily resolves. Leave file events unapplied until the safe
    // open pipeline can accept them.
    if (context.type === 'file') {
      if (!openContextFile(context.path)) return
      appliedContextRef.current = panelContext.id
      return
    }
    if (context.type === 'agent-terminal') {
      // The agent's own CLI is the interface; shell just provides the terminal
      // it wants. An existing terminal keeps the directory it was opened in,
      // so this opens a NEW tab rooted where the run worked.
      appliedContextRef.current = panelContext.id
      changeRoot(context.cwd)
      const stamp = `${Date.now().toString(36)}`
      dispatchTerminalWorkspace({ type: 'tab-created', tabId: `tab-agent-${stamp}`, paneId: `pane-agent-${stamp}`, root: context.cwd })
      setTerminalOpen(true)
      setTerminalActive(true)
      return
    }
    if (root === null) return
    appliedContextRef.current = panelContext.id
    const rel = context.path.startsWith('/') ? (relativeToRoot(context.path, root) ?? context.path) : context.path
    openDiffTab(rel, { type: 'change', changeId: context.changeId }, true)
  }, [panelContext, openContextFile, changeRoot, openDiffTab, root])

  // ── chat footer summary: the newest turn ──
  const newestTurn = sessionTurns[0] ?? null
  const summaryFiles = useTurnSummary(host, root, newestTurn, turnCache, diskEpoch)
  useShellReviewSummaryBridge({
    sessionId: conversationId,
    sourceId: paneKey,
    turnId: newestTurn?.turn_id ?? null,
    files: summaryFiles,
    onSelectFile: (path) => {
      if (newestTurn) openDiffTab(path, { type: 'turn', turnId: newestTurn.turn_id }, true)
    },
  })

  // ── terminal verbs ──
  const changeTerminalDock = useCallback((next: TerminalDock) => {
    setTerminalOpen(true)
    setTerminalDock(next)
    setTerminalActive(next === 'editor')
  }, [])
  const closeTerminal = useCallback(() => {
    setTerminalOpen(false)
    setTerminalActive(false)
  }, [])
  const toggleTerminal = useCallback(() => {
    if (!terminalOpen) {
      setTerminalOpen(true)
      setTerminalActive(terminalDock === 'editor')
      return
    }
    if (terminalDock === 'editor' && !terminalActive) {
      setTerminalActive(true)
      return
    }
    closeTerminal()
  }, [closeTerminal, terminalActive, terminalDock, terminalOpen])

  // Step through the changes the active view lists: source control rows
  // when that view is up, else the newest turn's files.
  const stepChange = useCallback(
    (delta: 1 | -1) => {
      const entries: { path: string; source: DiffSource }[] =
        sideTab === 'scm'
          ? [
              ...scm.unstaged.map((entry) => ({ path: entry.path, source: { type: 'unstaged' } as DiffSource })),
              ...scm.staged.map((entry) => ({ path: entry.path, source: { type: 'staged' } as DiffSource })),
            ]
          : newestTurn
            ? newestTurn.files
                .map((file) => relativeToRoot(file.path, rootRef.current ?? ''))
                .filter((rel): rel is string => rel !== null)
                .map((rel) => ({ path: rel, source: { type: 'turn', turnId: newestTurn.turn_id } as DiffSource }))
            : []
      if (entries.length === 0) return
      const current = activeTabOf(tabsRef.current)
      const index =
        current?.target.kind === 'diff'
          ? entries.findIndex((entry) => tabIdFor(diffTarget(entry.path, entry.source)) === current.id)
          : -1
      const start = delta === 1 ? 0 : entries.length - 1
      const next = entries[index === -1 ? start : (index + delta + entries.length) % entries.length]
      openDiffTab(next.path, next.source)
    },
    [sideTab, scm.unstaged, scm.staged, newestTurn, openDiffTab],
  )

  // The page's verbs, for the palette and for the keyboard while this pane
  // has the focus. The keys stay clear of the console's own.
  useEffect(
    () =>
      commands?.register([
        {
          id: 'open-file',
          title: 'Open file…',
          detail: 'Find a file by name',
          keywords: ['quick open', 'go to file', 'path'],
          shortcut: 'P',
          run: () => host.palette?.open({ query: '#' }),
        },
        {
          id: 'search',
          title: 'Search in files',
          detail: 'Find text across the working directory',
          keywords: ['grep', 'find', 'text'],
          shortcut: 'F',
          run: () => {
            setSideTab('search')
            setCollapsed(false)
            window.requestAnimationFrame(() => {
              frameEl?.querySelector<HTMLElement>('[data-shell-search-input]')?.focus()
            })
          },
        },
        {
          id: 'files',
          title: 'Show the explorer',
          detail: 'The file tree sidebar',
          keywords: ['explorer', 'tree', 'sidebar', 'files'],
          shortcut: 'E',
          run: () => {
            setSideTab('files')
            setCollapsed(false)
          },
        },
        {
          id: 'source-control',
          title: 'Show source control',
          detail: 'Staged and unstaged changes',
          keywords: ['git', 'scm', 'staged', 'commit', 'changes'],
          shortcut: 'S',
          run: () => {
            setSideTab('scm')
            setCollapsed(false)
          },
        },
        {
          id: 'timeline',
          title: 'Show the timeline',
          detail: 'Every turn of this chat and what it changed',
          keywords: ['history', 'turns', 'rollback', 'revert'],
          shortcut: 'H',
          run: () => {
            setSideTab('timeline')
            setCollapsed(false)
          },
        },
        {
          id: 'toggle-sidebar',
          title: 'Toggle the sidebar',
          detail: 'Hide or show the sidebar',
          keywords: ['collapse', 'explorer'],
          shortcut: 'B',
          run: () => setCollapsed((current) => !current),
        },
        {
          id: 'toggle-terminal',
          title: 'Toggle the terminal',
          detail: 'Open or close the terminal for this directory',
          keywords: ['pty', 'console', 'command line'],
          shortcut: '`',
          run: toggleTerminal,
        },
        {
          id: 'next-tab',
          title: 'Next tab',
          keywords: ['tab', 'file', 'cycle'],
          shortcut: 'Alt+ArrowRight',
          enabled: () => tabsRef.current.tabs.length > 1,
          run: () => showTab((state) => cycleTab(state, 1)),
        },
        {
          id: 'previous-tab',
          title: 'Previous tab',
          keywords: ['tab', 'file', 'cycle'],
          shortcut: 'Alt+ArrowLeft',
          enabled: () => tabsRef.current.tabs.length > 1,
          run: () => showTab((state) => cycleTab(state, -1)),
        },
        {
          id: 'close-tab',
          title: 'Close the tab',
          keywords: ['tab', 'file', 'close'],
          shortcut: 'W',
          enabled: () => tabsRef.current.active !== null,
          run: () => {
            const active = tabsRef.current.active
            if (active !== null) closeTabId(active)
          },
        },
        {
          id: 'reveal-active',
          title: 'Reveal the active file in the explorer',
          keywords: ['explorer', 'tree', 'locate'],
          shortcut: 'R',
          enabled: () => tabsRef.current.active !== null,
          run: () => {
            const active = activeTabOf(tabsRef.current)
            if (active) revealFolder(active.target.path)
          },
        },
        {
          id: 'go-to-line',
          title: 'Go to line…',
          keywords: ['line', 'jump'],
          shortcut: 'L',
          enabled: () => activeTabOf(tabsRef.current)?.target.kind === 'file',
          run: () => setGoToLineSeq((value) => value + 1),
        },
        {
          id: 'nav-back',
          title: 'Go back',
          detail: 'The previously opened tab',
          keywords: ['history', 'navigate', 'previous'],
          shortcut: 'Shift+Alt+ArrowLeft',
          enabled: () => canGoBack(historyRef.current),
          run: () => navigate(-1),
        },
        {
          id: 'nav-forward',
          title: 'Go forward',
          keywords: ['history', 'navigate', 'next'],
          shortcut: 'Shift+Alt+ArrowRight',
          enabled: () => canGoForward(historyRef.current),
          run: () => navigate(1),
        },
        {
          id: 'next-change',
          title: 'Next change',
          detail: 'Open the next changed file as a diff',
          keywords: ['diff', 'change', 'git', 'turn'],
          shortcut: 'J',
          run: () => stepChange(1),
        },
        {
          id: 'previous-change',
          title: 'Previous change',
          detail: 'Open the previous changed file as a diff',
          keywords: ['diff', 'change', 'git', 'turn'],
          shortcut: 'K',
          run: () => stepChange(-1),
        },
        {
          id: 'compare-active',
          title: 'Compare the active file with…',
          detail: 'A branch, tag or commit',
          keywords: ['diff', 'git', 'revision', 'branch', 'tag'],
          enabled: () => tabsRef.current.active !== null,
          run: () => {
            const active = activeTabOf(tabsRef.current)
            if (active) compareFile(active.target.path)
          },
        },
        {
          id: 'new-file',
          title: 'New file…',
          keywords: ['create', 'explorer'],
          run: () => {
            setSideTab('files')
            setCollapsed(false)
            window.requestAnimationFrame(() => {
              frameEl?.querySelector<HTMLElement>('[aria-label="New file"]')?.click()
            })
          },
        },
        {
          id: 'toggle-hidden',
          title: 'Toggle hidden files',
          keywords: ['dotfiles', 'explorer'],
          run: () => setShowHidden((value) => !value),
        },
        {
          id: 'toggle-word-wrap',
          title: 'Toggle word wrap',
          keywords: ['editor', 'wrap', 'lines', 'diff'],
          shortcut: 'Alt+Z',
          run: () => setDiffOptions((value) => ({ ...value, wordWrap: !value.wordWrap })),
        },
        {
          id: 'revert-last-turn',
          title: 'Revert the last turn',
          detail: 'Put every file the last turn changed back',
          keywords: ['rollback', 'undo', 'turn', 'timeline'],
          enabled: () => !!conversationId && sessionTurns.length > 0,
          run: () => {
            const last = sessionTurns[0]
            if (last) void revertTurnFiles(last.turn_id)
          },
        },
      ]),
    [
      commands,
      host,
      frameEl,
      toggleTerminal,
      showTab,
      closeTabId,
      revealFolder,
      navigate,
      stepChange,
      compareFile,
      conversationId,
      sessionTurns,
      revertTurnFiles,
    ],
  )

  // ── header ──
  // The folder picker is the chat composer's: remembered projects first,
  // a browse to add one, every pick validated by the worker.
  const header = (
    <PageHeader
      className="shui-page-header"
      icon={<SquareTerminal />}
      title="Shell"
      description={
        root ? (
          <DirectoryPicker
            value={pendingRoot ?? root}
            onChange={changeManualRoot}
            defaultDir={info?.primary_root ?? null}
            externalError={workingDirError}
            className="shui-header-root"
          />
        ) : undefined
      }
      actions={
        info && root ? (
          <div className="shui-page-actions">
            <HoverTip label="Go back (Shift+Alt+Left)">
              <button
                type="button"
                className="shui-side-tab"
                onClick={() => navigate(-1)}
                disabled={!historyState.back}
                aria-label="Go back"
              >
                <ArrowLeft aria-hidden className="shui-side-tab-icon" />
              </button>
            </HoverTip>
            <HoverTip label="Go forward (Shift+Alt+Right)">
              <button
                type="button"
                className="shui-side-tab"
                onClick={() => navigate(1)}
                disabled={!historyState.forward}
                aria-label="Go forward"
              >
                <ArrowRight aria-hidden className="shui-side-tab-icon" />
              </button>
            </HoverTip>
            {sideTab === 'files' && !collapsed ? (
              <HoverTip label={showHidden ? 'Hide hidden files (dotfiles)' : 'Show hidden files (dotfiles)'}>
                <button
                  type="button"
                  className={`shui-side-tab${showHidden ? ' active' : ''}`}
                  onClick={() => setShowHidden((value) => !value)}
                  aria-pressed={showHidden}
                  aria-label={showHidden ? 'Hide hidden files' : 'Show hidden files'}
                >
                  {showHidden ? <Eye aria-hidden className="shui-side-tab-icon" /> : <EyeOff aria-hidden className="shui-side-tab-icon" />}
                </button>
              </HoverTip>
            ) : null}
            <HoverTip label={terminalOpen ? 'Hide terminal' : 'Open terminal (zsh)'}>
              <button
                type="button"
                className={`shui-side-tab${terminalOpen ? ' active' : ''}`}
                onClick={toggleTerminal}
                aria-pressed={terminalOpen}
                aria-label={terminalOpen ? 'Hide terminal' : 'Open terminal'}
              >
                <Terminal aria-hidden className="shui-side-tab-icon" />
              </button>
            </HoverTip>
            {narrow ? (
              <HoverTip label={collapsed ? 'Show the sidebar' : 'Hide the sidebar'}>
                <button
                  type="button"
                  className="shui-collapse-btn"
                  onClick={() => setCollapsed((value) => !value)}
                  aria-label={collapsed ? 'Show sidebar' : 'Hide sidebar'}
                >
                  {panelSide === 'right' ? (
                    <PanelRight aria-hidden className="shui-side-tab-icon" />
                  ) : (
                    <PanelLeft aria-hidden className="shui-side-tab-icon" />
                  )}
                </button>
              </HoverTip>
            ) : null}
          </div>
        ) : undefined
      }
      onClose={
        onRequestClose === undefined
          ? undefined
          : () => {
              if (confirmDiscardAllEdits()) onRequestClose()
            }
      }
    />
  )

  if (infoError) {
    return (
      <PageShell>
        {header}
        <div className="shui-side-note warn pad">
          shell explorer needs the worker's coder surface — coder::info failed: {infoError}
        </div>
      </PageShell>
    )
  }
  if (!info || !root) {
    return (
      <PageShell>
        {header}
        <div className="shui-side-note pad">connecting to shell worker…</div>
      </PageShell>
    )
  }

  const activityBadges = {
    scm: git?.kind === 'ready' ? git.changes.length : undefined,
    timeline: sessionTurns.length,
  }
  const terminalInEditor = terminalOpen && terminalDock === 'editor'
  const activeSourceTitle = activeDiff?.source.type === 'turn' ? turnTitles.get(activeDiff.source.turnId) : undefined

  return (
    <PageShell>
      {header}
      <div ref={setFrameEl} className={`shui-workspace-frame terminal-${terminalDock}`}>
        {narrow && !collapsed ? (
          <button type="button" className="shui-sidebar-scrim" aria-label="Hide sidebar" onClick={() => setCollapsed(true)} />
        ) : null}
        <PageBody side={panelSide}>
          <PageSidebar
            label={sideTab === 'files' ? 'Explorer' : sideTab === 'search' ? 'Search' : sideTab === 'scm' ? 'Source control' : 'Timeline'}
            side={panelSide}
            storageKey={`shell:${tabId || 'page'}:sidebar`}
            defaultWidth={SIDEBAR_DEFAULT_WIDTH}
            minWidth={SIDEBAR_MIN_WIDTH}
            maxWidth={SIDEBAR_MAX_WIDTH}
            collapsible
            collapsed={collapsed}
            onCollapsedChange={setCollapsed}
            resizable
            narrow={narrow}
            hidden={narrow && collapsed}
            className="shui-sidebar"
            collapsedActions={
              <ActivityBar
                active={sideTab}
                side={panelSide}
                badges={activityBadges}
                onSelect={(view) => {
                  setSideTab(view)
                  setCollapsed(false)
                }}
              />
            }
          >
            <div className={`shui-side-body side-${panelSide}`}>
              <ActivityBar
                active={sideTab}
                side={panelSide}
                badges={activityBadges}
                onSelect={(view) => {
                  if (view === sideTab && !narrow) {
                    setCollapsed(true)
                    return
                  }
                  setSideTab(view)
                  setCollapsed(false)
                }}
              />
              <div className="shui-side-view">
                {sideTab === 'files' && (pendingRoot !== null || tree === null) ? (
                  <div className="shui-side-note">opening {lastSegments(pendingRoot ?? root ?? '')}…</div>
                ) : sideTab === 'files' ? (
                  <FilesTab
                    tree={tree}
                    gitStatus={treeGitStatus}
                    theme={theme}
                    hiddenFiltered={!showHidden}
                    rootLabel={rootLabel}
                    expanded={expanded}
                    onExpandedChange={setExpanded}
                    onExpandDir={ensureDir}
                    loadingDirs={workspaceTree.loadingDirs}
                    reveal={reveal}
                    onRevealed={onRevealed}
                    activePath={tabVisible ? (activeTab?.target.path ?? null) : null}
                    onActivateFile={(rel) => openFileTab(rel)}
                    onPinFile={(rel) => openFileTab(rel, { pin: true })}
                    actions={explorerActions}
                  />
                ) : sideTab === 'search' ? (
                  <SearchTab
                    host={host}
                    root={root}
                    request={searchRequest}
                    onOpenMatch={(rel, line, column, pin) => openFileTab(rel, { pin, line, column })}
                    onPreviewFile={(rel) => openFileTab(rel)}
                    onPinFile={(rel) => openFileTab(rel, { pin: true })}
                    onRevealFolder={revealFolder}
                  />
                ) : sideTab === 'scm' ? (
                  <SourceControlTab
                    scm={scm}
                    activePath={
                      activeDiff && (activeDiff.source.type === 'staged' || activeDiff.source.type === 'unstaged')
                        ? activeDiff.path
                        : null
                    }
                    activeSide={
                      activeDiff?.source.type === 'staged' ? 'staged' : activeDiff?.source.type === 'unstaged' ? 'unstaged' : null
                    }
                    onOpenChange={(scope, path, pin) => openDiffTab(path, { type: scope }, pin)}
                    onOpenFile={(rel) => openFileTab(rel, { pin: true })}
                  />
                ) : (
                  <TimelineTab
                    turns={sessionTurns}
                    root={root}
                    hasSession={!!conversationId}
                    runningTurnId={harnessTurn.active ? harnessTurn.turnId : null}
                    activeTurnId={activeDiff?.source.type === 'turn' ? activeDiff.source.turnId : null}
                    activePath={activeDiff?.source.type === 'turn' ? activeDiff.path : null}
                    reverting={reverting}
                    note={timelineNote}
                    onRefresh={() => {
                      turnCache.clear()
                      refreshSessionTurns()
                      setDiskEpoch((value) => value + 1)
                    }}
                    onOpenFile={(turnId, rel, pin) => openDiffTab(rel, { type: 'turn', turnId }, pin)}
                    onOpenWorkingFile={(rel) => openFileTab(rel, { pin: true })}
                    onRevertTurn={(turnId) => void revertTurnFiles(turnId)}
                    onRevertFile={(turnId, absPath) => void revertTurnFiles(turnId, [absPath])}
                  />
                )}
              </div>
            </div>
          </PageSidebar>

          <PageMain>
            {workingDirError ? (
              <Panel className="shui-review-notice warn" role="alert">
                <span className="shui-review-notice-icon" aria-hidden="true">
                  <CircleAlert />
                </span>
                <span className="shui-review-notice-copy">
                  <span className="shui-review-notice-title">Working directory unavailable</span>
                  <span className="shui-review-notice-detail">{workingDirError}</span>
                </span>
                <span className="shui-review-notice-actions">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      const next = workingDirRef.current
                      if (next === null) return
                      if (workingDirRetryTimerRef.current !== null) {
                        window.clearTimeout(workingDirRetryTimerRef.current)
                        workingDirRetryTimerRef.current = null
                      }
                      workingDirRetryRef.current = { path: next, failures: 0 }
                      setWorkingDirError(null)
                      setWorkingDirRetryEpoch((epoch) => epoch + 1)
                    }}
                  >
                    <RefreshCw aria-hidden="true" />
                    Retry
                  </Button>
                </span>
              </Panel>
            ) : null}
            {missingRoot ? (
              <Panel className="shui-review-notice warn" role="status">
                <span className="shui-review-notice-icon" aria-hidden="true">
                  <FolderX />
                </span>
                <span className="shui-review-notice-copy">
                  <span className="shui-review-notice-title">The folder you had open is gone</span>
                  <span className="shui-review-notice-detail" title={missingRoot}>
                    {missingRoot} was deleted or moved. Showing {rootLabel} instead.
                  </span>
                </span>
                <span className="shui-review-notice-actions">
                  <Button type="button" variant="ghost" size="sm" onClick={() => setMissingRoot(null)}>
                    Dismiss
                  </Button>
                </span>
              </Panel>
            ) : null}
            {pendingOpenError ? (
              <Panel className="shui-review-notice warn" role="alert">
                <span className="shui-review-notice-icon" aria-hidden="true">
                  <CircleAlert />
                </span>
                <span className="shui-review-notice-copy">
                  <span className="shui-review-notice-title">File could not be opened</span>
                  <span className="shui-review-notice-detail">{pendingOpenError}</span>
                </span>
                <span className="shui-review-notice-actions">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      pendingOpenRetryRef.current = 0
                      pendingOpenWaitingForRetryRef.current = false
                      setPendingOpenError(null)
                      setOpenBump((bump) => bump + 1)
                    }}
                  >
                    <RefreshCw aria-hidden="true" />
                    Retry
                  </Button>
                </span>
              </Panel>
            ) : null}

            {tabs.tabs.length > 0 || terminalInEditor ? (
              <EditorTabs
                tabs={tabs}
                dirtyPaths={dirtyPaths}
                missingPaths={missingPaths}
                tabVisible={tabVisible}
                gitStatus={tabGitStatus}
                turnTitles={turnTitles}
                terminal={
                  terminalInEditor
                    ? {
                        title: terminalWorkspace.tabs.find((tab) => tab.id === terminalWorkspace.activeTabId)?.title ?? 'zsh',
                        active: terminalActive,
                        onActivate: () => setTerminalActive(true),
                        onClose: closeTerminal,
                      }
                    : null
                }
                onActivate={activateTabId}
                onClose={closeTabId}
                onPin={pinTabId}
                onCloseOthers={(id) => closeTabIds(tabs.tabs.filter((tab) => tab.id !== id).map((tab) => tab.id))}
                onCloseRight={(id) => {
                  const index = tabs.tabs.findIndex((tab) => tab.id === id)
                  closeTabIds(tabs.tabs.slice(index + 1).map((tab) => tab.id))
                }}
                onCloseSaved={() =>
                  closeTabIds(
                    tabs.tabs.filter((tab) => tab.target.kind !== 'file' || !dirtyPaths.has(tab.target.path)).map((tab) => tab.id),
                  )
                }
                onCloseAll={() => closeTabIds(tabs.tabs.map((tab) => tab.id))}
                onReveal={revealFolder}
                onCopyPath={explorerActions.copyPath}
                onCompare={(path) => compareFile(path)}
                onOpenFile={(path) => openFileTab(path, { pin: true })}
              />
            ) : null}

            {terminalInEditor && terminalActive ? (
              <TerminalPanel
                state={terminalWorkspace}
                dispatch={dispatchTerminalWorkspace}
                root={root}
                visible
                router={terminalRouter}
                leaseStore={terminalLeaseStore}
                storageKey={terminalStorageKey}
                connectionCoordinators={terminalConnectionCoordinators}
                dock={terminalDock}
                size={terminalBottomSize}
                onDockChange={changeTerminalDock}
                onSizeChange={setTerminalBottomSize}
                onClose={closeTerminal}
              />
            ) : activeFilePath !== null ? (
              <EditorPane
                // fileBump remounts after an agent-side write to the active
                // file: the pane rehydrates from the refreshed cache entry.
                key={`${activeFilePath}:${fileBump}`}
                host={host}
                root={root}
                rootLabel={rootLabel}
                relPath={activeFilePath}
                cache={cacheRef.current}
                createObjectUrl={objectUrlsRef.current.create}
                wordWrap={diffOptions.wordWrap}
                reveal={revealLineRequest?.path === activeFilePath ? revealLineRequest : null}
                goToLineSeq={goToLineSeq}
                onSaved={afterDiskChange}
                onDirtyChange={onDirtyChange}
                onRevealDir={revealFolder}
                onCompare={(path) => compareFile(path)}
                missing={missingPaths.has(activeFilePath)}
                onMissing={onFileMissing}
                onClose={() => closeTabId(fileTabId(activeFilePath))}
              />
            ) : activeDiff !== null ? (
              <DiffTab
                key={activeDiffId ?? 'diff'}
                rootLabel={rootLabel}
                path={activeDiff.path}
                source={activeDiff.source}
                sourceTitle={activeSourceTitle}
                state={activeDiffState}
                options={diffOptions}
                onOptionsChange={setDiffOptions}
                onReload={reloadActiveDiff}
                onRevealDir={revealFolder}
                actions={diffActions}
                compareRefs={activeDiff.source.type === 'compare' ? compareRefs : undefined}
                busy={scm.busy || reverting !== null}
              />
            ) : browsePath !== null ? (
              <WorkspaceBrowser
                tree={tree}
                path={browsePath}
                rootLabel={rootLabel}
                onOpenFolder={(relPath) => {
                  setBrowsePath(relPath)
                  if (relPath !== '') revealFolder(relPath)
                }}
                onOpenFile={(rel) => openFileTab(rel, { pin: true })}
              />
            ) : (
              <ShellLauncher
                root={root}
                pendingRoot={pendingRoot}
                defaultRoot={info?.primary_root ?? null}
                rootError={workingDirError}
                onChangeRoot={changeManualRoot}
                git={git}
                turns={sessionTurns}
                hasSession={!!conversationId}
                turnRunning={harnessTurn.active}
                recent={recentPaths(historyRef.current, 6)}
                onOpenFile={(rel) => openFileTab(rel, { pin: true })}
                onQuickOpen={() => {
                  if (host.palette) host.palette.open({ query: '#' })
                  else {
                    setSideTab('files')
                    setCollapsed(false)
                  }
                }}
                onSearch={() => {
                  setSideTab('search')
                  setCollapsed(false)
                  window.requestAnimationFrame(() => {
                    frameEl?.querySelector<HTMLElement>('[data-shell-search-input]')?.focus()
                  })
                }}
                onOpenChanges={() => {
                  setSideTab('scm')
                  setCollapsed(false)
                }}
                onOpenTimeline={() => {
                  setSideTab('timeline')
                  setCollapsed(false)
                }}
                onOpenTerminal={() => {
                  setTerminalOpen(true)
                  setTerminalActive(true)
                }}
                // Browse opens the workspace browser in this pane and shows
                // the sidebar tree beside it.
                onOpenFiles={() => {
                  setBrowsePath('')
                  setSideTab('files')
                  setCollapsed(false)
                }}
              />
            )}
          </PageMain>
        </PageBody>
        <ConfirmDialog
          open={pendingDiscard !== null}
          onOpenChange={(open) => {
            if (!open) setPendingDiscard(null)
          }}
          title={`Discard changes in ${pendingDiscard ? lastSegments(pendingDiscard.path, 1) : ''}?`}
          description="Working-tree changes are lost; an untracked file is deleted. This cannot be undone."
          details={pendingDiscard ? [pendingDiscard.path] : undefined}
          confirmLabel="Discard"
          onConfirm={() => {
            const change = pendingDiscard
            setPendingDiscard(null)
            if (change) void runDiscard(change)
          }}
          onCancel={() => setPendingDiscard(null)}
        />
        {terminalOpen && terminalDock !== 'editor' ? (
          <TerminalPanel
            state={terminalWorkspace}
            dispatch={dispatchTerminalWorkspace}
            root={root}
            visible
            router={terminalRouter}
            leaseStore={terminalLeaseStore}
            storageKey={terminalStorageKey}
            connectionCoordinators={terminalConnectionCoordinators}
            dock={terminalDock}
            size={terminalDock === 'bottom' ? terminalBottomSize : terminalRightSize}
            onDockChange={changeTerminalDock}
            onSizeChange={terminalDock === 'bottom' ? setTerminalBottomSize : setTerminalRightSize}
            onClose={closeTerminal}
          />
        ) : null}
      </div>
    </PageShell>
  )
}
