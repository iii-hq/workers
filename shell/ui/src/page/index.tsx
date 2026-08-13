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
  type Host,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
} from '@iii-dev/console-ui'
import type { GitStatusEntry } from '@pierre/trees'
import {
  Check,
  ChevronsDownUp,
  ChevronsUpDown,
  Columns2,
  Eye,
  EyeOff,
  FolderTree,
  MoreHorizontal,
  RefreshCw,
  Rows3,
  Search,
  SquareTerminal,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { captureWorkspaceBaseline, classifyWorkspaceBaselinePath } from './baseline'
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
  type GitRevisionComparisonEntry,
  type GitRefSummary,
  type GitState,
  gitChanges,
  gitBranchComparison,
  gitCommitComparison,
  gitComparison,
  gitRecentCommits,
  gitRefs,
} from './git'
import { useWorkspaceChanges } from './live'
import { normalizeLiveReviewEvent } from './live-review'
import { createTabUiStateSaver, loadTabUiState, type TabUiState } from './persist'
import { diffForReviewEntry, mergeGitReviewEntries, mergeReviewEntry, type ReviewEntry } from './review'
import { useShellReviewSummaryBridge } from './review-summary-store'
import { changedParentDirs, withReviewChanges } from './review-tree'
import {
  ReviewScopePicker,
  type ReviewScopeSelection,
} from './ReviewScopePicker'
import {
  createReviewSaveBarrier,
  ReviewPane,
  type ReviewEditDraft,
  type ReviewFileSummary,
  type ReviewOptions,
  runReviewTransition,
} from './ReviewPane'
import { SearchTab } from './SearchTab'
import {
  activateTab,
  basename,
  closeTab,
  EMPTY_TABS,
  lastSegments,
  openPinned,
  openPreview,
  pinTab,
  restoreTabs,
  type TabsState,
} from './tabs'
import { useHarnessPreTurn, useHarnessTurn } from './turn'
import {
  canCaptureHarnessWorkspaceChange,
  type HarnessReviewWindow,
} from './turn-status'

type SideTab = 'files' | 'search'

const LAST_TURN_SCOPE: ReviewScopeSelection = { kind: 'last-turn' }

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

function ReviewOption({
  label,
  checked,
  onChange,
}: {
  label: string
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <button
      type="button"
      className="shui-review-option"
      role="menuitemcheckbox"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
    >
      <span>{label}</span>
      <span className="check" aria-hidden>
        {checked ? <Check /> : null}
      </span>
    </button>
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

const SIDEBAR_DEFAULT_WIDTH = 244
const SIDEBAR_MIN_WIDTH = 180
const SIDEBAR_MAX_WIDTH = 560
function reviewablePath(rel: string): boolean {
  // This page's own persisted UI state changes on every tab/expand and
  // must never become part of the user's review set.
  if (rel.endsWith('shell-ui.yaml')) return false
  const noise = ['Library', 'node_modules', 'target', 'dist', 'build', 'out', 'vendor', '__pycache__']
  const segments = rel.split('/')
  if (segments.some((segment) => segment === '.git' || noise.includes(segment))) return false
  return !/\.(o|a|d|rlib|rmeta|so|dylib|dll|class|pyc|wasm|map|log|output|tmp|swp|part|pid|sock)$/.test(rel)
}

function clampSidebarWidth(w: number): number {
  return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, Math.round(w)))
}

const SIDE_TABS: { id: SideTab; label: string; Icon: typeof FolderTree }[] = [
  { id: 'files', label: 'files', Icon: FolderTree },
  { id: 'search', label: 'search', Icon: Search },
]

export function ShellExplorerPage({
  host,
  panelSide,
  tabId,
  onRequestClose,
  workingDir,
  conversationId,
}: { host: Host } & PageRenderProps) {
  const theme = host.useTheme()
  const observedReview = useHarnessTurn(host, conversationId)
  const observedReviewKey = observedReview.turnId
  const [reviewKey, setReviewKey] = useState<string | null>(observedReviewKey)
  const [info, setInfo] = useState<CoderInfo | null>(null)
  const [infoError, setInfoError] = useState<string | null>(null)
  const [restored, setRestored] = useState<TabUiState | null | 'loading'>('loading')
  const [root, setRoot] = useState<string | null>(null)
  const rootRef = useRef(root)
  rootRef.current = root
  const rootGenerationRef = useRef(0)
  const rootResolveSeqRef = useRef(0)
  const rootTransitionRef = useRef(false)
  const [sideTab, setSideTab] = useState<SideTab>('files')
  const [collapsed, setCollapsed] = useState(false)
  const [sideWidth, setSideWidth] = useState(SIDEBAR_DEFAULT_WIDTH)
  const [tree, setTree] = useState<FlatTree | null>(null)
  // Dot entries are filtered by default (Finder/VS Code convention) —
  // in home-shaped folders they otherwise crowd out every visible name.
  const [showHidden, setShowHidden] = useState(false)
  const [git, setGit] = useState<GitState | null>(null)
  // Lazily fetched deep-folder listings, keyed by the folder's rel path.
  // The base tree snapshot is node-budgeted; expanding a folder the
  // snapshot didn't reach fetches its subtree on demand. An entry with
  // no paths marks a fetched-and-empty folder (no refetch loop).
  const [subtrees, setSubtrees] = useState<ReadonlyMap<string, FlatTree>>(new Map())
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
  const [reviewSavingPaths, setReviewSavingPaths] = useState<ReadonlySet<string>>(
    new Set(),
  )
  const reviewSavePending = reviewSavingPaths.size > 0
  const [diff, setDiff] = useState<DiffSelection | null>(null)
  const diffRequestRef = useRef(0)
  const [reviewRefreshEpoch, setReviewRefreshEpoch] = useState(0)
  const [reviewCollapseEpoch, setReviewCollapseEpoch] = useState(0)
  const [reviewExpandEpoch, setReviewExpandEpoch] = useState(0)
  const [reviewMenuOpen, setReviewMenuOpen] = useState(false)
  const [reviewScope, setReviewScope] = useState<ReviewScopeSelection>(LAST_TURN_SCOPE)
  const reviewScopeRef = useRef(reviewScope)
  reviewScopeRef.current = reviewScope
  const [scopeEntries, setScopeEntries] = useState<ReadonlyMap<string, ReviewEntry>>(new Map())
  const scopeEntriesRef = useRef<ReadonlyMap<string, ReviewEntry>>(scopeEntries)
  scopeEntriesRef.current = scopeEntries
  const [scopeSummary, setScopeSummary] = useState<readonly ReviewFileSummary[]>([])
  const [scopeLoading, setScopeLoading] = useState(false)
  const [scopeError, setScopeError] = useState<string | null>(null)
  const [scopeCommits, setScopeCommits] = useState<readonly GitCommitSummary[]>([])
  const [scopeRefs, setScopeRefs] = useState<readonly GitRefSummary[]>([])
  const [scopeMetadataLoading, setScopeMetadataLoading] = useState(false)
  const [scopeMetadataError, setScopeMetadataError] = useState<string | null>(null)
  const scopeLoadSeqRef = useRef(0)
  const scopeMetadataSeqRef = useRef(0)
  const [reviewSummary, setReviewSummary] = useState<readonly ReviewFileSummary[]>([])
  const [reviewOptions, setReviewOptions] = useState<ReviewOptions>({
    diffStyle: 'unified',
    wordWrap: true,
    wordDiffs: true,
    hideWhitespace: false,
    expandUnchanged: false,
    richPreview: false,
  })
  const [reviewEntries, setReviewEntries] = useState<ReadonlyMap<string, ReviewEntry>>(new Map())
  const reviewEntriesRef = useRef<ReadonlyMap<string, ReviewEntry>>(reviewEntries)
  reviewEntriesRef.current = reviewEntries
  // For ordinary non-Git folders, snapshot initial text before Harness
  // writes so every later row can open a real before/after diff.
  const baselineRef = useRef<Map<string, string>>(new Map())
  const baselineKindsRef = useRef<ReadonlyMap<string, TreeNode['kind']>>(new Map())
  const baselineCompleteRef = useRef(false)
  const baselineCapturedRef = useRef(false)
  const baselineReadyRef = useRef<Promise<void>>(Promise.resolve())
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
        if (cached !== undefined && cached.draft !== cached.savedContent) next.add(path)
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
    if (!window.confirm(`discard unsaved changes in ${count} ${count === 1 ? 'file' : 'files'}?`)) {
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

  const forceLastTurnScope = useCallback(() => {
    if (!reviewSaveBarrier.canTransition()) return false
    // Every forced Last Turn transition must retire an in-flight Git scope
    // request before changing the visible scope. Otherwise its late result can
    // replace the file selected from the chat summary or live watcher.
    scopeLoadSeqRef.current += 1
    setReviewScope(LAST_TURN_SCOPE)
    setScopeLoading(false)
    setScopeError(null)
    return true
  }, [reviewSaveBarrier])

  const beginReviewTurn = useCallback((turnId: string) => {
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
    if (liveTimerRef.current !== null) window.clearTimeout(liveTimerRef.current)
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
    reviewEntriesRef.current = new Map()
    setReviewEntries(new Map())
    scopeEntriesRef.current = new Map()
    setScopeEntries(new Map())
    setScopeSummary([])
    setScopeMetadataLoading(false)
    setScopeMetadataError(null)
    forceLastTurnScope()
    setReviewSummary([])
    setDiff(null)
    return true
  }, [forceLastTurnScope, reviewSaveBarrier])

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
      .then(({ contents, kinds, complete }) => {
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
    workspaceValidate(host, requested)
      .then(({ path: next }) => {
        if (cancelled || rootResolveSeqRef.current !== seq) return
        setRoot(next)
        if (restored?.root && requested === restored.root && next === restored.root) {
          setTabs(restoreTabs(restored.open, restored.active))
          setExpanded(restored.expanded)
          setShowHidden(restored.showHidden ?? false)
          setSideWidth(clampSidebarWidth(restored.sideWidth ?? SIDEBAR_DEFAULT_WIDTH))
        } else if (restored && !restored.root && requested === info.primary_root) {
          // Legacy/first save without a root: restore against the primary.
          setTabs(restoreTabs(restored.open, restored.active))
          setExpanded(restored.expanded)
          setShowHidden(restored.showHidden ?? false)
          setSideWidth(clampSidebarWidth(restored.sideWidth ?? SIDEBAR_DEFAULT_WIDTH))
        }
      })
      .catch(() => {
        if (!cancelled && rootResolveSeqRef.current === seq) setRoot(info.primary_root)
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

  const visibleReviewEntries = reviewScope.kind === 'last-turn' ? reviewEntries : scopeEntries
  const visibleReviewEntriesRef = useRef<ReadonlyMap<string, ReviewEntry>>(visibleReviewEntries)
  visibleReviewEntriesRef.current = visibleReviewEntries
  const visibleReviewSummary = reviewScope.kind === 'last-turn' ? reviewSummary : scopeSummary
  const reviewChanges = useMemo<readonly GitChange[]>(
    () => [...visibleReviewEntries.values()].map((entry) => entry.change),
    [visibleReviewEntries],
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
  // The Files tree is also the review navigator. Review-only rows keep
  // deleted files visible even after they disappear from coder::tree.
  const reviewTree = useMemo(() => withReviewChanges(mergedTree, reviewChanges), [mergedTree, reviewChanges])
  const changedDirsKey = useMemo(() => changedParentDirs(reviewChanges).join('\n'), [reviewChanges])
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
          setSubtrees((prev) => new Map(prev).set(dir, { paths: [], kinds: new Map(), truncations: [] }))
        })
        .finally(() => {
          if (rootGenerationRef.current === generation) subtreeLoadRef.current.delete(dir)
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
        if (rootGenerationRef.current !== generation || rootRef.current !== currentRoot) return
        if (tabsRef.current.active !== active) return
        const content = out.content ?? ''
        const entry = cacheRef.current.get(active)
        if (!entry) return
        if (!refreshCleanEditorCacheEntry(entry, content, out.revision ?? undefined)) return
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
  const openReviewEntry = useCallback((entry: ReviewEntry) => {
    if (!reviewSaveBarrier.canTransition()) return false
    diffRequestRef.current += 1
    setTabs((state) => openPreview(state, entry.path))
    setDiff(diffForReviewEntry(entry))
    setReviewRefreshEpoch((value) => value + 1)
    return true
  }, [reviewSaveBarrier])

  const loadScopeMetadata = useCallback(() => {
    if (root === null) return
    const seq = ++scopeMetadataSeqRef.current
    setScopeMetadataLoading(true)
    setScopeMetadataError(null)
    void Promise.all([gitRecentCommits(host, root), gitRefs(host, root)])
      .then(([commits, refs]) => {
        if (scopeMetadataSeqRef.current !== seq || rootRef.current !== root) return
        setScopeCommits(commits.kind === 'ready' ? commits.commits : [])
        setScopeRefs(refs.kind === 'ready' ? refs.refs : [])
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
      .finally(() => {
        if (scopeMetadataSeqRef.current === seq) setScopeMetadataLoading(false)
      })
  }, [host, root])

  const loadReviewScope = useCallback(
    (scope: Exclude<ReviewScopeSelection, { kind: 'last-turn' }>) => {
      if (root === null) return
      const seq = ++scopeLoadSeqRef.current
      setScopeLoading(true)
      setScopeError(null)
      const comparison =
        scope.kind === 'uncommitted' || scope.kind === 'unstaged' || scope.kind === 'staged'
          ? gitComparison(host, root, scope.kind satisfies GitComparisonScope)
          : scope.kind === 'commit'
            ? gitCommitComparison(host, root, scope.sha)
            : gitBranchComparison(host, root, scope.ref)
      void comparison
        .then((state) => {
          if (scopeLoadSeqRef.current !== seq || rootRef.current !== root) return
          if (state.kind !== 'ready') {
            const message = state.kind === 'error' ? state.message : 'not a git repository'
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
          const activePath = diffRef.current?.change.path
          const entry = (activePath ? next.get(activePath) : undefined) ?? next.values().next().value
          if (entry) openReviewEntry(entry)
          else {
            diffRequestRef.current += 1
            setDiff(null)
          }
        })
        .catch((error: unknown) => {
          if (scopeLoadSeqRef.current !== seq || rootRef.current !== root) return
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
    [host, root, openReviewEntry],
  )

  const onReviewEditDirtyChange = useCallback((path: string, dirty: boolean) => {
    // The page-owned draft remains authoritative until the row explicitly
    // sends a clean draft, cancels, or saves.
    if (!dirty) return
    setReviewDirtyPaths((previous) => {
      if (previous.has(path)) return previous
      const next = new Set(previous)
      next.add(path)
      return next
    })
  }, [])

  const onReviewEditSavingChange = useCallback(
    (path: string, saving: boolean) => {
      setReviewSavingPaths(reviewSaveBarrier.update(path, saving))
    },
    [reviewSaveBarrier],
  )

  const onRequestReviewEdit = useCallback((path: string) => {
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
  }, [reviewSaveBarrier])

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
      setScopeSummary([])
      setScopeError(null)
      setReviewMenuOpen(false)
      if (next.kind === 'last-turn') {
        forceLastTurnScope()
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
    [confirmDiscardReviewEdits, forceLastTurnScope, openReviewEntry],
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
        forceLastTurnScope()
        openReviewEntry(entry)
      }
    },
  })

  useWorkspaceChanges(host, root, (event) => {
    if (rootTransitionRef.current) return
    if (event.root !== rootRef.current) return
    const eventAbs = joinPath(event.root, event.path)
    changedAbsRef.current.set(eventAbs, event.kind)
    // Directories refresh the tree but must never open as files —
    // reading one is a C210.
    if (event.dir === true) {
      changedDirsRef.current.add(eventAbs)
    } else {
      const currentRoot = rootRef.current
      if (currentRoot) {
        const prefix = currentRoot.endsWith('/') ? currentRoot : `${currentRoot}/`
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
        if (rootGenerationRef.current !== generation || reviewEpochRef.current !== reviewEpoch) return
        // Capture the pre-refresh tree: watcher kinds are noisy, while this
        // tells an atomic replacement from a truly new path.
        const knownBefore = treeRef.current === null ? null : new Set(treeRef.current.paths)
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

        const prefix = currentRoot.endsWith('/') ? currentRoot : `${currentRoot}/`
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
        ]).then(([results, state]) => {
          if (
            rootGenerationRef.current !== generation ||
            reviewEpochRef.current !== reviewEpoch ||
            rootRef.current !== currentRoot
          ) {
            return
          }
          const existsByPath = new Map(results?.map((result) => [result.path, result.success] as const) ?? [])
          const currentGitChanges = state?.kind === 'ready' ? state.changes : []
          const currentGitByPath = new Map(currentGitChanges.map((change) => [change.path, change] as const))
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
              ? baselinePath?.priorKind ?? null
              : kindsBefore === undefined
                ? undefined
                : kindsBefore.get(rel) === 'file'
                  ? 'file'
                  : kindsBefore.get(rel) === 'dir' || knownBefore?.has(`${rel}/`)
                    ? 'dir'
                    : null
            const baseline = baselineRef.current.get(rel) ?? cacheRef.current.get(rel)?.savedContent
            const baselineUnavailable =
              baselinePath?.priorKind === 'file' && baseline === undefined
            const decision = normalizeLiveReviewEvent({
              path: rel,
              rawKind,
              priorKind,
              priorBaseline: baseline,
              existsNow:
                results === null
                  ? rawKind !== 'deleted'
                  : existsByPath.get(abs) === true,
            })
            if (decision.action === 'ignore-directory' || decision.action === 'ignore-delete') continue
            nextReview = mergeReviewEntry(
              nextReview,
              rel,
              decision.action,
              decision.baseline,
              // An added/untracked Git status normally supplies an empty
              // baseline. Do not use that fallback when an incomplete tree
              // may simply have omitted an existing pre-turn file.
              baselineUnavailable ? undefined : currentGitByPath.get(rel),
            )
          }
          const enriched = mergeGitReviewEntries(nextReview, currentGitChanges, false)
          reviewEntriesRef.current = enriched
          setReviewEntries(enriched)

          if (follow !== null && diffRequestRef.current === followTicket) {
            const entry =
              enriched.get(follow.rel) ??
              [...fileEvents]
                .reverse()
                .map(({ rel }) => enriched.get(rel))
                .find((candidate) => candidate !== undefined)
            if (entry) {
              forceLastTurnScope()
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
            const entry = enriched.get(open.change.path)
            if (entry) setDiff(diffForReviewEntry(entry))
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
      sideWidth,
    })
  }, [saver, root, tabs, expanded, showHidden, sideWidth])

  // ── open/close/pin actions ──
  const previewFile = useCallback((relPath: string) => {
    if (!confirmDiscardReviewEdits()) return
    diffRequestRef.current += 1
    setDiff(null)
    setTabs((s) => openPreview(s, relPath))
  }, [confirmDiscardReviewEdits])

  const activateFile = useCallback(
    (relPath: string) => {
      const entry = visibleReviewEntriesRef.current.get(relPath)
      if (entry) openReviewEntry(entry)
      else previewFile(relPath)
    },
    [openReviewEntry, previewFile],
  )

  const pinFile = useCallback(
    (relPath: string) => {
      const entry = visibleReviewEntriesRef.current.get(relPath)
      if (entry) {
        openReviewEntry(entry)
        return
      }
      if (!confirmDiscardReviewEdits()) return
      diffRequestRef.current += 1
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
      if (dirtyPaths.has(relPath) && !window.confirm(`discard unsaved changes to ${relPath}?`)) {
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

  // ── sidebar resize (drag handle on the boundary toward the main pane) ──
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null)
  const onHandlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLButtonElement>) => {
      dragRef.current = { startX: e.clientX, startWidth: sideWidth }
      // Capture is best-effort: some pointer types refuse it, and the
      // drag still works through the move/up handlers.
      try {
        e.currentTarget.setPointerCapture(e.pointerId)
      } catch {
        // no capture — moves outside the handle end the drag early
      }
    },
    [sideWidth],
  )
  const onHandlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLButtonElement>) => {
      const drag = dragRef.current
      if (!drag) return
      const delta = e.clientX - drag.startX
      // A right-hugging sidebar widens as the handle moves LEFT.
      setSideWidth(clampSidebarWidth(panelSide === 'right' ? drag.startWidth - delta : drag.startWidth + delta))
    },
    [panelSide],
  )
  const onHandlePointerUp = useCallback((e: React.PointerEvent<HTMLButtonElement>) => {
    dragRef.current = null
    try {
      e.currentTarget.releasePointerCapture(e.pointerId)
    } catch {
      // never captured — nothing to release
    }
  }, [])

  const changeRoot = useCallback((nextRoot: string): boolean => {
    if (!confirmDiscardAllEdits()) return false
    const resolveSeq = ++rootResolveSeqRef.current
    rootTransitionRef.current = true
    rootGenerationRef.current += 1
    reviewEpochRef.current += 1
    scopeMetadataSeqRef.current += 1
    treeSeqRef.current += 1
    gitSeqRef.current += 1
    diffRequestRef.current += 1
    if (liveTimerRef.current !== null) window.clearTimeout(liveTimerRef.current)
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
    preparedTurnRef.current = null
    reviewEntriesRef.current = new Map()
    reviewEditBackupsRef.current.clear()
    cacheRef.current.clear()
    setDirtyPaths(new Set())
    setTabs(EMPTY_TABS)
    setExpanded([])
    setDiff(null)
    setReviewEntries(new Map())
    scopeEntriesRef.current = new Map()
    setScopeEntries(new Map())
    setReviewSummary([])
    setScopeSummary([])
    setScopeCommits([])
    setScopeRefs([])
    setScopeMetadataLoading(false)
    setScopeMetadataError(null)
    forceLastTurnScope()
    setTree(null)
    setGit(null)
    setSubtrees(new Map())
    workspaceValidate(host, nextRoot)
      .then(({ path }) => {
        if (rootResolveSeqRef.current === resolveSeq) {
          rootTransitionRef.current = false
          if (path === rootRef.current) {
            refreshTree()
            void refreshGit()
          } else {
            setRoot(path)
          }
        }
      })
      .catch(() => {
        if (rootResolveSeqRef.current === resolveSeq) {
          rootTransitionRef.current = false
          if (nextRoot === rootRef.current) {
            refreshTree()
            void refreshGit()
          } else {
            setRoot(nextRoot)
          }
        }
      })
    return true
  }, [confirmDiscardAllEdits, forceLastTurnScope, host, refreshGit, refreshTree])

  // ── follow the chat's working directory ──
  // Picking another folder in chat re-roots the explorer (the split-screen
  // sync). A manual root pick sticks until the chat's folder moves again.
  const lastWorkingDirRef = useRef(workingDir ?? null)
  useEffect(() => {
    if (reviewSavePending) return
    const next = workingDir ?? null
    if (next === lastWorkingDirRef.current) return
    if (next === null || root === null || next === root) {
      lastWorkingDirRef.current = next
      return
    }
    // A cancelled discard prompt must not acknowledge the chat directory:
    // keeping the previous value lets the effect retry after edits are saved
    // or discarded instead of permanently breaking chat-to-Shell following.
    if (changeRoot(next)) lastWorkingDirRef.current = next
  }, [workingDir, root, changeRoot, reviewSavePending])

  // ── deep link: #/ext/shell/open/<encoded-abs>[:line] ──
  // The chat's "open in shell" lands here. The request is captured (and
  // stripped from the URL) immediately, then applied once the root has
  // resolved — re-rooting to the file's own folder when it lives outside
  // the browsed one; the effect refires on the new root and opens it.
  const pendingOpenRef = useRef<string | null>(null)
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
      const encoded = colon !== -1 && /^\d+$/.test(raw.slice(colon + 1)) ? raw.slice(0, colon) : raw
      let abs: string
      try {
        abs = decodeURIComponent(encoded)
      } catch {
        return // malformed percent escape — not our link
      }
      if (!abs.startsWith('/')) return
      pendingOpenRef.current = abs
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
      pendingOpenRef.current = null
      diffRequestRef.current += 1
      setDiff(null)
      setTabs((s) => openPinned(s, abs.slice(prefix.length)))
    } else if (abs !== root) {
      const cut = abs.lastIndexOf('/')
      changeRoot(cut > 0 ? abs.slice(0, cut) : '/')
    }
  }, [root, openBump, changeRoot, reviewSavingPaths, reviewSaveBarrier])

  const onSaved = useCallback(() => {
    refreshGit()
  }, [refreshGit])

  const treeGitStatus = useMemo<readonly GitStatusEntry[]>(() => {
    return reviewChanges.map((change) => ({ path: change.path, status: change.status }))
  }, [reviewChanges])

  // Chat-synced roots can be subfolders of a base path — surface the
  // current root as an option so the select never holds a value its
  // options don't contain (and the user can always pop back to a base).
  const rootOptions = useMemo(() => {
    if (!info || !root) return []
    return info.base_paths.includes(root) ? info.base_paths : [root, ...info.base_paths]
  }, [info, root])

  const header = (
    <PageHeader
      icon={<SquareTerminal />}
      title="shell"
      description={
        root ? (
          rootOptions.length > 1 ? (
            <select
              className="shui-header-root-select"
              value={root}
              onChange={(event) => changeRoot(event.target.value)}
              disabled={reviewSavePending}
              aria-label="browsed root"
              title={root}
            >
              {rootOptions.map((path) => (
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
              <button
                key={id}
                type="button"
                className={`shui-side-tab${sideTab === id ? ' active' : ''}`}
                onClick={() => {
                  setSideTab(id)
                  setCollapsed(false)
                }}
                aria-label={label}
                title={label}
              >
                <Icon aria-hidden className="shui-side-tab-icon" />
              </button>
            ))}
            {sideTab === 'files' && !collapsed ? (
              <button
                type="button"
                className={`shui-side-tab${showHidden ? ' active' : ''}`}
                onClick={() => setShowHidden((value) => !value)}
                aria-pressed={showHidden}
                aria-label={showHidden ? 'hide hidden files' : 'show hidden files'}
                title={showHidden ? 'hide hidden files' : 'show hidden files'}
              >
                {showHidden ? (
                  <Eye aria-hidden className="shui-side-tab-icon" />
                ) : (
                  <EyeOff aria-hidden className="shui-side-tab-icon" />
                )}
              </button>
            ) : null}
            <button
              type="button"
              className="shui-collapse-btn"
              onClick={() => setCollapsed((value) => !value)}
              aria-label={collapsed ? 'show explorer' : 'hide explorer'}
              title={collapsed ? 'show explorer' : 'hide explorer'}
            >
              {panelSide === 'right' ? (collapsed ? '‹' : '›') : collapsed ? '›' : '‹'}
            </button>
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

  return (
    <PageShell>
      {header}
      <PageBody side={panelSide}>
        {!collapsed ? (
          <PageSidebar width={sideWidth} className="shui-sidebar">
            <button
              type="button"
              className={`shui-resize-handle ${panelSide === 'right' ? 'left' : 'right'}`}
              onPointerDown={onHandlePointerDown}
              onPointerMove={onHandlePointerMove}
              onPointerUp={onHandlePointerUp}
              onPointerCancel={onHandlePointerUp}
              onLostPointerCapture={onHandlePointerUp}
              onKeyDown={(event) => {
                if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
                event.preventDefault()
                const inward = panelSide === 'right' ? 'ArrowLeft' : 'ArrowRight'
                setSideWidth((width) => clampSidebarWidth(width + (event.key === inward ? 10 : -10)))
              }}
              aria-label="resize sidebar"
              title="drag to resize"
            />
            <div className="shui-side-body">
              {sideTab === 'files' ? (
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
        ) : null}

        <PageMain>
            <div className="shui-review-toolbar">
              {reviewSavePending ? (
                <span
                  className="shui-review-count"
                  role="status"
                  aria-live="polite"
                >
                  saving {reviewSavingPaths.size === 1 ? 'review file' : `${reviewSavingPaths.size} review files`}… navigation paused
                </span>
              ) : null}
              <ReviewScopePicker
                value={reviewScope}
                commits={scopeCommits}
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
                <span className="shui-review-scope-error" title={scopeError}>unavailable</span>
              ) : null}
              {reviewTotals.ready > 0 ? (
                <>
                  <span className="shui-review-total add">+{reviewTotals.add}</span>
                  <span className="shui-review-total del">−{reviewTotals.del}</span>
                </>
              ) : null}
              {reviewTotals.pending > 0 || reviewTotals.unavailable > 0 ? (
                <span
                  className="shui-review-total"
                  title={`${reviewTotals.pending} pending, ${reviewTotals.unavailable} unavailable`}
                  aria-label={`${reviewTotals.pending} change totals pending, ${reviewTotals.unavailable} unavailable`}
                >
                  …
                </span>
              ) : null}
              <span className="spacer" />
              <button
                type="button"
                className="shui-review-action"
                onClick={() => {
                  refreshTree()
                  void refreshGit()
                  if (reviewScope.kind === 'last-turn') {
                    setReviewRefreshEpoch((value) => value + 1)
                  } else {
                    loadReviewScope(reviewScope)
                  }
                }}
                aria-label="refresh review"
                title="refresh"
              >
                <RefreshCw aria-hidden />
              </button>
              <button
                type="button"
                className="shui-review-action"
                onClick={() => setReviewExpandEpoch((value) => value + 1)}
                aria-label="expand all diffs"
                title="expand all diffs"
              >
                <ChevronsUpDown aria-hidden />
              </button>
              <button
                type="button"
                className="shui-review-action"
                onClick={() => setReviewCollapseEpoch((value) => value + 1)}
                aria-label="collapse all diffs"
                title="collapse all diffs"
              >
                <ChevronsDownUp aria-hidden />
              </button>
              <button
                type="button"
                className="shui-review-action"
                onClick={() =>
                  setReviewOptions((previous) => ({
                    ...previous,
                    diffStyle: previous.diffStyle === 'unified' ? 'split' : 'unified',
                  }))
                }
                aria-label={`switch to ${reviewOptions.diffStyle === 'unified' ? 'split' : 'unified'} diff`}
                title={`switch to ${reviewOptions.diffStyle === 'unified' ? 'split' : 'unified'} diff`}
              >
                {reviewOptions.diffStyle === 'unified' ? <Columns2 aria-hidden /> : <Rows3 aria-hidden />}
              </button>
              <div className="shui-review-menu-wrap">
                <button
                  type="button"
                  className={`shui-review-action${reviewMenuOpen ? ' active' : ''}`}
                  onClick={() => setReviewMenuOpen((value) => !value)}
                  aria-expanded={reviewMenuOpen}
                  aria-label="review options"
                  title="review options"
                >
                  <MoreHorizontal aria-hidden />
                </button>
                {reviewMenuOpen ? (
                  <div className="shui-review-menu" role="menu">
                    <ReviewOption
                      label="Enable word wrap"
                      checked={reviewOptions.wordWrap}
                      onChange={(wordWrap) => setReviewOptions((value) => ({ ...value, wordWrap }))}
                    />
                    <ReviewOption
                      label="Enable word diffs"
                      checked={reviewOptions.wordDiffs}
                      onChange={(wordDiffs) => setReviewOptions((value) => ({ ...value, wordDiffs }))}
                    />
                    <ReviewOption
                      label="Hide whitespace"
                      checked={reviewOptions.hideWhitespace}
                      onChange={(hideWhitespace) =>
                        setReviewOptions((value) => ({ ...value, hideWhitespace }))
                      }
                    />
                    <ReviewOption
                      label="Load full files"
                      checked={reviewOptions.expandUnchanged}
                      onChange={(expandUnchanged) =>
                        setReviewOptions((value) => ({ ...value, expandUnchanged }))
                      }
                    />
                    <ReviewOption
                      label="Enable rich preview"
                      checked={reviewOptions.richPreview}
                      onChange={(richPreview) =>
                        setReviewOptions((value) => ({ ...value, richPreview }))
                      }
                    />
                  </div>
                ) : null}
              </div>
            </div>
          {diff === null && tabs.tabs.length > 0 ? (
            <div className="shui-editor-tabs" role="tablist">
              {tabs.tabs.map((tab) => {
                const active = diff === null && tab.path === tabs.active
                return (
                  <div key={tab.path} className={`shui-etab${active ? ' active' : ''}${tab.pinned ? '' : ' preview'}`}>
                    <button
                      type="button"
                      className="open"
                      role="tab"
                      aria-selected={active}
                      title={tab.path}
                      onClick={() =>
                        runReviewTransition(reviewSaveBarrier, () => {
                          diffRequestRef.current += 1
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
                      {dirtyPaths.has(tab.path) ? <span className="shui-dirty" title="unsaved changes" /> : null}
                    </button>
                    <button
                      type="button"
                      className="close"
                      aria-label={`close ${basename(tab.path)}`}
                      title="close"
                      onClick={() => onCloseTab(tab.path)}
                    >
                      <X aria-hidden className="shui-x-icon" />
                    </button>
                  </div>
                )
              })}
            </div>
          ) : null}

          {diff !== null ? (
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
              onActivate={(path) => {
                const entry = visibleReviewEntriesRef.current.get(path)
                if (entry) openReviewEntry(entry)
              }}
              onSummaryChange={reviewScope.kind === 'last-turn' ? setReviewSummary : setScopeSummary}
            />
          ) : scopeLoading ? (
            <div className="shui-main-empty">
              <span className="t-ghost">loading review…</span>
            </div>
          ) : scopeError ? (
            <div className="shui-main-empty">
              <span className="t-warn">{scopeError}</span>
            </div>
          ) : tabs.active !== null ? (
            <EditorPane
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
          ) : (
            <div className="shui-main-empty">
              <span className="t-ghost">select a file to edit or review</span>
            </div>
          )}
        </PageMain>
      </PageBody>
    </PageShell>
  )
}
