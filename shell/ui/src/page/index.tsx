/**
 * The shell explorer page (#/ext/shell) — an IDE-shaped surface over the
 * worker's own functions: a collapsible sidebar (files / git / search,
 * icon tabs) beside a Monaco editor with VS Code-style file tabs
 * (single click previews, double click pins) and a FileDiff pane for
 * git selections.
 *
 * The sidebar hugs the pane's OUTER edge (`panelSide`), and the whole
 * UI state — browsed root, open tabs, expanded folders — persists per
 * workspace tab (`tabId`) in the `shell-ui` configuration entry.
 */

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  type Host,
  IconButton,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Selector,
} from '@iii-dev/console-ui'
import type { GitStatusEntry } from '@pierre/trees'
import {
  Check,
  ChevronsDownUp,
  ChevronsUpDown,
  ClipboardCopy,
  Eye,
  EyeOff,
  FileSearch,
  FileStack,
  FolderTree,
  Image,
  MoreHorizontal,
  PanelLeft,
  PanelRight,
  RefreshCw,
  Search,
  Space,
  SquareTerminal,
  Terminal,
  WholeWord,
  WrapText,
  X,
} from 'lucide-react'
import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react'
import { errorMessage } from '../lib/format'
import {
  captureWorkspaceBaseline,
  classifyWorkspaceBaselinePath,
  type WorkspaceBaselineCoverage,
} from './baseline'
import { ChangeDiffPane } from './ChangeDiffPane'
import {
  type CoderInfo,
  coderInfo,
  coderReadFile,
  coderStatFiles,
  coderTree,
  type FlatTree,
  flattenTree,
  joinPath,
  type TreeNode,
  workspaceValidate,
  coderCreateNewFile,
  shellCreateFolder,
} from './coder'
import {
  type EditorCache,
  type EditorCacheEntry,
  EditorPane,
} from './EditorPane'
import {
  currentReviewDirtyPaths,
  refreshCleanEditorCacheEntry,
} from './editor-cache'
import { FilesTab } from './FilesTab'
import {
  type GitChange,
  type GitCommitSummary,
  type GitComparisonEntry,
  type GitComparisonScope,
  type GitRefSummary,
  type GitRevisionComparisonEntry,
  type GitState,
  gitBranchComparison,
  gitChanges,
  gitCommitComparison,
  gitComparison,
  gitPatch,
  gitRecentCommits,
  gitRefs,
} from './git'
import { HoverTip } from './HoverTip'
import { useWorkspaceChanges } from './live'
import { normalizeLiveReviewEvent } from './live-review'
import { parseShellPanelContext } from './panel-context'
import {
  createTabUiStateSaver,
  loadTabUiState,
  type TabUiState,
  type TerminalDock,
} from './persist'
import {
  createReviewSaveBarrier,
  type ReviewEditDraft,
  type ReviewFileSummary,
  type ReviewOptions,
  loadReviewContents,
  ReviewPane,
  runReviewTransition,
} from './ReviewPane'
import {
  type ReviewScopeCounts,
  ReviewScopePicker,
  type ReviewScopeSelection,
  reviewScopeLabel,
} from './ReviewScopePicker'
import {
  canUseGitMetadataForLiveEntry,
  diffForReviewEntry,
  mergeGitReviewEntries,
  mergeReviewEntry,
  reviewContentsRepresentChange,
  type ReviewEntry,
} from './review'
import {
  DEFAULT_REVIEW_SCOPE,
  EMPTY_TURN_FALLBACK_MS,
  isLiveGitReviewScope,
  isShellUiStatePath,
  LAST_TURN_SCOPE,
  SESSION_SCOPE,
  shouldFallbackToTurnScope,
  shouldEnterTurnScope,
} from './review-scope'
import { useShellReviewSummaryBridge } from './review-summary-store'
import { changedParentDirs, withReviewChanges } from './review-tree'
import { SearchTab } from './SearchTab'
import { ShellLauncher } from './ShellLauncher'
import { TerminalPanel } from './TerminalPanel'
import {
  activateTab,
  basename,
  closeTab,
  cycleTab,
  EMPTY_TABS,
  lastSegments,
  openPinned,
  openPreview,
  pinTab,
  restoreTabs,
  type TabsState,
} from './tabs'
import {
  createTerminalWorkspace,
  normalizeTerminalWorkspace,
  reduceTerminalWorkspace,
} from './terminal-layout'
import type { TerminalOutputRouter } from './terminal-output-router'
import type { TerminalConnectionCoordinator } from './terminal-session-state'
import { useHarnessPreTurn, useHarnessTurn } from './turn'
import {
  canCaptureHarnessWorkspaceChange,
  type HarnessReviewWindow,
} from './turn-status'
import {
  fetchSessionTurn,
  fetchSessionTurns,
  relativeToRoot,
  reviewEntriesFromSession,
  reviewEntriesFromTurn,
  summarizeSessionActivity,
  type SessionTurn,
  type SessionTurnSummary,
  type TurnEntries,
  turnLabel,
} from './turns'
import { WorkspaceBrowser } from './WorkspaceBrowser'
import {
  acknowledgeValidatedWorkingDirectory,
  deepLinkRootTarget,
  ownsRequestToken,
  ownsScopedRequestToken,
  type RootTargetValidation,
  rebasePathAfterValidation,
  rootValidationRetryDelay,
  type ScopedRequestToken,
  validateRootTarget,
  workingDirectoryNeedsFollow,
  workingDirectoryRetryMessage,
} from './working-dir-sync'

type SideTab = 'files' | 'search'

interface DiffSelection {
  /** The change shown — from git status, or synthesized for live
      follows in folders that aren't a repo. */
  change: GitChange
  /** Overrides the git-HEAD baseline: the last content this page saw,
      so modified files outside a repo still diff instead of dumping. */
  /** null means the pre-write text could not be captured. */
  baseline?: string | null
  /** The file's own repo directory when the browsed root sits above it
      (a worktree under the home directory) — see DiffPane. */
  gitDir?: string
}

interface ReviewEditBackup {
  cache: EditorCacheEntry | null
  hadTab: boolean
}

type RootChangeOutcome = RootTargetValidation['outcome'] | 'declined'

type WorkingDirectoryHost = Host & {
  chat?: {
    requestWorkingDirectoryChange?(request: {
      sessionId: string
      path: string
    }): boolean
  }
}

function ReviewOption({
  label,
  icon,
  checked,
  onChange,
}: {
  label: string
  icon: ReactNode
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <DropdownMenuItem
      className="shui-review-option"
      role="menuitemcheckbox"
      aria-checked={checked}
      onSelect={(event) => {
        event.preventDefault()
        onChange(!checked)
      }}
    >
      <span className="menu-icon" aria-hidden>
        {icon}
      </span>
      <span>{label}</span>
      <span className="check" aria-hidden>
        {checked ? <Check /> : null}
      </span>
    </DropdownMenuItem>
  )
}

function SplitDiffIcon() {
  return (
    <svg
      className="shui-diff-style-icon"
      viewBox="0 0 16 16"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="1.5" y="2.5" width="13" height="11" rx="2.5" className="frame" />
      <rect x="3" y="4" width="4.5" height="8" rx="1.2" className="del" />
      <rect x="8.5" y="4" width="4.5" height="8" rx="1.2" className="add" />
    </svg>
  )
}

function UnifiedDiffIcon() {
  return (
    <svg
      className="shui-diff-style-icon"
      viewBox="0 0 16 16"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="1.5" y="2.5" width="13" height="11" rx="2.5" className="frame" />
      <rect x="3" y="4" width="10" height="3.5" rx="1.2" className="del" />
      <rect x="3" y="8.5" width="10" height="3.5" rx="1.2" className="add" />
    </svg>
  )
}

function ReviewMenuAction({
  label,
  icon,
  description,
  disabled,
  onSelect,
}: {
  label: string
  icon: ReactNode
  description?: string
  disabled?: boolean
  onSelect: () => void
}) {
  return (
    <DropdownMenuItem
      className="shui-review-option"
      disabled={disabled}
      onSelect={onSelect}
    >
      <span className="menu-icon" aria-hidden>
        {icon}
      </span>
      <span className="shui-review-option-copy">
        <span>{label}</span>
        {description ? <small>{description}</small> : null}
      </span>
    </DropdownMenuItem>
  )
}

function reviewEntriesFromGit(
  changes: readonly (GitComparisonEntry | GitRevisionComparisonEntry)[],
): ReadonlyMap<string, ReviewEntry> {
  return new Map(
    changes.map((entry) => {
      const change: GitChange = {
        path: entry.path,
        status: entry.status,
        staged: entry.staged,
        ...(entry.from === undefined ? {} : { from: entry.from }),
      }
      return [
        entry.path,
        {
          path: entry.path,
          change,
          before: entry.before,
          after: entry.after,
        },
      ] as const
    }),
  )
}

async function withoutUnreviewableBaselines(
  host: Host,
  root: string,
  entries: ReadonlyMap<string, ReviewEntry>,
  paths?: ReadonlySet<string>,
): Promise<ReadonlyMap<string, ReviewEntry>> {
  const next = new Map(entries)
  const candidates = [...entries].filter(
    ([path, entry]) => entry.baseline === null && (!paths || paths.has(path)),
  )
  await Promise.all(
    candidates.map(async ([path, entry]) => {
      try {
        const contents = await loadReviewContents(host, root, entry)
        if (!reviewContentsRepresentChange(entry, contents)) next.delete(path)
      } catch {
        return
      }
    }),
  )
  return next
}

const SIDEBAR_DEFAULT_WIDTH = 244
const SIDEBAR_MIN_WIDTH = 180
const SIDEBAR_MAX_WIDTH = 560
const TERMINAL_BOTTOM_DEFAULT_SIZE = 280
const TERMINAL_RIGHT_DEFAULT_SIZE = 420
function reviewablePath(rel: string): boolean {
  if (isShellUiStatePath(rel)) return false
  const noise = [
    'Library',
    'node_modules',
    'target',
    'dist',
    'build',
    'out',
    'vendor',
    '__pycache__',
  ]
  const segments = rel.split('/')
  if (segments.some((segment) => segment === '.git' || noise.includes(segment)))
    return false
  return !/\.(o|a|d|rlib|rmeta|so|dylib|dll|class|pyc|wasm|map|log|output|tmp|swp|part|pid|sock)$/.test(
    rel,
  )
}

function clampTerminalSize(size: number | undefined, fallback: number): number {
  if (size === undefined || !Number.isFinite(size)) return fallback
  return Math.min(1200, Math.max(160, Math.round(size)))
}

const SIDE_TABS: { id: SideTab; label: string; Icon: typeof FolderTree }[] = [
  {
    id: 'files',
    label: 'File tree — browse workspace files',
    Icon: FolderTree,
  },
  { id: 'search', label: 'Search — find text in files', Icon: Search },
]

export function ShellExplorerPage({
  host,
  terminalRouter,
  panelSide,
  tabId,
  onRequestClose,
  workingDir,
  panelContext,
  conversationId,
  commands,
}: { host: Host; terminalRouter: TerminalOutputRouter } & PageRenderProps) {
  const theme = host.useTheme()
  const observedReview = useHarnessTurn(host, conversationId)
  const observedReviewKey = observedReview.turnId
  const observedReviewKeyRef = useRef(observedReviewKey)
  observedReviewKeyRef.current = observedReviewKey
  const [reviewKey, setReviewKey] = useState<string | null>(observedReviewKey)
  const [info, setInfo] = useState<CoderInfo | null>(null)
  const [infoError, setInfoError] = useState<string | null>(null)
  const [restored, setRestored] = useState<TabUiState | null | 'loading'>(
    'loading',
  )
  const [root, setRoot] = useState<string | null>(null)
  // The root the picker is validating right now: the select holds this
  // value so the choice never appears to snap back, and the files pane
  // says what it is opening instead of sitting empty.
  const [pendingRoot, setPendingRoot] = useState<string | null>(null)
  const rootRef = useRef(root)
  rootRef.current = root
  const workingDirRef = useRef(workingDir ?? null)
  workingDirRef.current = workingDir ?? null
  const acknowledgedWorkingDirRef = useRef<string | null>(null)
  const workingDirFollowRequestSeqRef = useRef(0)
  const workingDirFollowPendingRef = useRef<{
    path: string
    request: number
  } | null>(null)
  const workingDirRetryRef = useRef({
    path: null as string | null,
    failures: 0,
  })
  const workingDirRetryTimerRef = useRef<number | null>(null)
  const manualRootRequestSeqRef = useRef(0)
  const manualRootActiveRequestRef = useRef<number | null>(null)
  const [workingDirError, setWorkingDirError] = useState<string | null>(null)
  const [workingDirRetryEpoch, setWorkingDirRetryEpoch] = useState(0)
  const [rootChangeSettledEpoch, setRootChangeSettledEpoch] = useState(0)
  const rootGenerationRef = useRef(0)
  const rootResolveSeqRef = useRef(0)
  const rootTransitionRef = useRef(false)
  const [sideTab, setSideTab] = useState<SideTab>('files')
  const [browsePath, setBrowsePath] = useState<string | null>(null)
  const [terminalOpen, setTerminalOpen] = useState(false)
  const [terminalDock, setTerminalDock] = useState<TerminalDock>('bottom')
  const [terminalActive, setTerminalActive] = useState(false)
  const [terminalBottomSize, setTerminalBottomSize] = useState(
    TERMINAL_BOTTOM_DEFAULT_SIZE,
  )
  const [terminalRightSize, setTerminalRightSize] = useState(
    TERMINAL_RIGHT_DEFAULT_SIZE,
  )
  const [terminalWorkspace, dispatchTerminalWorkspace] = useReducer(
    reduceTerminalWorkspace,
    '/',
    createTerminalWorkspace,
  )
  const terminalConnectionCoordinators = useRef(
    new Map<string, TerminalConnectionCoordinator>(),
  ).current
  const terminalLeaseStore = useMemo(() => {
    try {
      return window.localStorage
    } catch {
      return null
    }
  }, [])
  const terminalStorageKey = `iii::shell-ui::terminal-leases::${tabId}`
  const [collapsed, setCollapsed] = useState(false)
  const [narrow, setNarrow] = useState(false)
  // A callback ref, not useRef: the page renders a placeholder shell before
  // the workspace frame exists, so an effect that reads a ref once would
  // observe nothing.
  const [frameEl, setFrameEl] = useState<HTMLDivElement | null>(null)
  const [tree, setTree] = useState<FlatTree | null>(null)
  // Dot entries are filtered by default (Finder/VS Code convention) —
  // in home-shaped folders they otherwise crowd out every visible name.
  const [showHidden, setShowHidden] = useState(false)
  const [git, setGit] = useState<GitState | null>(null)
  // Lazily fetched deep-folder listings, keyed by the folder's rel path.
  // The base tree snapshot is node-budgeted; expanding a folder the
  // snapshot didn't reach fetches its subtree on demand. An entry with
  // no paths marks a fetched-and-empty folder (no refetch loop).
  const [subtrees, setSubtrees] = useState<ReadonlyMap<string, FlatTree>>(
    new Map(),
  )
  const [tabs, setTabs] = useState<TabsState>(EMPTY_TABS)
  const [expanded, setExpanded] = useState<string[]>([])
  const [reveal, setReveal] = useState<string | null>(null)
  const [dirtyPaths, setDirtyPaths] = useState<ReadonlySet<string>>(new Set())
  const [reviewDirtyPaths, setReviewDirtyPaths] = useState<ReadonlySet<string>>(
    new Set(),
  )
  const reviewSaveBarrierRef = useRef<ReturnType<
    typeof createReviewSaveBarrier
  > | null>(null)
  if (reviewSaveBarrierRef.current === null) {
    reviewSaveBarrierRef.current = createReviewSaveBarrier()
  }
  const reviewSaveBarrier = reviewSaveBarrierRef.current
  const [reviewSavingPaths, setReviewSavingPaths] = useState<
    ReadonlySet<string>
  >(new Set())
  const reviewSavePending = reviewSavingPaths.size > 0
  const [diff, setDiff] = useState<DiffSelection | null>(null)
  const diffRequestRef = useRef(0)
  const [reviewRefreshEpoch, setReviewRefreshEpoch] = useState(0)
  const [reviewCollapseEpoch, setReviewCollapseEpoch] = useState(0)
  const [reviewExpandEpoch, setReviewExpandEpoch] = useState(0)
  const [reviewAllCollapsed, setReviewAllCollapsed] = useState(false)
  const toggleAllDiffs = () => {
    if (reviewAllCollapsed) setReviewExpandEpoch((value) => value + 1)
    else setReviewCollapseEpoch((value) => value + 1)
    setReviewAllCollapsed((value) => !value)
  }
  const [reviewMenuOpen, setReviewMenuOpen] = useState(false)
  const [copyingPatch, setCopyingPatch] = useState(false)
  const reloadReview = () => {
    refreshTree()
    void refreshGit()
    if (reviewScope.kind === 'last-turn') {
      setReviewRefreshEpoch((value) => value + 1)
    } else {
      loadReviewScope(reviewScope)
    }
  }
  const copyApplyCommand = async () => {
    if (
      reviewScope.kind === 'last-turn' ||
      reviewScope.kind === 'session' ||
      reviewScope.kind === 'turn' ||
      copyingPatch ||
      root === null
    )
      return
    setCopyingPatch(true)
    try {
      const patch = await gitPatch(host, root, reviewScope)
      await navigator.clipboard.writeText(
        `git apply <<'PATCH'\n${patch}\nPATCH\n`,
      )
    } catch (error: unknown) {
      setScopeError(errorMessage(error))
    } finally {
      setCopyingPatch(false)
    }
  }
  const [reviewScope, setReviewScope] =
    useState<ReviewScopeSelection>(DEFAULT_REVIEW_SCOPE)
  const reviewScopeRef = useRef(reviewScope)
  reviewScopeRef.current = reviewScope
  const followHarnessTurnsRef = useRef(true)
  const emptyTurnFallbackTimerRef = useRef<number | null>(null)
  const [scopeEntries, setScopeEntries] = useState<
    ReadonlyMap<string, ReviewEntry>
  >(new Map())
  const scopeEntriesRef = useRef<ReadonlyMap<string, ReviewEntry>>(scopeEntries)
  scopeEntriesRef.current = scopeEntries
  const [scopeSummary, setScopeSummary] = useState<
    readonly ReviewFileSummary[]
  >([])
  const [scopeLoading, setScopeLoading] = useState(false)
  const [scopeError, setScopeError] = useState<string | null>(null)
  const [scopeCommits, setScopeCommits] = useState<readonly GitCommitSummary[]>(
    [],
  )
  const [scopeRefs, setScopeRefs] = useState<readonly GitRefSummary[]>([])
  const [scopeCounts, setScopeCounts] = useState<ReviewScopeCounts>({})
  const [sessionTurns, setSessionTurns] = useState<
    readonly SessionTurnSummary[]
  >([])
  const [turnOutside, setTurnOutside] = useState(0)
  const [turnOutsideRoot, setTurnOutsideRoot] = useState<string | null>(null)
  const sessionTurnsSeqRef = useRef(0)
  const [scopeMetadataLoading, setScopeMetadataLoading] = useState(false)
  const [scopeMetadataError, setScopeMetadataError] = useState<string | null>(
    null,
  )
  const scopeLoadSeqRef = useRef(0)
  const scopeMetadataSeqRef = useRef(0)
  const [reviewSummary, setReviewSummary] = useState<
    readonly ReviewFileSummary[]
  >([])
  const [reviewOptions, setReviewOptions] = useState<ReviewOptions>({
    diffStyle: 'unified',
    wordWrap: true,
    wordDiffs: true,
    hideWhitespace: false,
    expandUnchanged: false,
    richPreview: false,
  })
  const [reviewEntries, setReviewEntries] = useState<
    ReadonlyMap<string, ReviewEntry>
  >(new Map())
  const reviewEntriesRef =
    useRef<ReadonlyMap<string, ReviewEntry>>(reviewEntries)
  reviewEntriesRef.current = reviewEntries
  // For ordinary non-Git folders, snapshot initial text before Harness
  // writes so every later row can open a real before/after diff.
  const baselineRef = useRef<Map<string, string>>(new Map())
  const baselineKindsRef = useRef<ReadonlyMap<string, TreeNode['kind']>>(
    new Map(),
  )
  const baselineCompleteRef = useRef(false)
  const baselineCapturedRef = useRef(false)
  const baselineReadyRef = useRef<Promise<void>>(Promise.resolve())
  // A capped snapshot degrades quietly per row, so the toolbar says so once.
  const [baselineCoverage, setBaselineCoverage] =
    useState<WorkspaceBaselineCoverage | null>(null)
  const preparedTurnRef = useRef<string | null>(null)
  const lastReviewKeyRef = useRef<string | null>(observedReviewKey ?? null)
  const reviewEpochRef = useRef(0)
  const reviewWindowRef = useRef<HarnessReviewWindow>({
    turnId: observedReviewKey,
    epoch: reviewEpochRef.current,
    active: observedReview.active,
    completedAtMs: observedReview.completedAtMs,
  })
  if (observedReviewKey === lastReviewKeyRef.current) {
    reviewWindowRef.current = {
      turnId: observedReviewKey,
      epoch: reviewEpochRef.current,
      active: observedReview.active,
      completedAtMs: observedReview.completedAtMs,
    }
  }
  const [contextDiff, setContextDiff] = useState<{
    eventId: number
    changeId: string
    path: string
    canViewFile: boolean
  } | null>(null)
  const cacheRef = useRef<EditorCache>(new Map())
  const reviewEditBackupsRef = useRef<Map<string, ReviewEditBackup>>(new Map())
  const reviewDirtyPathsRef = useRef(reviewDirtyPaths)
  reviewDirtyPathsRef.current = reviewDirtyPaths

  const restoreReviewEditCaches = useCallback((paths: Iterable<string>) => {
    const restored = new Set<string>()
    const autoOpenedTabs = new Set<string>()
    for (const path of paths) {
      const backup = reviewEditBackupsRef.current.get(path)
      if (backup === undefined) continue
      reviewEditBackupsRef.current.delete(path)
      if (backup.cache === null) cacheRef.current.delete(path)
      else cacheRef.current.set(path, { ...backup.cache })
      if (!backup.hadTab) autoOpenedTabs.add(path)
      restored.add(path)
    }
    if (restored.size === 0) return
    setDirtyPaths((previous) => {
      const next = new Set(previous)
      for (const path of restored) {
        const cached = cacheRef.current.get(path)
        if (cached !== undefined && cached.draft !== cached.savedContent)
          next.add(path)
        else next.delete(path)
      }
      return next
    })
    if (autoOpenedTabs.size > 0) {
      setTabs((previous) => {
        let next = previous
        for (const path of autoOpenedTabs) next = closeTab(next, path)
        return next
      })
    }
  }, [])

  const confirmDiscardReviewEdits = useCallback(() => {
    if (!reviewSaveBarrier.canTransition()) return false
    const editPaths = [...reviewEditBackupsRef.current.keys()]
    const dirtyReviewPaths = currentReviewDirtyPaths(
      editPaths,
      cacheRef.current,
      reviewDirtyPaths,
    )
    if (dirtyReviewPaths.size === 0) {
      restoreReviewEditCaches(editPaths)
      return true
    }
    const label =
      dirtyReviewPaths.size === 1
        ? [...dirtyReviewPaths][0]
        : `${dirtyReviewPaths.size} review files`
    if (!window.confirm(`discard unsaved changes to ${label}?`)) return false
    restoreReviewEditCaches(editPaths)
    setReviewDirtyPaths(new Set())
    return true
  }, [reviewDirtyPaths, restoreReviewEditCaches, reviewSaveBarrier])

  const confirmDiscardAllEdits = useCallback(() => {
    if (!reviewSaveBarrier.canTransition()) return false
    const dirtyReviewPaths = currentReviewDirtyPaths(
      reviewEditBackupsRef.current.keys(),
      cacheRef.current,
      reviewDirtyPaths,
    )
    const count = new Set([...dirtyPaths, ...dirtyReviewPaths]).size
    if (count === 0) return true
    if (
      !window.confirm(
        `discard unsaved changes in ${count} ${count === 1 ? 'file' : 'files'}?`,
      )
    ) {
      return false
    }
    restoreReviewEditCaches(reviewEditBackupsRef.current.keys())
    setReviewDirtyPaths(new Set())
    return true
  }, [dirtyPaths, reviewDirtyPaths, restoreReviewEditCaches, reviewSaveBarrier])

  useEffect(() => {
    if (
      dirtyPaths.size === 0 &&
      reviewDirtyPaths.size === 0 &&
      !reviewSavePending
    ) {
      return
    }
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener('beforeunload', warn)
    return () => window.removeEventListener('beforeunload', warn)
  }, [dirtyPaths, reviewDirtyPaths, reviewSavePending])

  const forceReviewScope = useCallback(
    (scope: ReviewScopeSelection) => {
      if (!reviewSaveBarrier.canTransition()) return false
      // Forced transitions retire in-flight scope requests so a late Git result
      // cannot replace a file selected from the chat summary or live watcher.
      scopeLoadSeqRef.current += 1
      setReviewScope(scope)
      setScopeLoading(scope.kind !== 'last-turn')
      setScopeError(null)
      return true
    },
    [reviewSaveBarrier],
  )

  const beginReviewTurn = useCallback(
    (turnId: string) => {
      if (turnId === lastReviewKeyRef.current) return false
      if (!reviewSaveBarrier.canTransition()) return false
      const reviewDrafts = [...reviewDirtyPathsRef.current]
      if (reviewDrafts.length > 0) {
        setTabs((previous) => {
          let next = previous
          for (const path of reviewDrafts) next = openPinned(next, path)
          return next
        })
      }
      reviewEditBackupsRef.current.clear()
      setReviewDirtyPaths(new Set())
      lastReviewKeyRef.current = turnId
      setReviewKey(turnId)
      reviewEpochRef.current += 1
      reviewWindowRef.current = {
        turnId,
        epoch: reviewEpochRef.current,
        active: false,
        completedAtMs: null,
      }
      scopeMetadataSeqRef.current += 1
      diffRequestRef.current += 1
      if (liveTimerRef.current !== null)
        window.clearTimeout(liveTimerRef.current)
      liveTimerRef.current = null
      changedAbsRef.current = new Map()
      reviewEligibleAbsRef.current = new Set()
      changedDirsRef.current = new Set()
      followRef.current = null
      preparedTurnRef.current = null
      baselineRef.current = new Map()
      baselineKindsRef.current = new Map()
      baselineCompleteRef.current = false
      baselineCapturedRef.current = false
      baselineReadyRef.current = Promise.resolve()
      setBaselineCoverage(null)
      reviewEntriesRef.current = new Map()
      setReviewEntries(new Map())
      setReviewSummary([])
      setScopeMetadataLoading(false)
      setScopeMetadataError(null)
      setTurnOutside(0)
      setTurnOutsideRoot(null)
      if (reviewScopeRef.current.kind === 'last-turn') {
        setDiff(null)
      }
      return true
    },
    [reviewSaveBarrier],
  )

  // This runs inside Harness's awaited pre-turn hook: the snapshot is fully
  // frozen before model/tool execution can create, edit, rename, or delete.
  useHarnessPreTurn(host, conversationId, tabId, async ({ turn_id }) => {
    await reviewSaveBarrier.wait()
    const currentRoot = rootRef.current
    if (currentRoot === null) return
    beginReviewTurn(turn_id)
    reviewWindowRef.current = {
      turnId: turn_id,
      epoch: reviewEpochRef.current,
      active: true,
      completedAtMs: null,
    }
    if (preparedTurnRef.current === turn_id) {
      await baselineReadyRef.current
      return
    }
    preparedTurnRef.current = turn_id
    const generation = rootGenerationRef.current
    const epoch = reviewEpochRef.current
    baselineRef.current = new Map()
    baselineKindsRef.current = new Map()
    baselineCompleteRef.current = false
    baselineCapturedRef.current = false
    const snapshot = captureWorkspaceBaseline(host, currentRoot, reviewablePath)
      .then(({ contents, kinds, complete, coverage }) => {
        if (
          rootGenerationRef.current !== generation ||
          reviewEpochRef.current !== epoch ||
          rootRef.current !== currentRoot
        ) {
          return
        }
        baselineRef.current = new Map(contents)
        baselineKindsRef.current = kinds
        baselineCompleteRef.current = complete
        baselineCapturedRef.current = true
        setBaselineCoverage(coverage)
      })
      .catch(() => {
        // Git can still provide HEAD; non-Git rows fail closed with a clear
        // unavailable-baseline message instead of showing a false empty diff.
      })
    baselineReadyRef.current = snapshot
    await snapshot
  })

  // Catch up when the page mounted after pre-turn (or against an older
  // Harness without hook support). It can still track rows, but deliberately
  // does not pretend that a post-write read is a pre-turn baseline.
  useEffect(() => {
    if (observedReviewKey === null) return
    if (!reviewSaveBarrier.canTransition()) return
    beginReviewTurn(observedReviewKey)
    reviewWindowRef.current = {
      turnId: observedReviewKey,
      epoch: reviewEpochRef.current,
      active: observedReview.active,
      completedAtMs: observedReview.completedAtMs,
    }
  }, [
    observedReviewKey,
    observedReview.active,
    observedReview.completedAtMs,
    beginReviewTurn,
    reviewSavingPaths,
    reviewSaveBarrier,
  ])

  useEffect(() => {
    if (emptyTurnFallbackTimerRef.current !== null) {
      window.clearTimeout(emptyTurnFallbackTimerRef.current)
      emptyTurnFallbackTimerRef.current = null
    }
    if (
      observedReview.turnId === null ||
      observedReview.active ||
      observedReview.completedAtMs === null ||
      reviewScopeRef.current.kind !== 'last-turn' ||
      !followHarnessTurnsRef.current
    ) {
      return
    }
    const completedTurnId = observedReview.turnId
    emptyTurnFallbackTimerRef.current = window.setTimeout(() => {
      emptyTurnFallbackTimerRef.current = null
      if (
        reviewWindowRef.current.turnId !== completedTurnId ||
        reviewWindowRef.current.active ||
        reviewScopeRef.current.kind !== 'last-turn' ||
        reviewEntriesRef.current.size > 0
      ) {
        return
      }
      forceReviewScope(DEFAULT_REVIEW_SCOPE)
    }, EMPTY_TURN_FALLBACK_MS)
    return () => {
      if (emptyTurnFallbackTimerRef.current !== null) {
        window.clearTimeout(emptyTurnFallbackTimerRef.current)
        emptyTurnFallbackTimerRef.current = null
      }
    }
  }, [
    forceReviewScope,
    observedReview.active,
    observedReview.completedAtMs,
    observedReview.turnId,
  ])

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
    loadTabUiState(host, tabId)
      .then((state) => {
        if (!cancelled) setRestored(state)
      })
      .catch(() => {
        if (!cancelled) setRestored(null)
      })
    return () => {
      cancelled = true
    }
  }, [host, tabId])

  // Root resolution waits for BOTH. A chat's working directory is the
  // live source of truth for a split Shell pane; persisted state is only
  // restored when there is no current chat folder. Both may be subfolders
  // of an allowed base path, not just the base paths themselves.
  useEffect(() => {
    if (!info || restored === 'loading' || root !== null) return
    let cancelled = false
    const seq = ++rootResolveSeqRef.current
    const requested = workingDir ?? restored?.root ?? info.primary_root
    const requestedWorkingDir = workingDir ?? null
    workspaceValidate(host, requested)
      .then(({ path: next }) => {
        if (cancelled || rootResolveSeqRef.current !== seq) return
        if (requestedWorkingDir !== null) {
          acknowledgedWorkingDirRef.current =
            acknowledgeValidatedWorkingDirectory(
              acknowledgedWorkingDirRef.current,
              requestedWorkingDir,
              workingDirRef.current,
              true,
            )
        }
        function restoreTerminalUiState(state: TabUiState): void {
          if (state.terminalOpen) setTerminalOpen(true)
          if (state.terminalDock) setTerminalDock(state.terminalDock)
          if (state.terminalActive) setTerminalActive(true)
          setTerminalBottomSize(
            clampTerminalSize(
              state.terminalBottomSize,
              TERMINAL_BOTTOM_DEFAULT_SIZE,
            ),
          )
          setTerminalRightSize(
            clampTerminalSize(
              state.terminalRightSize,
              TERMINAL_RIGHT_DEFAULT_SIZE,
            ),
          )
          dispatchTerminalWorkspace({
            type: 'workspace-restored',
            state: state.terminalWorkspace
              ? normalizeTerminalWorkspace(state.terminalWorkspace, next)
              : createTerminalWorkspace(next),
          })
        }
        setRoot(next)
        if (
          restored?.root &&
          requested === restored.root &&
          next === restored.root
        ) {
          setTabs(restoreTabs(restored.open, restored.active))
          setExpanded(restored.expanded)
          setShowHidden(restored.showHidden ?? false)
          restoreTerminalUiState(restored)
        } else if (
          restored &&
          !restored.root &&
          requested === info.primary_root
        ) {
          // Legacy/first save without a root: restore against the primary.
          setTabs(restoreTabs(restored.open, restored.active))
          setExpanded(restored.expanded)
          setShowHidden(restored.showHidden ?? false)
          restoreTerminalUiState(restored)
        } else {
          dispatchTerminalWorkspace({
            type: 'workspace-restored',
            state: createTerminalWorkspace(next),
          })
        }
      })
      .catch(() => {
        if (!cancelled && rootResolveSeqRef.current === seq) {
          setRoot(info.primary_root)
          dispatchTerminalWorkspace({
            type: 'workspace-restored',
            state: createTerminalWorkspace(info.primary_root),
          })
        }
      })
    return () => {
      cancelled = true
    }
  }, [host, info, restored, root, workingDir])

  // ── data loads (gated on the resolved root) ──
  const gitSeqRef = useRef(0)
  const refreshGit = useCallback((): Promise<GitState | null> => {
    if (!root) return Promise.resolve(null)
    const seq = ++gitSeqRef.current
    return gitChanges(host, root)
      .then((state) => {
        if (gitSeqRef.current === seq) setGit(state)
        return state
      })
      .catch((err: unknown) => {
        if (gitSeqRef.current === seq) {
          setGit({
            kind: 'error',
            message: errorMessage(err),
          })
        }
        return null
      })
  }, [host, root])

  const treeSeqRef = useRef(0)
  const refreshTree = useCallback(() => {
    if (!root) return
    const seq = ++treeSeqRef.current
    coderTree(host, root, showHidden)
      .then((out) => {
        if (treeSeqRef.current === seq) setTree(flattenTree(out.root))
      })
      .catch(() => {
        if (treeSeqRef.current === seq) {
          setTree({ paths: [], kinds: new Map(), truncations: [] })
        }
      })
  }, [host, root, showHidden])

  // Separate effects: toggling the hidden filter reloads the TREE only —
  // the git listing is unaffected and must not flash back to loading.
  useEffect(() => {
    setTree(null)
    // Lazy subtrees were fetched with the OLD hidden filter — always stale.
    setSubtrees(new Map())
    refreshTree()
  }, [refreshTree])

  // The tree the sidebar renders: the budgeted base snapshot plus every
  // lazily fetched subtree spliced in under its folder.
  const mergedTree = useMemo((): FlatTree | null => {
    if (tree === null) return null
    if (subtrees.size === 0) return tree
    const paths = [...tree.paths]
    const kinds = new Map(tree.kinds)
    const seen = new Set(paths)
    for (const [dir, sub] of subtrees) {
      for (const p of sub.paths) {
        const joined = `${dir}/${p}`
        if (!seen.has(joined)) {
          seen.add(joined)
          paths.push(joined)
        }
      }
      for (const [p, k] of sub.kinds) kinds.set(`${dir}/${p}`, k)
    }
    return { paths, kinds, truncations: tree.truncations }
  }, [tree, subtrees])

  // Git is an optional baseline/enrichment source. The review set itself
  // is also fed by shell::changed, so plain temporary folders behave the
  // same as worktrees.
  useEffect(() => {
    if (git?.kind !== 'ready') return
    setReviewEntries((previous) => {
      const next = mergeGitReviewEntries(previous, git.changes, false)
      reviewEntriesRef.current = next
      return next
    })
  }, [git, reviewKey])

  const visibleReviewEntries =
    reviewScope.kind === 'last-turn' ? reviewEntries : scopeEntries
  const visibleReviewEntriesRef =
    useRef<ReadonlyMap<string, ReviewEntry>>(visibleReviewEntries)
  visibleReviewEntriesRef.current = visibleReviewEntries
  const visibleReviewSummary =
    reviewScope.kind === 'last-turn' ? reviewSummary : scopeSummary
  const reviewChanges = useMemo<readonly GitChange[]>(
    () => [...visibleReviewEntries.values()].map((entry) => entry.change),
    [visibleReviewEntries],
  )
  const scopeEmpty =
    reviewScope.kind !== 'last-turn' &&
    !scopeLoading &&
    scopeError === null &&
    scopeEntries.size === 0
  const currentTurnEmpty =
    reviewScope.kind === 'last-turn' &&
    observedReview.active &&
    reviewEntries.size === 0
  const sessionActivity = useMemo(
    () =>
      root === null
        ? { inside: 0, outside: 0, outsideRoot: null }
        : summarizeSessionActivity(sessionTurns, root),
    [root, sessionTurns],
  )
  const canUseSessionOutsideForChat =
    !!conversationId &&
    typeof (host as WorkingDirectoryHost).chat?.requestWorkingDirectoryChange ===
      'function'

  // The panel, not the viewport, decides what fits: a shell page shares the
  // console with other panels. Below the same width the stylesheet treats as
  // narrow, the sidebar becomes an overlay, so it starts out of the way.
  useEffect(() => {
    if (!frameEl) return
    const measure = () =>
      setNarrow(frameEl.getBoundingClientRect().width <= 720)
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

  const rootLabel = useMemo(
    () => root?.split('/').filter(Boolean).slice(-1)[0] ?? 'workspace',
    [root],
  )

  const orderedReviewEntries = useMemo<readonly ReviewEntry[]>(
    () => [...visibleReviewEntries.values()],
    [visibleReviewEntries],
  )
  const reviewTotals = useMemo(
    () =>
      visibleReviewSummary.reduce(
        (total, file) => ({
          add: total.add + (file.add ?? 0),
          del: total.del + (file.del ?? 0),
          ready: total.ready + (file.state === 'ready' ? 1 : 0),
          pending: total.pending + (file.state === 'pending' ? 1 : 0),
          unavailable:
            total.unavailable + (file.state === 'unavailable' ? 1 : 0),
        }),
        { add: 0, del: 0, ready: 0, pending: 0, unavailable: 0 },
      ),
    [visibleReviewSummary],
  )
  const reviewScopeCounts = useMemo<ReviewScopeCounts>(() => {
    const next: ReviewScopeCounts = {
      ...scopeCounts,
      'last-turn': reviewEntries.size,
      session: sessionActivity.inside,
    }
    if (isLiveGitReviewScope(reviewScope)) {
      if (scopeLoading || scopeError !== null) delete next[reviewScope.kind]
      else next[reviewScope.kind] = orderedReviewEntries.length
    }
    return next
  }, [
    orderedReviewEntries.length,
    reviewEntries.size,
    reviewScope,
    sessionActivity.inside,
    scopeCounts,
    scopeError,
    scopeLoading,
  ])
  // The Files tree is also the review navigator. Review-only rows keep
  // deleted files visible even after they disappear from coder::tree.
  const reviewTree = useMemo(
    () => withReviewChanges(mergedTree, reviewChanges),
    [mergedTree, reviewChanges],
  )
  const changedDirsKey = useMemo(
    () => changedParentDirs(reviewChanges).join('\n'),
    [reviewChanges],
  )
  useEffect(() => {
    if (changedDirsKey === '') return
    const changedDirs = changedDirsKey.split('\n')
    setExpanded((previous) => {
      const next = new Set(previous)
      let added = false
      for (const dir of changedDirs) {
        if (!next.has(dir)) {
          next.add(dir)
          added = true
        }
      }
      return added ? [...next] : previous
    })
  }, [changedDirsKey])

  // Expanding a folder the snapshot didn't reach fetches its listing.
  const subtreeLoadRef = useRef<Set<string>>(new Set())
  useEffect(() => {
    if (root === null || mergedTree === null) return
    const generation = rootGenerationRef.current
    for (const dir of expanded) {
      if (mergedTree.kinds.get(dir) !== 'dir') continue
      if (subtrees.has(dir) || subtreeLoadRef.current.has(dir)) continue
      const prefix = `${dir}/`
      let hasChild = false
      for (const key of mergedTree.kinds.keys()) {
        if (key.startsWith(prefix)) {
          hasChild = true
          break
        }
      }
      if (hasChild) continue
      subtreeLoadRef.current.add(dir)
      coderTree(host, joinPath(root, dir), showHidden)
        .then((out) => {
          if (rootGenerationRef.current !== generation) return
          setSubtrees((prev) => new Map(prev).set(dir, flattenTree(out.root)))
        })
        .catch(() => {
          if (rootGenerationRef.current !== generation) return
          // Inaccessible folder — recorded as fetched-and-empty so the
          // load effect doesn't refetch it on every live burst; a change
          // under it drops the entry and retries.
          setSubtrees((prev) =>
            new Map(prev).set(dir, {
              paths: [],
              kinds: new Map(),
              truncations: [],
            }),
          )
        })
        .finally(() => {
          if (rootGenerationRef.current === generation)
            subtreeLoadRef.current.delete(dir)
        })
    }
  }, [expanded, mergedTree, subtrees, root, showHidden, host])

  useEffect(() => {
    setGit(null)
    refreshGit()
  }, [refreshGit])

  // ── live updates: the watched root streams every change here ──
  // The worker runs a system-level watch on the browsed root for this
  // binding (`shell::changed`), so agent writes, shell::exec side effects,
  // and outside-the-engine edits all land: each event refreshes the tree
  // and git views, and reloads the ACTIVE file when it was the one written
  // (a clean buffer follows the disk, a dirty one keeps the user's edits).
  // Bursts coalesce worker-side and again in a short window here.
  const [fileBump, setFileBump] = useState(0)
  const tabsRef = useRef(tabs)
  tabsRef.current = tabs
  const gitRef = useRef(git)
  gitRef.current = git
  const liveTimerRef = useRef<number | null>(null)
  const changedAbsRef = useRef<Map<string, string>>(new Map())
  const reviewEligibleAbsRef = useRef<Set<string>>(new Set())
  const changedDirsRef = useRef<Set<string>>(new Set())

  const reloadActiveFile = useCallback(() => {
    const currentRoot = rootRef.current
    const generation = rootGenerationRef.current
    const active = tabsRef.current.active
    if (!currentRoot || !active) return
    // An image preview follows the disk through its own render path; a
    // text read here would overwrite the cache with mangled bytes.
    if (cacheRef.current.get(active)?.image) return
    const absPath = joinPath(currentRoot, active)
    if (!changedAbsRef.current.has(absPath)) return
    coderReadFile(host, absPath)
      .then((out) => {
        if (
          rootGenerationRef.current !== generation ||
          rootRef.current !== currentRoot
        )
          return
        if (tabsRef.current.active !== active) return
        const content = out.content ?? ''
        const entry = cacheRef.current.get(active)
        if (!entry) return
        if (
          !refreshCleanEditorCacheEntry(
            entry,
            content,
            out.revision ?? undefined,
          )
        )
          return
        setFileBump((n) => n + 1)
      })
      .catch(() => {
        // A deleted-then-read race resolves through the next tree refresh.
      })
  }, [host])

  // The last written file in a burst follows the writer into review. All
  // files in the burst stay in reviewEntries, independent of Git.
  const followRef = useRef<{ rel: string; kind: string } | null>(null)
  const diffRef = useRef(diff)
  diffRef.current = diff
  const treeRef = useRef(tree)
  treeRef.current = tree
  const openReviewEntry = useCallback(
    (entry: ReviewEntry) => {
      if (!reviewSaveBarrier.canTransition()) return false
      setTerminalActive(false)
      diffRequestRef.current += 1
      setContextDiff(null)
      setTabs((state) => openPreview(state, entry.path))
      setDiff(diffForReviewEntry(entry))
      setReviewRefreshEpoch((value) => value + 1)
      return true
    },
    [reviewSaveBarrier],
  )

  useEffect(() => {
    if (
      !conversationId ||
      root === null ||
      observedReview.turnId === null ||
      observedReview.active ||
      observedReview.completedAtMs === null
    ) {
      return
    }
    let cancelled = false
    const completedTurnId = observedReview.turnId
    const completedRoot = root
    void fetchSessionTurn(host, conversationId, completedTurnId)
      .then(async (turn) => {
        if (
          cancelled ||
          turn === null ||
          rootRef.current !== completedRoot ||
          reviewWindowRef.current.turnId !== completedTurnId
        ) {
          return
        }
        const mapped = reviewEntriesFromTurn(turn, completedRoot)
        const storedEntries = await withoutUnreviewableBaselines(
          host,
          completedRoot,
          mapped.entries,
        )
        if (
          cancelled ||
          rootRef.current !== completedRoot ||
          reviewWindowRef.current.turnId !== completedTurnId
        ) {
          return
        }
        setTurnOutside(mapped.outside)
        setTurnOutsideRoot(mapped.outsideRoot)

        const merged = new Map(reviewEntriesRef.current)
        for (const [path, stored] of storedEntries) {
          const live = merged.get(path)
          merged.set(path, {
            ...stored,
            ...(live ?? {}),
            baseline:
              live?.baseline === undefined || live.baseline === null
                ? stored.baseline
                : live.baseline,
          })
        }
        reviewEntriesRef.current = merged
        setReviewEntries(merged)

        const scope = reviewScopeRef.current
        if (
          !shouldEnterTurnScope(
            followHarnessTurnsRef.current,
            scope,
            merged.size,
          )
        ) {
          return
        }
        forceReviewScope(LAST_TURN_SCOPE)
        const activePath = diffRef.current?.change.path
        const entry =
          (activePath ? merged.get(activePath) : undefined) ??
          merged.values().next().value
        if (entry) openReviewEntry(entry)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [
    conversationId,
    forceReviewScope,
    host,
    observedReview.active,
    observedReview.completedAtMs,
    observedReview.turnId,
    openReviewEntry,
    root,
  ])

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

  useEffect(() => {
    refreshSessionTurns()
    if (!observedReview.active) return
    const timer = window.setInterval(refreshSessionTurns, 1_500)
    return () => window.clearInterval(timer)
  }, [
    observedReview.active,
    observedReview.completedAtMs,
    observedReview.turnId,
    refreshSessionTurns,
  ])

  const loadScopeMetadata = useCallback(() => {
    if (root === null) return
    const seq = ++scopeMetadataSeqRef.current
    setScopeMetadataLoading(true)
    setScopeMetadataError(null)
    refreshSessionTurns()
    void Promise.all([
      gitRecentCommits(host, root),
      gitRefs(host, root),
      gitComparison(host, root, 'uncommitted'),
      gitComparison(host, root, 'unstaged'),
      gitComparison(host, root, 'staged'),
    ])
      .then(([commits, refs, uncommitted, unstaged, staged]) => {
        if (scopeMetadataSeqRef.current !== seq || rootRef.current !== root)
          return
        setScopeCommits(commits.kind === 'ready' ? commits.commits : [])
        setScopeRefs(refs.kind === 'ready' ? refs.refs : [])
        const counts: ReviewScopeCounts = {}
        if (uncommitted.kind === 'ready')
          counts.uncommitted = uncommitted.changes.length
        if (unstaged.kind === 'ready')
          counts.unstaged = unstaged.changes.length
        if (staged.kind === 'ready') counts.staged = staged.changes.length
        setScopeCounts(counts)
        const failure =
          commits.kind === 'error'
            ? commits.message
            : refs.kind === 'error'
              ? refs.message
              : commits.kind === 'not-a-repo' || refs.kind === 'not-a-repo'
                ? 'not a git repository'
                : null
        setScopeMetadataError(failure)
      })
      .catch((error: unknown) => {
        if (scopeMetadataSeqRef.current !== seq || rootRef.current !== root)
          return
        setScopeMetadataError(errorMessage(error))
      })
      .finally(() => {
        if (scopeMetadataSeqRef.current === seq) setScopeMetadataLoading(false)
      })
  }, [host, refreshSessionTurns, root])

  const loadReviewScope = useCallback(
    (
      scope: Exclude<ReviewScopeSelection, { kind: 'last-turn' }>,
      preferredPath?: string | null,
    ) => {
      if (root === null) return
      const seq = ++scopeLoadSeqRef.current
      setScopeLoading(true)
      setScopeError(null)
      setTurnOutside(0)
      setTurnOutsideRoot(null)
      const applyMapped = (mapped: TurnEntries) => {
        if (scopeLoadSeqRef.current !== seq || rootRef.current !== root)
          return
        scopeEntriesRef.current = mapped.entries
        setScopeEntries(mapped.entries)
        setTurnOutside(mapped.outside)
        setTurnOutsideRoot(mapped.outsideRoot)
        const activePath = diffRef.current?.change.path
        const entry =
          (preferredPath
            ? mapped.entries.get(preferredPath)
            : undefined) ??
          (activePath ? mapped.entries.get(activePath) : undefined) ??
          (preferredPath === undefined
            ? mapped.entries.values().next().value
            : undefined)
        if (entry) openReviewEntry(entry)
        else {
          diffRequestRef.current += 1
          setDiff(null)
        }
      }
      if (scope.kind === 'session') {
        if (!conversationId) {
          setScopeLoading(false)
          setScopeError('no chat session')
          return
        }
        void fetchSessionTurns(host, conversationId)
          .then(async (summaries) => {
            if (scopeLoadSeqRef.current !== seq || rootRef.current !== root)
              return null
            setSessionTurns(summaries)
            const relevant = summaries.filter((turn) =>
              turn.files.some(
                (file) => relativeToRoot(file.path, root) !== null,
              ),
            )
            const turns = (
              await Promise.all(
                relevant.map((turn) =>
                  fetchSessionTurn(host, conversationId, turn.turn_id),
                ),
              )
            ).filter((turn): turn is SessionTurn => turn !== null)
            const mapped = reviewEntriesFromSession(turns, root)
            const entries = await withoutUnreviewableBaselines(
              host,
              root,
              mapped.entries,
            )
            if (scopeLoadSeqRef.current !== seq || rootRef.current !== root)
              return null
            const activity = summarizeSessionActivity(summaries, root)
            return {
              entries,
              outside: activity.outside,
              outsideRoot: activity.outsideRoot,
            } satisfies TurnEntries
          })
          .then((mapped) => {
            if (mapped) applyMapped(mapped)
          })
          .catch((error: unknown) => {
            if (scopeLoadSeqRef.current !== seq || rootRef.current !== root)
              return
            scopeEntriesRef.current = new Map()
            setScopeEntries(new Map())
            setScopeError(errorMessage(error))
            diffRequestRef.current += 1
            setDiff(null)
          })
          .finally(() => {
            if (scopeLoadSeqRef.current === seq) setScopeLoading(false)
          })
        return
      }
      if (scope.kind === 'turn') {
        if (!conversationId) {
          setScopeLoading(false)
          setScopeError('no chat session')
          return
        }
        void fetchSessionTurn(host, conversationId, scope.turnId)
          .then(async (turn) => {
            if (scopeLoadSeqRef.current !== seq || rootRef.current !== root)
              return
            const mapped = turn
              ? reviewEntriesFromTurn(turn, root)
              : { entries: new Map(), outside: 0, outsideRoot: null }
            const entries = await withoutUnreviewableBaselines(
              host,
              root,
              mapped.entries,
            )
            if (scopeLoadSeqRef.current !== seq || rootRef.current !== root)
              return
            applyMapped({ ...mapped, entries })
          })
          .catch((error: unknown) => {
            if (scopeLoadSeqRef.current !== seq || rootRef.current !== root)
              return
            scopeEntriesRef.current = new Map()
            setScopeEntries(new Map())
            setScopeError(errorMessage(error))
            diffRequestRef.current += 1
            setDiff(null)
          })
          .finally(() => {
            if (scopeLoadSeqRef.current === seq) setScopeLoading(false)
          })
        return
      }
      const comparison =
        scope.kind === 'uncommitted' ||
        scope.kind === 'unstaged' ||
        scope.kind === 'staged'
          ? gitComparison(host, root, scope.kind satisfies GitComparisonScope)
          : scope.kind === 'commit'
            ? gitCommitComparison(host, root, scope.sha)
            : gitBranchComparison(host, root, scope.ref)
      void comparison
        .then((state) => {
          if (scopeLoadSeqRef.current !== seq || rootRef.current !== root)
            return
          if (state.kind !== 'ready') {
            if (shouldFallbackToTurnScope(scope, state.kind)) {
              followHarnessTurnsRef.current = true
              if (conversationId) {
                forceReviewScope(SESSION_SCOPE)
                return
              }
              forceReviewScope(LAST_TURN_SCOPE)
              const activePath = diffRef.current?.change.path
              const entry =
                (preferredPath
                  ? reviewEntriesRef.current.get(preferredPath)
                  : undefined) ??
                (activePath
                  ? reviewEntriesRef.current.get(activePath)
                  : undefined) ??
                reviewEntriesRef.current.values().next().value
              if (entry) openReviewEntry(entry)
              else {
                diffRequestRef.current += 1
                setDiff(null)
              }
              return
            }
            const message =
              state.kind === 'error' ? state.message : 'not a git repository'
            scopeEntriesRef.current = new Map()
            setScopeEntries(new Map())
            setScopeError(message)
            diffRequestRef.current += 1
            setDiff(null)
            return
          }
          const next = reviewEntriesFromGit(state.changes)
          scopeEntriesRef.current = next
          setScopeEntries(next)
          if (isLiveGitReviewScope(scope)) {
            setScopeCounts((previous) => ({
              ...previous,
              [scope.kind]: next.size,
            }))
          }
          const activePath = diffRef.current?.change.path
          const entry =
            (preferredPath ? next.get(preferredPath) : undefined) ??
            (activePath ? next.get(activePath) : undefined) ??
            (preferredPath === undefined
              ? next.values().next().value
              : undefined)
          if (entry) openReviewEntry(entry)
          else {
            diffRequestRef.current += 1
            setDiff(null)
          }
        })
        .catch((error: unknown) => {
          if (scopeLoadSeqRef.current !== seq || rootRef.current !== root)
            return
          scopeEntriesRef.current = new Map()
          setScopeEntries(new Map())
          setScopeError(errorMessage(error))
          diffRequestRef.current += 1
          setDiff(null)
        })
        .finally(() => {
          if (scopeLoadSeqRef.current === seq) setScopeLoading(false)
        })
    },
    [host, root, conversationId, forceReviewScope, openReviewEntry],
  )

  const onReviewEditDirtyChange = useCallback(
    (path: string, dirty: boolean) => {
      // The page-owned draft remains authoritative until the row explicitly
      // sends a clean draft, cancels, or saves.
      if (!dirty) return
      setReviewDirtyPaths((previous) => {
        if (previous.has(path)) return previous
        const next = new Set(previous)
        next.add(path)
        return next
      })
    },
    [],
  )

  const onReviewEditSavingChange = useCallback(
    (path: string, saving: boolean) => {
      setReviewSavingPaths(reviewSaveBarrier.update(path, saving))
    },
    [reviewSaveBarrier],
  )

  const onRequestReviewEdit = useCallback(
    (path: string) => {
      if (!reviewSaveBarrier.canTransition()) return false
      if (reviewEditBackupsRef.current.has(path)) return true
      const cached = cacheRef.current.get(path)
      if (
        cached !== undefined &&
        cached.draft !== cached.savedContent &&
        !window.confirm(`discard the existing editor draft for ${path}?`)
      ) {
        return false
      }
      reviewEditBackupsRef.current.set(path, {
        cache:
          cached === undefined
            ? null
            : { ...cached, draft: cached.savedContent },
        hadTab: tabsRef.current.tabs.some((tab) => tab.path === path),
      })
      if (cached !== undefined && cached.draft !== cached.savedContent) {
        setDirtyPaths((previous) => {
          const next = new Set(previous)
          next.delete(path)
          return next
        })
      }
      return true
    },
    [reviewSaveBarrier],
  )

  const onReviewEditDraftChange = useCallback(
    (path: string, edit: ReviewEditDraft | null) => {
      if (edit === null) {
        restoreReviewEditCaches([path])
        setReviewDirtyPaths((previous) => {
          if (!previous.has(path)) return previous
          const next = new Set(previous)
          next.delete(path)
          return next
        })
        return
      }
      const previous = cacheRef.current.get(path)
      cacheRef.current.set(path, {
        savedContent: edit.savedContent,
        draft: edit.draft,
        revision: edit.revision,
        readOnly: null,
        mode: edit.mode ?? null,
        size: previous?.size ?? null,
      })
      const dirty = edit.draft !== edit.savedContent
      setReviewDirtyPaths((previous) => {
        if (previous.has(path) === dirty) return previous
        const next = new Set(previous)
        if (dirty) next.add(path)
        else next.delete(path)
        return next
      })
      setDirtyPaths((paths) => {
        if (paths.has(path) === dirty) return paths
        const next = new Set(paths)
        if (dirty) next.add(path)
        else next.delete(path)
        return next
      })
      if (dirty) setTabs((previousTabs) => openPinned(previousTabs, path))
    },
    [restoreReviewEditCaches],
  )

  const onReviewFileSaved = useCallback(
    (path: string, contents: string, revision?: string) => {
      const backup = reviewEditBackupsRef.current.get(path)
      reviewEditBackupsRef.current.delete(path)
      setReviewDirtyPaths((previous) => {
        if (!previous.has(path)) return previous
        const next = new Set(previous)
        next.delete(path)
        return next
      })
      const cached = cacheRef.current.get(path)
      if (cached !== undefined && backup?.hadTab !== false) {
        cached.savedContent = contents
        cached.draft = contents
        cached.revision = revision ?? cached.revision
        setFileBump((value) => value + 1)
      } else if (backup?.hadTab === false) {
        cacheRef.current.delete(path)
        setTabs((previous) => closeTab(previous, path))
      }
      setDirtyPaths((previous) => {
        if (!previous.has(path)) return previous
        const next = new Set(previous)
        next.delete(path)
        return next
      })
      refreshTree()
      void refreshGit()
      const scope = reviewScopeRef.current
      if (scope.kind === 'last-turn') {
        setReviewRefreshEpoch((value) => value + 1)
      } else {
        loadReviewScope(scope)
      }
    },
    [loadReviewScope, refreshGit, refreshTree],
  )

  useEffect(() => {
    if (reviewScope.kind !== 'last-turn') loadReviewScope(reviewScope)
  }, [reviewScope, loadReviewScope])

  const selectReviewScope = useCallback(
    (next: ReviewScopeSelection) => {
      if (!confirmDiscardReviewEdits()) return
      followHarnessTurnsRef.current = next.kind === 'last-turn'
      setScopeSummary([])
      setScopeError(null)
      setReviewMenuOpen(false)
      if (next.kind === 'last-turn') {
        forceReviewScope(LAST_TURN_SCOPE)
        const activePath = diffRef.current?.change.path
        const entry =
          (activePath ? reviewEntriesRef.current.get(activePath) : undefined) ??
          reviewEntriesRef.current.values().next().value
        if (entry) openReviewEntry(entry)
        else {
          diffRequestRef.current += 1
          setDiff(null)
        }
        return
      }
      scopeLoadSeqRef.current += 1
      setReviewScope(next)
      scopeEntriesRef.current = new Map()
      setScopeEntries(new Map())
      setScopeLoading(true)
      diffRequestRef.current += 1
      setDiff(null)
    },
    [confirmDiscardReviewEdits, forceReviewScope, openReviewEntry],
  )

  useShellReviewSummaryBridge({
    sessionId: conversationId,
    sourceId: tabId,
    turnId: reviewKey,
    files: reviewSummary,
    onSelectFile: (path) => {
      if (!confirmDiscardReviewEdits()) return
      const entry = reviewEntriesRef.current.get(path)
      if (entry) {
        followHarnessTurnsRef.current = true
        forceReviewScope(LAST_TURN_SCOPE)
        openReviewEntry(entry)
      }
    },
  })

  useWorkspaceChanges(host, root, (event) => {
    if (rootTransitionRef.current) return
    if (event.root !== rootRef.current) return
    if (isShellUiStatePath(event.path)) return
    const eventAbs = joinPath(event.root, event.path)
    changedAbsRef.current.set(eventAbs, event.kind)
    // Directories refresh the tree but must never open as files —
    // reading one is a C210.
    if (event.dir === true) {
      changedDirsRef.current.add(eventAbs)
    } else {
      const currentRoot = rootRef.current
      if (currentRoot) {
        const prefix = currentRoot.endsWith('/')
          ? currentRoot
          : `${currentRoot}/`
        if (eventAbs.startsWith(prefix)) {
          const rel = eventAbs.slice(prefix.length)
          if (
            reviewablePath(rel) &&
            canCaptureHarnessWorkspaceChange(
              reviewWindowRef.current,
              lastReviewKeyRef.current,
              reviewEpochRef.current,
              Date.now(),
            )
          ) {
            reviewEligibleAbsRef.current.add(eventAbs)
            followRef.current = { rel, kind: event.kind }
          }
        }
      }
    }
    if (liveTimerRef.current !== null) return
    const generation = rootGenerationRef.current
    const reviewEpoch = reviewEpochRef.current
    liveTimerRef.current = window.setTimeout(() => {
      // Keep the burst buffered until the turn's baseline snapshot is
      // complete. New watcher events coalesce into the same maps meanwhile.
      void baselineReadyRef.current.then(() => {
        liveTimerRef.current = null
        if (
          rootGenerationRef.current !== generation ||
          reviewEpochRef.current !== reviewEpoch
        )
          return
        // Capture the pre-refresh tree: watcher kinds are noisy, while this
        // tells an atomic replacement from a truly new path.
        const knownBefore =
          treeRef.current === null ? null : new Set(treeRef.current.paths)
        const kindsBefore = treeRef.current?.kinds
        reloadActiveFile()
        const follow = followRef.current
        followRef.current = null
        const changed = changedAbsRef.current
        changedAbsRef.current = new Map()
        const reviewEligible = reviewEligibleAbsRef.current
        reviewEligibleAbsRef.current = new Set()
        const changedDirs = changedDirsRef.current
        changedDirsRef.current = new Set()
        const currentRoot = rootRef.current
        if (currentRoot === null) return

        const prefix = currentRoot.endsWith('/')
          ? currentRoot
          : `${currentRoot}/`
        const fileEvents = [...changed]
          .filter(
            ([abs]) =>
              reviewEligible.has(abs) &&
              !changedDirs.has(abs) &&
              abs.startsWith(prefix),
          )
          .map(([abs, rawKind]) => ({
            abs,
            rawKind,
            rel: abs.slice(prefix.length),
          }))
          .filter(({ rel }) => reviewablePath(rel))

        // A lazily fetched subtree with a change under it is stale —
        // drop it; the load effect refetches while it stays expanded.
        setSubtrees((prev) => {
          let next: Map<string, FlatTree> | null = null
          for (const dir of prev.keys()) {
            const dirPrefix = `${joinPath(currentRoot, dir)}/`
            for (const abs of changed.keys()) {
              if (abs.startsWith(dirPrefix)) {
                next ??= new Map(prev)
                next.delete(dir)
                break
              }
            }
          }
          return next ?? prev
        })
        refreshTree()
        const followTicket = diffRequestRef.current
        void Promise.all([
          coderStatFiles(
            host,
            fileEvents.map(({ abs }) => abs),
          ).catch(() => null),
          refreshGit(),
        ]).then(async ([results, state]) => {
          if (
            rootGenerationRef.current !== generation ||
            reviewEpochRef.current !== reviewEpoch ||
            rootRef.current !== currentRoot
          ) {
            return
          }
          const existsByPath = new Map(
            results?.map((result) => [result.path, result.success] as const) ??
              [],
          )
          const currentGitChanges = state?.kind === 'ready' ? state.changes : []
          const currentGitByPath = new Map(
            currentGitChanges.map((change) => [change.path, change] as const),
          )
          let nextReview = reviewEntriesRef.current
          for (const { abs, rawKind, rel } of fileEvents) {
            const baselinePath = baselineCapturedRef.current
              ? classifyWorkspaceBaselinePath(
                  {
                    kinds: baselineKindsRef.current,
                    complete: baselineCompleteRef.current,
                  },
                  rel,
                )
              : null
            const priorKind = baselineCapturedRef.current
              ? (baselinePath?.priorKind ?? null)
              : kindsBefore === undefined
                ? undefined
                : kindsBefore.get(rel) === 'file'
                  ? 'file'
                  : kindsBefore.get(rel) === 'dir' ||
                      knownBefore?.has(`${rel}/`)
                    ? 'dir'
                    : null
            const baseline =
              baselineRef.current.get(rel) ??
              cacheRef.current.get(rel)?.savedContent
            const baselineUnavailable =
              baselinePath?.priorKind === 'file' && baseline === undefined
            const decision = normalizeLiveReviewEvent({
              path: rel,
              rawKind,
              priorKind,
              priorKindExact: baselinePath?.exact,
              priorBaseline: baseline,
              existsNow:
                results === null
                  ? rawKind !== 'deleted'
                  : existsByPath.get(abs) === true,
            })
            if (
              decision.action === 'ignore-directory' ||
              decision.action === 'ignore-delete'
            )
              continue
            nextReview = mergeReviewEntry(
              nextReview,
              rel,
              decision.action,
              decision.baseline,
              // An added/untracked Git status normally supplies an empty
              // baseline. Do not use that fallback when an incomplete tree
              // may simply have omitted an existing pre-turn file.
              canUseGitMetadataForLiveEntry(
                baselineCapturedRef.current,
                baselineUnavailable,
                decision.baseline,
              )
                ? currentGitByPath.get(rel)
                : undefined,
            )
          }
          const enriched = mergeGitReviewEntries(
            nextReview,
            currentGitChanges,
            false,
          )
          const validated = await withoutUnreviewableBaselines(
            host,
            currentRoot,
            enriched,
            new Set(fileEvents.map(({ rel }) => rel)),
          )
          if (
            rootGenerationRef.current !== generation ||
            reviewEpochRef.current !== reviewEpoch ||
            rootRef.current !== currentRoot
          ) {
            return
          }
          reviewEntriesRef.current = validated
          setReviewEntries(validated)

          const activeScope = reviewScopeRef.current
          if (
            activeScope.kind !== 'last-turn' &&
            shouldEnterTurnScope(
              followHarnessTurnsRef.current,
              activeScope,
              validated.size,
            )
          ) {
            forceReviewScope(LAST_TURN_SCOPE)
            const entry =
              (follow ? validated.get(follow.rel) : undefined) ??
              validated.values().next().value
            if (entry) openReviewEntry(entry)
            return
          }
          if (isLiveGitReviewScope(activeScope)) {
            loadReviewScope(activeScope, follow?.rel ?? null)
            return
          }
          if (
            activeScope.kind === 'last-turn' &&
            follow !== null &&
            diffRequestRef.current === followTicket
          ) {
            const entry =
              validated.get(follow.rel) ??
              [...fileEvents]
                .reverse()
                .map(({ rel }) => validated.get(rel))
                .find((candidate) => candidate !== undefined)
            if (entry) {
              openReviewEntry(entry)
            }
            return
          }
          const open = diffRef.current
          if (
            reviewScopeRef.current.kind === 'last-turn' &&
            open !== null &&
            changed.has(joinPath(currentRoot, open.change.path))
          ) {
            const entry = validated.get(open.change.path)
            if (entry) setDiff(diffForReviewEntry(entry))
            else setDiff(null)
          }
        })
      })
    }, 400)
  })
  useEffect(
    () => () => {
      if (liveTimerRef.current !== null) {
        window.clearTimeout(liveTimerRef.current)
      }
    },
    [],
  )

  // ── persistence: any state change after boot writes (debounced) ──
  const saver = useMemo(() => createTabUiStateSaver(host, tabId), [host, tabId])
  useEffect(() => () => saver.dispose(), [saver])
  const bootedRef = useRef(false)
  useEffect(() => {
    if (root === null) return
    if (!bootedRef.current) {
      // The first pass after restore replays state we just loaded.
      bootedRef.current = true
      return
    }
    saver.save({
      root,
      open: tabs.tabs,
      active: tabs.active,
      expanded,
      showHidden,
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
    tabs,
    expanded,
    showHidden,
    terminalOpen,
    terminalDock,
    terminalActive,
    terminalBottomSize,
    terminalRightSize,
    terminalWorkspace,
  ])

  // ── open/close/pin actions ──
  const previewFile = useCallback(
    (relPath: string) => {
      setContextDiff(null)
      if (!confirmDiscardReviewEdits()) return
      setTerminalActive(false)
      diffRequestRef.current += 1
      setDiff(null)
      setTabs((s) => openPreview(s, relPath))
    },
    [confirmDiscardReviewEdits],
  )

  const activateFile = useCallback(
    (relPath: string) => {
      const entry = visibleReviewEntriesRef.current.get(relPath)
      if (entry) openReviewEntry(entry)
      else previewFile(relPath)
    },
    [openReviewEntry, previewFile],
  )

  const [revealLineRequest, setRevealLineRequest] = useState<{
    path: string
    line: number
    seq: number
  } | null>(null)
  const openFileAtLine = useCallback(
    (relPath: string, line: number) => {
      if (!confirmDiscardReviewEdits()) return
      setTerminalActive(false)
      diffRequestRef.current += 1
      setContextDiff(null)
      setDiff(null)
      setTabs((s) => openPinned(s, relPath))
      setRevealLineRequest((previous) => ({
        path: relPath,
        line,
        seq: (previous?.seq ?? 0) + 1,
      }))
    },
    [confirmDiscardReviewEdits],
  )

  const pinFile = useCallback(
    (relPath: string) => {
      const entry = visibleReviewEntriesRef.current.get(relPath)
      if (entry) {
        openReviewEntry(entry)
        return
      }
      if (!confirmDiscardReviewEdits()) return
      setTerminalActive(false)
      diffRequestRef.current += 1
      setContextDiff(null)
      setDiff(null)
      setTabs((s) => openPinned(s, relPath))
    },
    [confirmDiscardReviewEdits, openReviewEntry],
  )

  const revealFolder = useCallback((relPath: string) => {
    setSideTab('files')
    setReveal(relPath)
  }, [])

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
    if (dirty) setTabs((s) => pinTab(s, relPath))
  }, [])

  const onCloseTab = useCallback(
    (relPath: string) => {
      if (!reviewSaveBarrier.canTransition()) return
      if (reviewDirtyPaths.has(relPath)) {
        setTabs((s) => closeTab(s, relPath))
        return
      }
      if (
        dirtyPaths.has(relPath) &&
        !window.confirm(`discard unsaved changes to ${relPath}?`)
      ) {
        return
      }
      cacheRef.current.delete(relPath)
      if (diffRef.current?.change.path === relPath) {
        diffRequestRef.current += 1
        setDiff(null)
      }
      setDirtyPaths((prev) => {
        if (!prev.has(relPath)) return prev
        const next = new Set(prev)
        next.delete(relPath)
        return next
      })
      setTabs((s) => closeTab(s, relPath))
    },
    [dirtyPaths, reviewDirtyPaths, reviewSaveBarrier],
  )

  const changeRoot = useCallback(
    (
      nextRoot: string,
      onResolved?: (outcome: RootChangeOutcome, path?: string) => void,
    ): boolean => {
      if (!reviewSaveBarrier.canTransition()) return false
      const resolveSeq = ++rootResolveSeqRef.current
      void validateRootTarget(
        () => workspaceValidate(host, nextRoot),
        () => rootResolveSeqRef.current === resolveSeq,
      ).then((result) => {
        if (result.outcome !== 'validated') {
          onResolved?.(result.outcome)
          if (result.outcome === 'failed') {
            setRootChangeSettledEpoch((epoch) => epoch + 1)
          }
          return
        }
        if (result.path === rootRef.current) {
          refreshTree()
          void refreshGit()
          onResolved?.('validated', result.path)
          setRootChangeSettledEpoch((epoch) => epoch + 1)
          return
        }
        // Validation can take long enough for a draft or save to begin. Confirm
        // at commit time so the validated transition cannot discard newer work.
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
        reviewEpochRef.current += 1
        scopeMetadataSeqRef.current += 1
        treeSeqRef.current += 1
        gitSeqRef.current += 1
        diffRequestRef.current += 1
        if (liveTimerRef.current !== null)
          window.clearTimeout(liveTimerRef.current)
        liveTimerRef.current = null
        followRef.current = null
        changedAbsRef.current = new Map()
        reviewEligibleAbsRef.current = new Set()
        changedDirsRef.current = new Set()
        subtreeLoadRef.current.clear()
        baselineRef.current.clear()
        baselineKindsRef.current = new Map()
        baselineCompleteRef.current = false
        baselineCapturedRef.current = false
        baselineReadyRef.current = Promise.resolve()
        setBaselineCoverage(null)
        preparedTurnRef.current = null
        reviewEntriesRef.current = new Map()
        reviewEditBackupsRef.current.clear()
        cacheRef.current.clear()
        setDirtyPaths(new Set())
        setTabs(EMPTY_TABS)
        setExpanded([])
        setDiff(null)
        setContextDiff(null)
        setReviewEntries(new Map())
        scopeEntriesRef.current = new Map()
        setScopeEntries(new Map())
        setReviewSummary([])
        setScopeSummary([])
        setScopeCommits([])
        setScopeRefs([])
        setScopeCounts({})
        setTurnOutside(0)
        setTurnOutsideRoot(null)
        setScopeMetadataLoading(false)
        setScopeMetadataError(null)
        followHarnessTurnsRef.current = true
        forceReviewScope(DEFAULT_REVIEW_SCOPE)
        setTree(null)
        setGit(null)
        setSubtrees(new Map())
        if (path === rootRef.current) {
          rootTransitionRef.current = false
          refreshTree()
          void refreshGit()
        } else {
          // Keep event filtering coherent until React renders the new root.
          rootRef.current = path
          setRoot(path)
          rootTransitionRef.current = false
        }
        onResolved?.('validated', path)
        setRootChangeSettledEpoch((epoch) => epoch + 1)
      })
      return true
    },
    [
      confirmDiscardAllEdits,
      forceReviewScope,
      host,
      refreshGit,
      refreshTree,
      reviewSaveBarrier,
    ],
  )

  // ── follow the chat's working directory ──
  // Picking another folder in chat re-roots the explorer (the split-screen
  // sync). A manual root pick sticks until the chat's folder moves again.
  useEffect(() => {
    if (reviewSavePending) return
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
    const accepted = changeRoot(next, (outcome) => {
      if (workingDirFollowPendingRef.current?.request !== request) return
      workingDirFollowPendingRef.current = null
      setPendingRoot(null)
      if (outcome === 'validated') {
        acknowledgedWorkingDirRef.current =
          acknowledgeValidatedWorkingDirectory(
            acknowledgedWorkingDirRef.current,
            next,
            workingDirRef.current,
            true,
          )
        workingDirRetryRef.current = { path: next, failures: 0 }
        setWorkingDirError(null)
      } else if (outcome === 'failed' && workingDirRef.current === next) {
        const retry = workingDirRetryRef.current
        const delay = rootValidationRetryDelay(retry.failures)
        retry.failures += 1
        if (delay !== null && workingDirRetryTimerRef.current === null) {
          workingDirRetryTimerRef.current = window.setTimeout(() => {
            workingDirRetryTimerRef.current = null
            setWorkingDirRetryEpoch((epoch) => epoch + 1)
          }, delay)
        } else if (delay === null) {
          setWorkingDirError(
            workingDirectoryRetryMessage(next, 'failed', delay),
          )
        }
      } else if (outcome === 'declined' && workingDirRef.current === next) {
        setWorkingDirError(workingDirectoryRetryMessage(next, 'declined', null))
      }
    })
    if (!accepted && workingDirFollowPendingRef.current?.request === request) {
      workingDirFollowPendingRef.current = null
      setPendingRoot(null)
    }
  }, [workingDir, root, changeRoot, reviewSavePending, workingDirRetryEpoch])

  useEffect(
    () => () => {
      if (workingDirRetryTimerRef.current !== null) {
        window.clearTimeout(workingDirRetryTimerRef.current)
      }
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
      const accepted = changeRoot(nextRoot, (outcome) => {
        if (!ownsRequestToken(manualRootActiveRequestRef.current, request))
          return
        manualRootActiveRequestRef.current = null
        setPendingRoot(null)
        if (outcome === 'validated' && workingDirRef.current === chatDir) {
          acknowledgedWorkingDirRef.current = chatDir
          workingDirRetryRef.current = { path: chatDir, failures: 0 }
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
    [changeRoot],
  )

  // ── deep link: #/ext/shell/open/<encoded-abs>[:line] ──
  // The chat's "open in shell" lands here. The request is captured (and
  // stripped from the URL) immediately, then applied once the root has
  // resolved — re-rooting to the file's own folder when it lives outside
  // the browsed one; the effect refires on the new root and opens it.
  const pendingOpenRef = useRef<string | null>(null)
  const pendingOpenCaptureSeqRef = useRef(0)
  const pendingOpenRequestSeqRef = useRef(0)
  const pendingOpenRootRequestRef = useRef<{
    target: string
    token: ScopedRequestToken
  } | null>(null)
  const pendingOpenWaitingForRetryRef = useRef(false)
  const pendingOpenRetryRef = useRef(0)
  const pendingOpenRetryTimerRef = useRef<number | null>(null)
  const [pendingOpenError, setPendingOpenError] = useState<string | null>(null)
  const [openBump, setOpenBump] = useState(0)
  useEffect(() => {
    const capture = () => {
      const m = window.location.hash.match(/^#\/ext\/shell\/open\/([^/]+)/)
      if (m === null) return
      window.history.replaceState(
        window.history.state,
        '',
        `${window.location.pathname}${window.location.search}#/ext/shell`,
      )
      const raw = m[1]
      const colon = raw.lastIndexOf(':')
      const encoded =
        colon !== -1 && /^\d+$/.test(raw.slice(colon + 1))
          ? raw.slice(0, colon)
          : raw
      let abs: string
      try {
        abs = decodeURIComponent(encoded)
      } catch {
        return // malformed percent escape — not our link
      }
      if (!abs.startsWith('/')) return
      if (rootRef.current !== null) rootResolveSeqRef.current += 1
      pendingOpenCaptureSeqRef.current += 1
      pendingOpenRef.current = abs
      pendingOpenRootRequestRef.current = null
      pendingOpenWaitingForRetryRef.current = false
      pendingOpenRetryRef.current = 0
      setPendingOpenError(null)
      if (pendingOpenRetryTimerRef.current !== null) {
        window.clearTimeout(pendingOpenRetryTimerRef.current)
        pendingOpenRetryTimerRef.current = null
      }
      setOpenBump((n) => n + 1)
    }
    capture()
    window.addEventListener('hashchange', capture)
    return () => window.removeEventListener('hashchange', capture)
  }, [])
  useEffect(() => {
    const abs = pendingOpenRef.current
    if (abs === null || root === null) return
    if (!reviewSaveBarrier.canTransition()) return
    const prefix = root.endsWith('/') ? root : `${root}/`
    if (abs.startsWith(prefix)) {
      pendingOpenCaptureSeqRef.current += 1
      pendingOpenRef.current = null
      pendingOpenRootRequestRef.current = null
      pendingOpenWaitingForRetryRef.current = false
      pendingOpenRetryRef.current = 0
      setPendingOpenError(null)
      diffRequestRef.current += 1
      setContextDiff(null)
      setDiff(null)
      setTabs((s) => openPinned(s, abs.slice(prefix.length)))
    } else if (abs !== root) {
      const target = deepLinkRootTarget(abs, workingDirRef.current)
      if (
        pendingOpenWaitingForRetryRef.current ||
        pendingOpenRootRequestRef.current?.target === target
      ) {
        return
      }
      const requestToken: ScopedRequestToken = {
        scope: pendingOpenCaptureSeqRef.current,
        request: ++pendingOpenRequestSeqRef.current,
      }
      pendingOpenRootRequestRef.current = { target, token: requestToken }
      const accepted = changeRoot(target, (outcome, validatedRoot) => {
        if (
          !ownsScopedRequestToken(
            pendingOpenCaptureSeqRef.current,
            pendingOpenRootRequestRef.current?.token ?? null,
            requestToken,
          )
        ) {
          return
        }
        pendingOpenRootRequestRef.current = null
        if (outcome === 'validated') {
          pendingOpenWaitingForRetryRef.current = false
          if (validatedRoot !== undefined) {
            pendingOpenRef.current = rebasePathAfterValidation(
              abs,
              target,
              validatedRoot,
            )
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
              if (pendingOpenCaptureSeqRef.current !== requestToken.scope)
                return
              pendingOpenRetryTimerRef.current = null
              pendingOpenWaitingForRetryRef.current = false
              setOpenBump((bump) => bump + 1)
            }, delay)
          } else if (delay === null) {
            pendingOpenWaitingForRetryRef.current = true
            setPendingOpenError(`could not validate the folder for ${abs}`)
          }
        } else if (outcome === 'declined') {
          pendingOpenWaitingForRetryRef.current = true
          setPendingOpenError(`open paused for ${abs}`)
        }
        // A superseding root request will change root or report its own error;
        // the pending absolute path stays intact and is reconsidered afterward.
      })
      if (
        !accepted &&
        ownsScopedRequestToken(
          pendingOpenCaptureSeqRef.current,
          pendingOpenRootRequestRef.current?.token ?? null,
          requestToken,
        )
      ) {
        pendingOpenRootRequestRef.current = null
      }
    }
  }, [
    root,
    openBump,
    changeRoot,
    reviewSavingPaths,
    reviewSaveBarrier,
    rootChangeSettledEpoch,
  ])
  useEffect(
    () => () => {
      if (pendingOpenRetryTimerRef.current !== null) {
        window.clearTimeout(pendingOpenRetryTimerRef.current)
      }
    },
    [],
  )

  const openContextFile = useCallback(
    (path: string): boolean => {
      if (!reviewSaveBarrier.canTransition() || root === null) return false
      setTerminalActive(false)
      setContextDiff(null)
      setSideTab('files')
      setCollapsed(false)

      if (!path.startsWith('/')) {
        diffRequestRef.current += 1
        setDiff(null)
        setTabs((state) => openPinned(state, path))
        return true
      }

      // Reuse the PR's validated deep-link pipeline for contextual panel
      // requests. It safely re-roots when the file lives outside the current
      // workspace and preserves the same retry/error behavior.
      if (rootRef.current !== null) rootResolveSeqRef.current += 1
      pendingOpenCaptureSeqRef.current += 1
      pendingOpenRef.current = path
      pendingOpenRootRequestRef.current = null
      pendingOpenWaitingForRetryRef.current = false
      pendingOpenRetryRef.current = 0
      setPendingOpenError(null)
      if (pendingOpenRetryTimerRef.current !== null) {
        window.clearTimeout(pendingOpenRetryTimerRef.current)
        pendingOpenRetryTimerRef.current = null
      }
      setOpenBump((value) => value + 1)
      return true
    },
    [reviewSaveBarrier, root],
  )

  // The browser card is only honest when that worker is actually on the bus.
  const [browserAvailable, setBrowserAvailable] = useState(false)
  useEffect(() => {
    let cancelled = false
    void host.iii
      .trigger<{ workers?: Array<{ name?: unknown }> }>(
        'engine::workers::list',
        {},
      )
      .then((response) => {
        if (cancelled) return
        const workers = Array.isArray(response?.workers) ? response.workers : []
        setBrowserAvailable(
          workers.some((worker) => worker?.name === 'browser'),
        )
      })
      .catch(() => {
        if (!cancelled) setBrowserAvailable(false)
      })
    return () => {
      cancelled = true
    }
  }, [host])

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
      // so this opens a NEW tab rooted where the run worked — otherwise the
      // pane says pi-demo while the shell inside it sits somewhere else.
      appliedContextRef.current = panelContext.id
      changeRoot(context.cwd)
      const stamp = `${Date.now().toString(36)}`
      dispatchTerminalWorkspace({
        type: 'tab-created',
        tabId: `tab-agent-${stamp}`,
        paneId: `pane-agent-${stamp}`,
        root: context.cwd,
      })
      setTerminalOpen(true)
      setTerminalActive(true)
      return
    }
    if (!confirmDiscardReviewEdits()) return
    appliedContextRef.current = panelContext.id
    setTerminalActive(false)
    setDiff(null)
    setContextDiff({
      eventId: panelContext.id,
      changeId: context.changeId,
      path: context.path,
      canViewFile: context.canViewFile,
    })
  }, [confirmDiscardReviewEdits, openContextFile, panelContext])

  const onSaved = useCallback(() => {
    refreshGit()
  }, [refreshGit])

  const treeGitStatus = useMemo<readonly GitStatusEntry[]>(() => {
    return reviewChanges.map((change) => ({
      path: change.path,
      status: change.status,
    }))
  }, [reviewChanges])

  // Chat-synced roots can be subfolders of a base path — surface the
  // current root as an option so the select never holds a value its
  // options don't contain (and the user can always pop back to a base).
  // "New file" / "New folder" from the Files tree: root-relative path,
  // parents created, a new file opens in the editor right away.
  const createTreeEntry = useCallback(
    async (kind: 'file' | 'folder', rel: string) => {
      const currentRoot = rootRef.current
      if (!currentRoot) return
      const generation = rootGenerationRef.current
      const absPath = joinPath(currentRoot, rel)
      if (kind === 'folder') {
        await shellCreateFolder(host, absPath)
      } else {
        await coderCreateNewFile(host, absPath)
      }
      // A root switch during the write: the entry landed on disk, but the
      // pane now shows another tree — refreshing or pinning would talk to
      // the wrong root.
      if (
        rootGenerationRef.current !== generation ||
        rootRef.current !== currentRoot
      ) {
        return
      }
      refreshTree()
      void refreshGit()
      if (kind === 'file') pinFile(rel)
    },
    [host, refreshTree, refreshGit, pinFile],
  )

  const rootOptions = useMemo(() => {
    if (!info || !root) return []
    const bases = info.base_paths.includes(root)
      ? info.base_paths
      : [root, ...info.base_paths]
    // The console's remembered working directories (the composer's picker
    // list) are as reachable here as a chat-synced root; offer them too.
    const remembered = host.workspace?.recentDirectories() ?? []
    return [...new Set([...bases, ...remembered])]
  }, [info, root, host])

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

  const orderedReviewEntriesRef = useRef(orderedReviewEntries)
  orderedReviewEntriesRef.current = orderedReviewEntries
  const stepReviewEntry = useCallback(
    (delta: 1 | -1) => {
      const entries = orderedReviewEntriesRef.current
      if (entries.length === 0) return
      const current = entries.findIndex(
        (entry) => entry.path === tabsRef.current.active,
      )
      const start = delta === 1 ? 0 : entries.length - 1
      const index =
        current === -1
          ? start
          : (current + delta + entries.length) % entries.length
      openReviewEntry(entries[index])
    },
    [openReviewEntry],
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
              frameEl
                ?.querySelector<HTMLElement>('[data-shell-search-input]')
                ?.focus()
            })
          },
        },
        {
          id: 'files',
          title: 'Show the file tree',
          detail: 'The explorer sidebar',
          keywords: ['explorer', 'tree', 'sidebar'],
          shortcut: 'E',
          run: () => {
            setSideTab('files')
            setCollapsed(false)
          },
        },
        {
          id: 'toggle-sidebar',
          title: 'Toggle the sidebar',
          detail: 'Hide or show the file sidebar',
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
          title: 'Next editor tab',
          keywords: ['tab', 'file', 'cycle'],
          shortcut: 'Alt+ArrowRight',
          enabled: () => tabsRef.current.tabs.length > 1,
          run: () => setTabs((state) => cycleTab(state, 1)),
        },
        {
          id: 'previous-tab',
          title: 'Previous editor tab',
          keywords: ['tab', 'file', 'cycle'],
          shortcut: 'Alt+ArrowLeft',
          enabled: () => tabsRef.current.tabs.length > 1,
          run: () => setTabs((state) => cycleTab(state, -1)),
        },
        {
          id: 'close-tab',
          title: 'Close the editor tab',
          keywords: ['tab', 'file', 'close'],
          shortcut: 'W',
          enabled: () => tabsRef.current.active !== null,
          run: () => {
            const active = tabsRef.current.active
            if (active !== null) onCloseTab(active)
          },
        },
        {
          id: 'review-uncommitted',
          title: 'View uncommitted changes',
          detail: 'All working tree changes since the last commit',
          keywords: ['review', 'diff', 'git', 'working tree'],
          run: () => selectReviewScope(DEFAULT_REVIEW_SCOPE),
        },
        {
          id: 'review-unstaged',
          title: 'View unstaged changes',
          detail: 'Working tree changes not added to the index',
          keywords: ['review', 'diff', 'git', 'working tree'],
          run: () => selectReviewScope({ kind: 'unstaged' }),
        },
        {
          id: 'review-staged',
          title: 'View staged changes',
          detail: 'Changes added to the Git index',
          keywords: ['review', 'diff', 'git', 'index'],
          run: () => selectReviewScope({ kind: 'staged' }),
        },
        {
          id: 'review-last-turn',
          title: 'Follow Harness turn changes',
          detail: 'Current turn while running, then the completed turn',
          keywords: ['review', 'diff', 'activity', 'agent', 'turn'],
          run: () => selectReviewScope(LAST_TURN_SCOPE),
        },
        {
          id: 'next-change',
          title: 'Next changed file',
          detail: 'Open the next file in the review',
          keywords: ['review', 'diff', 'change'],
          shortcut: 'J',
          enabled: () => orderedReviewEntriesRef.current.length > 0,
          run: () => stepReviewEntry(1),
        },
        {
          id: 'previous-change',
          title: 'Previous changed file',
          detail: 'Open the previous file in the review',
          keywords: ['review', 'diff', 'change'],
          shortcut: 'K',
          enabled: () => orderedReviewEntriesRef.current.length > 0,
          run: () => stepReviewEntry(-1),
        },
      ]),
    [
      commands,
      host,
      toggleTerminal,
      onCloseTab,
      selectReviewScope,
      stepReviewEntry,
      frameEl,
    ],
  )

  const header = (
    <PageHeader
      className="shui-page-header"
      icon={<SquareTerminal />}
      title="Shell"
      description={
        root ? (
          rootOptions.length > 1 ? (
            <select
              className="shui-header-root-select"
              value={pendingRoot ?? root}
              onChange={(event) => changeManualRoot(event.target.value)}
              disabled={reviewSavePending}
              aria-label="browsed root"
              title={pendingRoot ?? root}
            >
              {(pendingRoot !== null && !rootOptions.includes(pendingRoot)
                ? [...rootOptions, pendingRoot]
                : rootOptions
              ).map((path) => (
                <option key={path} value={path}>
                  {lastSegments(path)}
                </option>
              ))}
            </select>
          ) : (
            <span title={root}>{lastSegments(root)}</span>
          )
        ) : undefined
      }
      actions={
        info && root ? (
          <div className="shui-page-actions">
            {SIDE_TABS.map(({ id, label, Icon }) => (
              <HoverTip key={id} label={label}>
                <button
                  type="button"
                  className={`shui-side-tab${sideTab === id ? ' active' : ''}`}
                  onClick={() => {
                    setSideTab(id)
                    setCollapsed(false)
                  }}
                  aria-label={label}
                >
                  <Icon aria-hidden className="shui-side-tab-icon" />
                </button>
              </HoverTip>
            ))}
            {sideTab === 'files' && !collapsed ? (
              <HoverTip
                label={
                  showHidden
                    ? 'Hide hidden files (dotfiles)'
                    : 'Show hidden files (dotfiles)'
                }
              >
                <button
                  type="button"
                  className={`shui-side-tab${showHidden ? ' active' : ''}`}
                  onClick={() => setShowHidden((value) => !value)}
                  aria-pressed={showHidden}
                  aria-label={
                    showHidden ? 'Hide hidden files' : 'Show hidden files'
                  }
                >
                  {showHidden ? (
                    <Eye aria-hidden className="shui-side-tab-icon" />
                  ) : (
                    <EyeOff aria-hidden className="shui-side-tab-icon" />
                  )}
                </button>
              </HoverTip>
            ) : null}
            <HoverTip
              label={terminalOpen ? 'Hide terminal' : 'Open terminal (zsh)'}
            >
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
              <HoverTip
                label={
                  collapsed ? 'Show the file sidebar' : 'Hide the file sidebar'
                }
              >
                <button
                  type="button"
                  className="shui-collapse-btn"
                  onClick={() => setCollapsed((value) => !value)}
                  aria-label={
                    collapsed ? 'Show file sidebar' : 'Hide file sidebar'
                  }
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
          shell explorer needs the worker's coder surface — coder::info failed:{' '}
          {infoError}
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

  return (
    <PageShell>
      {header}
      <div
        ref={setFrameEl}
        className={`shui-workspace-frame terminal-${terminalDock}`}
      >
        {narrow && !collapsed ? (
          <button
            type="button"
            className="shui-sidebar-scrim"
            aria-label="Hide file sidebar"
            onClick={() => setCollapsed(true)}
          />
        ) : null}
        <PageBody side={panelSide}>
          <PageSidebar
            label={sideTab === 'files' ? 'Files' : 'Search'}
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
          >
            <div className="shui-side-body">
              {sideTab === 'files' && (pendingRoot !== null || tree === null) ? (
                <div className="shui-side-note">
                  opening {lastSegments(pendingRoot ?? root ?? '')}…
                </div>
              ) : sideTab === 'files' ? (
                <FilesTab
                  tree={reviewTree}
                  gitStatus={treeGitStatus}
                  theme={theme}
                  hiddenFiltered={!showHidden}
                  expanded={expanded}
                  onExpandedChange={setExpanded}
                  reveal={reveal}
                  onRevealed={onRevealed}
                  activePath={diff?.change.path ?? tabs.active}
                  onActivateFile={activateFile}
                  onPinFile={pinFile}
                  onCreate={createTreeEntry}
                />
              ) : (
                <SearchTab
                  host={host}
                  root={root}
                  onPreviewFile={previewFile}
                  onPinFile={pinFile}
                  onRevealFolder={revealFolder}
                />
              )}
            </div>
          </PageSidebar>

          <PageMain>
            {workingDirError ? (
              <div className="shui-review-message warn" role="alert">
                <span>{workingDirError}</span>
                <button
                  type="button"
                  className="shui-review-inline-action"
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
                  retry
                </button>
              </div>
            ) : null}
            {pendingOpenError ? (
              <div className="shui-review-message warn" role="alert">
                <span>{pendingOpenError}</span>
                <button
                  type="button"
                  className="shui-review-inline-action"
                  onClick={() => {
                    pendingOpenRetryRef.current = 0
                    pendingOpenWaitingForRetryRef.current = false
                    setPendingOpenError(null)
                    setOpenBump((bump) => bump + 1)
                  }}
                >
                  retry
                </button>
              </div>
            ) : null}
            {sessionActivity.outside > 0 ? (
              <div className="shui-review-message" role="status">
                <span>
                  {sessionActivity.outside}{' '}
                  {sessionActivity.outside === 1 ? 'file changed' : 'files changed'}
                  {' '}outside this folder
                  {sessionActivity.outsideRoot
                    ? ` in ${sessionActivity.outsideRoot}`
                    : ''}
                </span>
                {sessionActivity.outsideRoot ? (
                  <>
                    <button
                      type="button"
                      className="shui-review-inline-action"
                      onClick={() =>
                        changeManualRoot(sessionActivity.outsideRoot!)
                      }
                    >
                      open in Shell
                    </button>
                    {canUseSessionOutsideForChat ? (
                      <button
                        type="button"
                        className="shui-review-inline-action"
                        onClick={() =>
                          (host as WorkingDirectoryHost).chat?.requestWorkingDirectoryChange?.(
                            {
                              sessionId: conversationId!,
                              path: sessionActivity.outsideRoot!,
                            },
                          )
                        }
                      >
                        use for chat
                      </button>
                    ) : null}
                  </>
                ) : null}
              </div>
            ) : null}
            {!(terminalOpen && terminalDock === 'editor' && terminalActive) ? (
              <div className="shui-review-toolbar">
                {reviewSavePending ? (
                  <span
                    className="shui-review-count"
                    role="status"
                    aria-live="polite"
                  >
                    saving{' '}
                    {reviewSavingPaths.size === 1
                      ? 'review file'
                      : `${reviewSavingPaths.size} review files`}
                    … navigation paused
                  </span>
                ) : null}
                <ReviewScopePicker
                  value={reviewScope}
                  commits={scopeCommits}
                  counts={reviewScopeCounts}
                  currentTurn={observedReview.active}
                  turns={sessionTurns.map((turn) => ({
                    turnId: turn.turn_id,
                    label: turnLabel(turn),
                    fileCount: turn.file_count,
                    active: turn.ended_at == null,
                  }))}
                  branches={scopeRefs.map((ref) => ({
                    ref: ref.fullName,
                    name: ref.name,
                    current: ref.current,
                  }))}
                  metadataLoading={scopeMetadataLoading}
                  metadataError={scopeMetadataError}
                  onOpen={loadScopeMetadata}
                  onChange={selectReviewScope}
                />
                <span className="shui-review-count">
                  {scopeLoading
                    ? 'loading…'
                    : `${orderedReviewEntries.length} ${orderedReviewEntries.length === 1 ? 'file' : 'files'}`}
                </span>
                {scopeError ? (
                  <span className="shui-review-scope-error" title={scopeError}>
                    unavailable
                  </span>
                ) : null}
                {(reviewScope.kind === 'last-turn' ||
                  reviewScope.kind === 'session' ||
                  reviewScope.kind === 'turn') &&
                turnOutside > 0 ? (
                  <span
                    className="shui-review-count"
                    title={`files changed outside the folder you are browsing${turnOutsideRoot ? ` in ${turnOutsideRoot}` : ''}`}
                  >
                    +{turnOutside} outside
                  </span>
                ) : null}
                {reviewScope.kind === 'last-turn' &&
                baselineCoverage?.capped ? (
                  <span
                    className="shui-review-count"
                    title={`This workspace holds ${baselineCoverage.candidates} reviewable files; the pre-turn snapshot captured the ${baselineCoverage.captured} most recently modified. Rows outside it fall back to the last commit, or say so when there is none. Open a narrower folder for full coverage.`}
                  >
                    snapshot {baselineCoverage.captured}/
                    {baselineCoverage.candidates}
                  </span>
                ) : null}
                {reviewTotals.ready > 0 ? (
                  <>
                    <span className="shui-review-total add">
                      +{reviewTotals.add}
                    </span>
                    <span className="shui-review-total del">
                      −{reviewTotals.del}
                    </span>
                  </>
                ) : null}
                {reviewTotals.pending > 0 || reviewTotals.unavailable > 0 ? (
                  <span
                    className="shui-review-total"
                    role="status"
                    title={`${reviewTotals.pending} pending, ${reviewTotals.unavailable} unavailable`}
                    aria-label={`${reviewTotals.pending} change totals pending, ${reviewTotals.unavailable} unavailable`}
                  >
                    …
                  </span>
                ) : null}
                <span className="spacer" />
                <DropdownMenu
                  open={reviewMenuOpen}
                  onOpenChange={setReviewMenuOpen}
                >
                  <DropdownMenuTrigger asChild>
                    <IconButton
                      label="Review options"
                      className={reviewMenuOpen ? 'active' : undefined}
                    >
                      <MoreHorizontal aria-hidden />
                    </IconButton>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent
                    align="end"
                    className="shui-review-menu-content"
                  >
                    <ReviewMenuAction
                      label="Refresh"
                      icon={<RefreshCw />}
                      onSelect={reloadReview}
                    />
                    <ReviewOption
                      label="Enable word wrap"
                      icon={<WrapText />}
                      checked={reviewOptions.wordWrap}
                      onChange={(wordWrap) =>
                        setReviewOptions((value) => ({ ...value, wordWrap }))
                      }
                    />
                    <DropdownMenuSeparator />
                    <ReviewOption
                      label="Load full files"
                      icon={<FileStack />}
                      checked={reviewOptions.expandUnchanged}
                      onChange={(expandUnchanged) =>
                        setReviewOptions((value) => ({
                          ...value,
                          expandUnchanged,
                        }))
                      }
                    />
                    <ReviewOption
                      label="Enable rich preview"
                      icon={<Image />}
                      checked={reviewOptions.richPreview}
                      onChange={(richPreview) =>
                        setReviewOptions((value) => ({
                          ...value,
                          richPreview,
                        }))
                      }
                    />
                    <ReviewOption
                      label="Enable word diffs"
                      icon={<WholeWord />}
                      checked={reviewOptions.wordDiffs}
                      onChange={(wordDiffs) =>
                        setReviewOptions((value) => ({ ...value, wordDiffs }))
                      }
                    />
                    <ReviewOption
                      label="Hide whitespace"
                      icon={<Space />}
                      checked={reviewOptions.hideWhitespace}
                      onChange={(hideWhitespace) =>
                        setReviewOptions((value) => ({
                          ...value,
                          hideWhitespace,
                        }))
                      }
                    />
                    <DropdownMenuSeparator />
                    <ReviewMenuAction
                      label={
                        reviewScope.kind === 'last-turn' ||
                        reviewScope.kind === 'session' ||
                        reviewScope.kind === 'turn'
                          ? 'Copy git apply command (git scopes only)'
                          : 'Copy git apply command'
                      }
                      icon={<ClipboardCopy />}
                      disabled={
                        reviewScope.kind === 'last-turn' ||
                        reviewScope.kind === 'session' ||
                        reviewScope.kind === 'turn' ||
                        copyingPatch
                      }
                      onSelect={() => void copyApplyCommand()}
                    />
                  </DropdownMenuContent>
                </DropdownMenu>
                {orderedReviewEntries.length > 0 ? (
                  <HoverTip label="Jump to file">
                    <div className="shui-review-jump-wrap">
                      <Selector
                        aria-label="Jump to file"
                        className="shui-review-jump"
                        contentClassName="shui-review-jump-list"
                        value={undefined}
                        options={orderedReviewEntries.map((entry) => ({
                          value: entry.path,
                          label: entry.path,
                        }))}
                        placeholder=""
                        searchPlaceholder="Jump to file…"
                        emptyMessage="no matching file"
                        triggerIcon={<FileSearch aria-hidden />}
                        onChange={(path) => {
                          const entry =
                            visibleReviewEntriesRef.current.get(path)
                          if (entry) openReviewEntry(entry)
                        }}
                      />
                    </div>
                  </HoverTip>
                ) : null}
                <IconButton
                  label={
                    reviewAllCollapsed
                      ? 'Expand all diffs'
                      : 'Collapse all diffs'
                  }
                  onClick={toggleAllDiffs}
                >
                  {reviewAllCollapsed ? (
                    <ChevronsUpDown aria-hidden />
                  ) : (
                    <ChevronsDownUp aria-hidden />
                  )}
                </IconButton>
                <IconButton
                  label={
                    reviewOptions.diffStyle === 'unified'
                      ? 'Switch to split diff'
                      : 'Switch to unified diff'
                  }
                  onClick={() =>
                    setReviewOptions((previous) => ({
                      ...previous,
                      diffStyle:
                        previous.diffStyle === 'unified' ? 'split' : 'unified',
                    }))
                  }
                >
                  {reviewOptions.diffStyle === 'unified' ? (
                    <SplitDiffIcon />
                  ) : (
                    <UnifiedDiffIcon />
                  )}
                </IconButton>
              </div>
            ) : null}
            {(diff === null && tabs.tabs.length > 0) ||
            (terminalOpen && terminalDock === 'editor') ? (
              <div className="shui-editor-tabs" role="tablist">
                {tabs.tabs.map((tab) => {
                  const active =
                    !terminalActive &&
                    diff === null &&
                    contextDiff === null &&
                    tab.path === tabs.active
                  return (
                    <div
                      key={tab.path}
                      className={`shui-etab${active ? ' active' : ''}${tab.pinned ? '' : ' preview'}`}
                    >
                      <button
                        type="button"
                        className="open"
                        role="tab"
                        aria-selected={active}
                        title={tab.path}
                        onClick={() =>
                          runReviewTransition(reviewSaveBarrier, () => {
                            setTerminalActive(false)
                            diffRequestRef.current += 1
                            setContextDiff(null)
                            setDiff(null)
                            setTabs((s) => activateTab(s, tab.path))
                          })
                        }
                        onDoubleClick={() =>
                          runReviewTransition(reviewSaveBarrier, () => {
                            setTabs((s) => pinTab(s, tab.path))
                          })
                        }
                      >
                        {basename(tab.path)}
                        {dirtyPaths.has(tab.path) ? (
                          <span
                            className="shui-dirty"
                            title="unsaved changes"
                          />
                        ) : null}
                      </button>
                      <HoverTip label={`Close ${basename(tab.path)}`}>
                        <button
                          type="button"
                          className="close"
                          aria-label={`close ${basename(tab.path)}`}
                          onClick={() => onCloseTab(tab.path)}
                        >
                          <X aria-hidden className="shui-x-icon" />
                        </button>
                      </HoverTip>
                    </div>
                  )
                })}
                {terminalOpen && terminalDock === 'editor' ? (
                  <div
                    className={`shui-etab${terminalActive ? ' active' : ''}`}
                  >
                    <button
                      type="button"
                      className="open"
                      role="tab"
                      aria-selected={terminalActive}
                      onClick={() => setTerminalActive(true)}
                    >
                      <Terminal aria-hidden className="shui-etab-icon" />
                      {terminalWorkspace.tabs.find(
                        (tab) => tab.id === terminalWorkspace.activeTabId,
                      )?.title ?? 'zsh'}
                    </button>
                    <HoverTip label="Close terminal">
                      <button
                        type="button"
                        className="close"
                        aria-label="Close terminal"
                        onClick={closeTerminal}
                      >
                        <X aria-hidden className="shui-x-icon" />
                      </button>
                    </HoverTip>
                  </div>
                ) : null}
              </div>
            ) : null}

            {terminalOpen && terminalDock === 'editor' && terminalActive ? (
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
            ) : contextDiff !== null ? (
              <ChangeDiffPane
                key={contextDiff.eventId}
                host={host}
                changeId={contextDiff.changeId}
                path={contextDiff.path}
                canViewFile={contextDiff.canViewFile}
                onViewFile={openContextFile}
              />
            ) : diff !== null ? (
              <ReviewPane
                host={host}
                root={root}
                entries={orderedReviewEntries}
                activePath={diff.change.path}
                options={reviewOptions}
                collapseEpoch={reviewCollapseEpoch}
                expandEpoch={reviewExpandEpoch}
                refreshEpoch={reviewRefreshEpoch}
                onRequestEdit={onRequestReviewEdit}
                onEditDraftChange={onReviewEditDraftChange}
                onEditDirtyChange={onReviewEditDirtyChange}
                onEditSavingChange={onReviewEditSavingChange}
                onFileSaved={onReviewFileSaved}
                onOpenLine={openFileAtLine}
                onActivate={(path) => {
                  const entry = visibleReviewEntriesRef.current.get(path)
                  if (entry) openReviewEntry(entry)
                }}
                onSummaryChange={
                  reviewScope.kind === 'last-turn'
                    ? setReviewSummary
                    : setScopeSummary
                }
              />
            ) : scopeLoading ? (
              <div className="shui-main-empty">
                <span className="t-ghost">loading review…</span>
              </div>
            ) : scopeError ? (
              <div className="shui-main-empty">
                <span className="t-warn">{scopeError}</span>
              </div>
            ) : scopeEmpty ? (
              <div className="shui-main-empty">
                <span className="t-ghost">
                  No changes in{' '}
                  {reviewScopeLabel(reviewScope, observedReview.active)}
                </span>
              </div>
            ) : currentTurnEmpty ? (
              <div className="shui-main-empty">
                <span className="t-ghost">
                  Changes from this turn will appear here…
                </span>
              </div>
            ) : tabs.active !== null ? (
              <EditorPane
                richPreview={reviewOptions.richPreview}
                reveal={
                  revealLineRequest?.path === tabs.active
                    ? revealLineRequest
                    : null
                }
                // fileBump remounts after an agent-side write to the active
                // file: the pane rehydrates from the refreshed cache entry.
                key={`${tabs.active}:${fileBump}`}
                host={host}
                root={root}
                relPath={tabs.active}
                cache={cacheRef.current}
                onSaved={onSaved}
                onDirtyChange={onDirtyChange}
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
                onOpenFile={pinFile}
              />
            ) : (
              <ShellLauncher
                host={host}
                browserAvailable={browserAvailable}
                // Changes is cumulative working-tree work since HEAD. Last
                // Turn remains available as an explicit review scope.
                onOpenChanges={() => selectReviewScope(DEFAULT_REVIEW_SCOPE)}
                onOpenTerminal={() => {
                  setTerminalOpen(true)
                  setTerminalActive(true)
                }}
                // File opens the workspace browser in this pane and shows the
                // sidebar tree beside it.
                onOpenFiles={() => {
                  setBrowsePath('')
                  setSideTab('files')
                  setCollapsed(false)
                }}
              />
            )}
          </PageMain>
        </PageBody>
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
            size={
              terminalDock === 'bottom' ? terminalBottomSize : terminalRightSize
            }
            onDockChange={changeTerminalDock}
            onSizeChange={
              terminalDock === 'bottom'
                ? setTerminalBottomSize
                : setTerminalRightSize
            }
            onClose={closeTerminal}
          />
        ) : null}
      </div>
    </PageShell>
  )
}
