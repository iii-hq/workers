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
import { Eye, EyeOff, FolderTree, GitBranch, History, Search, SquareTerminal, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { type CoderInfo, coderInfo, coderReadFile, coderTree, type FlatTree, flattenTree, joinPath } from './coder'
import { DiffPane } from './DiffPane'
import { type EditorCache, EditorPane } from './EditorPane'
import { useWorkspaceChanges } from './live'
import { FilesTab } from './FilesTab'
import { GitTab } from './GitTab'
import { type GitChange, type GitState, gitChanges, nestedGitStatus } from './git'
import { createTabUiStateSaver, loadTabUiState, type TabUiState } from './persist'
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

type SideTab = 'files' | 'git' | 'search' | 'changes'

interface DiffSelection {
  /** The change shown — from git status, or synthesized for live
      follows in folders that aren't a repo. */
  change: GitChange
  /** Overrides the git-HEAD baseline: the last content this page saw,
      so modified files outside a repo still diff instead of dumping. */
  baseline?: string
  /** The file's own repo directory when the browsed root sits above it
      (a worktree under the home directory) — see DiffPane. */
  gitDir?: string
}

/** One live event the changes tab keeps for review — the auto-follow is
    last-write-wins and moves fast; this list doesn't. */
interface FeedEntry {
  rel: string
  kind: string
  at: number
}

function timeAgo(at: number): string {
  const s = Math.max(0, Math.round((Date.now() - at) / 1000))
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m`
  return `${Math.floor(s / 3600)}h`
}

const SIDEBAR_DEFAULT_WIDTH = 280
const SIDEBAR_MIN_WIDTH = 180
const SIDEBAR_MAX_WIDTH = 560

function clampSidebarWidth(w: number): number {
  return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, Math.round(w)))
}

const SIDE_TABS: { id: SideTab; label: string; Icon: typeof FolderTree }[] = [
  { id: 'files', label: 'files', Icon: FolderTree },
  { id: 'git', label: 'git', Icon: GitBranch },
  { id: 'search', label: 'search', Icon: Search },
  { id: 'changes', label: 'changes', Icon: History },
]

export function ShellExplorerPage({
  host,
  panelSide,
  tabId,
  onRequestClose,
  workingDir,
}: { host: Host } & PageRenderProps) {
  const theme = host.useTheme()
  const [info, setInfo] = useState<CoderInfo | null>(null)
  const [infoError, setInfoError] = useState<string | null>(null)
  const [restored, setRestored] = useState<TabUiState | null | 'loading'>('loading')
  const [root, setRoot] = useState<string | null>(null)
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
  const [feed, setFeed] = useState<FeedEntry[]>([])
  const [tabs, setTabs] = useState<TabsState>(EMPTY_TABS)
  const [expanded, setExpanded] = useState<string[]>([])
  const [reveal, setReveal] = useState<string | null>(null)
  const [dirtyPaths, setDirtyPaths] = useState<ReadonlySet<string>>(new Set())
  const [diff, setDiff] = useState<DiffSelection | null>(null)
  const cacheRef = useRef<EditorCache>(new Map())

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

  // Root resolution waits for BOTH: a persisted root only counts while it
  // still lives inside an allowed base path (chat-synced roots may be
  // subfolders, not just the base paths themselves). Without one, the
  // chat's working dir wins over the primary root, so a fresh split opens
  // where the conversation is.
  useEffect(() => {
    if (!info || restored === 'loading' || root !== null) return
    const withinBase = (p: string) =>
      info.base_paths.includes(p) || info.base_paths.some((base) => p.startsWith(`${base}/`))
    const persisted = restored?.root && withinBase(restored.root) ? restored.root : null
    const next = persisted ?? workingDir ?? info.primary_root
    setRoot(next)
    if (persisted) {
      setTabs(restoreTabs(restored?.open, restored?.active))
      setExpanded(restored?.expanded ?? [])
      setShowHidden(restored?.showHidden ?? false)
      setSideWidth(clampSidebarWidth(restored?.sideWidth ?? SIDEBAR_DEFAULT_WIDTH))
    } else if (restored && !restored.root && next === info.primary_root) {
      // Legacy/first save without a root: restore against the primary.
      setTabs(restoreTabs(restored.open, restored.active))
      setExpanded(restored.expanded)
      setShowHidden(restored.showHidden ?? false)
      setSideWidth(clampSidebarWidth(restored.sideWidth ?? SIDEBAR_DEFAULT_WIDTH))
    }
  }, [info, restored, root, workingDir])

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
    setSubtrees(new Map())
    setFeed([])
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

  // Expanding a folder the snapshot didn't reach fetches its listing.
  const subtreeLoadRef = useRef<Set<string>>(new Set())
  useEffect(() => {
    if (root === null || mergedTree === null) return
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
          setSubtrees((prev) => new Map(prev).set(dir, flattenTree(out.root)))
        })
        .catch(() => {
          // Inaccessible folder — leave it childless; the next expand
          // after a live change under it retries.
        })
        .finally(() => subtreeLoadRef.current.delete(dir))
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
  const rootRef = useRef(root)
  rootRef.current = root
  const tabsRef = useRef(tabs)
  tabsRef.current = tabs
  const liveTimerRef = useRef<number | null>(null)
  const changedAbsRef = useRef<Map<string, string>>(new Map())

  const reloadActiveFile = useCallback(() => {
    const currentRoot = rootRef.current
    const active = tabsRef.current.active
    if (!currentRoot || !active) return
    const absPath = joinPath(currentRoot, active)
    if (!changedAbsRef.current.has(absPath)) return
    coderReadFile(host, absPath)
      .then((out) => {
        if (tabsRef.current.active !== active) return
        const content = out.content ?? ''
        const entry = cacheRef.current.get(active)
        if (!entry) return
        if (entry.savedContent === content) return
        const wasClean = entry.draft === entry.savedContent
        entry.savedContent = content
        if (wasClean) {
          entry.draft = content
          setFileBump((n) => n + 1)
        }
      })
      .catch(() => {
        // A deleted-then-read race resolves through the next tree refresh.
      })
  }, [host])

  // The last written file in a burst follows the writer into view — as a
  // DIFF, the way the change reads best: git baseline in a repo, empty
  // baseline for a created file, the last content this page saw for a
  // modified one. The file also opens as the PREVIEW tab underneath —
  // non-destructive by construction: preview replacement never touches
  // pinned tabs, and a dirty preview auto-pins on edit. A system-level
  // watch sees AMBIENT writes too (session logs, caches, build output),
  // so only visible, non-system, non-artifact paths ever steal the view.
  const followRef = useRef<{ rel: string; kind: string } | null>(null)
  const followable = (rel: string): boolean => {
    const noise = ['Library', 'node_modules', 'target', 'dist', 'build', 'out', 'vendor', '__pycache__']
    const segments = rel.split('/')
    if (segments.some((s) => s.startsWith('.') || noise.includes(s))) return false
    return !/\.(o|a|d|rlib|rmeta|so|dylib|dll|class|pyc|wasm|map|lock|log|output|tmp|swp|part|pid|sock)$/.test(rel)
  }
  const diffRef = useRef(diff)
  diffRef.current = diff
  const treeRef = useRef(tree)
  treeRef.current = tree

  // Open one change for review: preview tab plus the best diff this file
  // supports — the browsed root's git status first, then the file's OWN
  // repo (`git -C` auto-discovers upward, covering a worktree under the
  // home directory), then created/last-seen baselines, then plain
  // content. Used by both the auto-follow and the changes tab.
  const openChangeDiff = useCallback(
    (rel: string, kindHint: string) => {
      const currentRoot = rootRef.current
      if (currentRoot === null) return
      const entry = cacheRef.current.get(rel)
      setTabs((s) => openPreview(s, rel))
      void gitChanges(host, currentRoot)
        .then(async (state): Promise<DiffSelection | null> => {
          const changes = state.kind === 'ready' ? state.changes : []
          const fromGit = changes.find((c) => c.path === rel)
          if (fromGit) return { change: fromGit }
          const abs = joinPath(currentRoot, rel)
          const cut = abs.lastIndexOf('/')
          const dir = abs.slice(0, cut)
          const nested = await nestedGitStatus(host, dir, abs.slice(cut + 1))
          if (nested === 'clean') return null
          if (nested !== null) {
            return { change: { path: rel, status: nested, staged: false }, gitDir: dir }
          }
          if (kindHint === 'created') {
            return { change: { path: rel, status: 'untracked', staged: false } }
          }
          if (entry !== undefined) {
            return {
              change: { path: rel, status: 'modified', staged: false },
              baseline: entry.savedContent,
            }
          }
          return null
        })
        .then((sel) => {
          if (rootRef.current === currentRoot) setDiff(sel)
        })
        .catch(() => {
          if (rootRef.current === currentRoot) setDiff(null)
        })
    },
    [host],
  )

  useWorkspaceChanges(host, root, (event) => {
    changedAbsRef.current.set(joinPath(event.root, event.path), event.kind)
    if (event.kind !== 'deleted') {
      const currentRoot = rootRef.current
      if (currentRoot) {
        const abs = joinPath(event.root, event.path)
        const prefix = currentRoot.endsWith('/') ? currentRoot : `${currentRoot}/`
        if (abs.startsWith(prefix)) {
          const rel = abs.slice(prefix.length)
          if (followable(rel)) followRef.current = { rel, kind: event.kind }
        }
      }
    }
    if (liveTimerRef.current !== null) return
    liveTimerRef.current = window.setTimeout(() => {
      liveTimerRef.current = null
      // Captured before the refresh: "was this file known?" decides
      // created-vs-modified when the OS event kind is unreliable (macOS
      // reports the create and the write that fills it separately).
      const knownBefore = treeRef.current === null ? null : new Set(treeRef.current.paths)
      refreshTree()
      const gitLoad = refreshGit()
      reloadActiveFile()
      const follow = followRef.current
      followRef.current = null
      const changed = changedAbsRef.current
      changedAbsRef.current = new Map()
      const currentRoot = rootRef.current
      if (currentRoot !== null) {
        const prefix = currentRoot.endsWith('/') ? currentRoot : `${currentRoot}/`
        // Every visible change lands in the review feed — the auto-follow
        // is last-write-wins; this list is where a burst stays legible.
        const entries: FeedEntry[] = []
        for (const [abs, kind] of changed) {
          if (!abs.startsWith(prefix)) continue
          const rel = abs.slice(prefix.length)
          if (followable(rel)) entries.push({ rel, kind, at: Date.now() })
        }
        if (entries.length > 0) {
          setFeed((prev) => {
            const fresh = new Set(entries.map((e) => e.rel))
            return [...entries, ...prev.filter((e) => !fresh.has(e.rel))].slice(0, 200)
          })
        }
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
      }
      if (follow !== null && follow.rel !== tabsRef.current.active) {
        const looksCreated =
          follow.kind === 'created' || (knownBefore !== null && !knownBefore.has(follow.rel))
        openChangeDiff(follow.rel, looksCreated ? 'created' : follow.kind)
      } else {
        void gitLoad.then((state) => {
          const changes = state?.kind === 'ready' ? state.changes : []
          if (diffRef.current === null || currentRoot === null) return
          // The open diff tracks further writes to its file live.
          const open = diffRef.current
          if (changed.has(joinPath(currentRoot, open.change.path))) {
            const fresh = changes.find((c) => c.path === open.change.path)
            setDiff(
              fresh
                ? { change: fresh }
                : { change: { ...open.change }, baseline: open.baseline, gitDir: open.gitDir },
            )
          }
        })
      }
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
    setDiff(null)
    setTabs((s) => openPreview(s, relPath))
  }, [])

  const pinFile = useCallback((relPath: string) => {
    setDiff(null)
    setTabs((s) => openPinned(s, relPath))
  }, [])

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
      if (dirtyPaths.has(relPath) && !window.confirm(`discard unsaved changes to ${relPath}?`)) {
        return
      }
      cacheRef.current.delete(relPath)
      setDirtyPaths((prev) => {
        if (!prev.has(relPath)) return prev
        const next = new Set(prev)
        next.delete(relPath)
        return next
      })
      setTabs((s) => closeTab(s, relPath))
    },
    [dirtyPaths],
  )

  // ── sidebar resize (drag handle on the boundary toward the main pane) ──
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null)
  const onHandlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
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
    (e: React.PointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current
      if (!drag) return
      const delta = e.clientX - drag.startX
      // A right-hugging sidebar widens as the handle moves LEFT.
      setSideWidth(
        clampSidebarWidth(
          panelSide === 'right' ? drag.startWidth - delta : drag.startWidth + delta,
        ),
      )
    },
    [panelSide],
  )
  const onHandlePointerUp = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    dragRef.current = null
    try {
      e.currentTarget.releasePointerCapture(e.pointerId)
    } catch {
      // never captured — nothing to release
    }
  }, [])

  const changeRoot = useCallback((nextRoot: string) => {
    cacheRef.current.clear()
    setDirtyPaths(new Set())
    setTabs(EMPTY_TABS)
    setExpanded([])
    setDiff(null)
    setRoot(nextRoot)
  }, [])

  // ── follow the chat's working directory ──
  // Picking another folder in chat re-roots the explorer (the split-screen
  // sync). Only CHANGES sync: the boot-resolved root wins on mount, and a
  // manual root pick sticks until the chat's folder moves again.
  const lastWorkingDirRef = useRef(workingDir ?? null)
  useEffect(() => {
    const next = workingDir ?? null
    if (next === lastWorkingDirRef.current) return
    lastWorkingDirRef.current = next
    if (next !== null && root !== null && next !== root) changeRoot(next)
  }, [workingDir, root, changeRoot])

  const onSaved = useCallback(() => {
    refreshGit()
  }, [refreshGit])

  const treeGitStatus = useMemo<readonly GitStatusEntry[]>(() => {
    if (git?.kind !== 'ready') return []
    return git.changes.map((c) => ({ path: c.path, status: c.status }))
  }, [git])

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
      description={root ? <span title={root}>{root}</span> : undefined}
      onClose={onRequestClose}
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
        <PageSidebar
          width={collapsed ? 34 : sideWidth}
          className={`shui-sidebar${collapsed ? ' collapsed' : ''}`}
        >
          {!collapsed ? (
            <div
              className={`shui-resize-handle ${panelSide === 'right' ? 'left' : 'right'}`}
              onPointerDown={onHandlePointerDown}
              onPointerMove={onHandlePointerMove}
              onPointerUp={onHandlePointerUp}
              role="separator"
              aria-orientation="vertical"
              aria-label="resize sidebar"
              title="drag to resize"
            />
          ) : null}
          {collapsed ? (
            <button
              type="button"
              className="shui-collapse-btn"
              onClick={() => setCollapsed(false)}
              aria-label="expand sidebar"
              title="expand sidebar"
            >
              {panelSide === 'right' ? '‹' : '›'}
            </button>
          ) : (
            <>
              <div className="shui-side-tabs">
                {SIDE_TABS.map(({ id, label, Icon }) => (
                  <button
                    key={id}
                    type="button"
                    className={`shui-side-tab${sideTab === id ? ' active' : ''}`}
                    onClick={() => setSideTab(id)}
                    aria-label={label}
                    title={label}
                  >
                    <Icon aria-hidden className="shui-side-tab-icon" />
                  </button>
                ))}
                <span className="spacer" />
                {sideTab === 'files' ? (
                  <button
                    type="button"
                    className={`shui-side-tab${showHidden ? ' active' : ''}`}
                    onClick={() => setShowHidden((v) => !v)}
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
                  onClick={() => setCollapsed(true)}
                  aria-label="collapse sidebar"
                  title="collapse sidebar"
                >
                  {panelSide === 'right' ? '›' : '‹'}
                </button>
              </div>

              {rootOptions.length > 1 ? (
                <select
                  className="shui-root-select"
                  value={root}
                  onChange={(e) => changeRoot(e.target.value)}
                  aria-label="browsed root"
                  title={root}
                >
                  {rootOptions.map((p) => (
                    <option key={p} value={p}>
                      {lastSegments(p)}
                    </option>
                  ))}
                </select>
              ) : (
                <div className="shui-root-label" title={root}>
                  {lastSegments(root)}
                </div>
              )}

              <div className="shui-side-body">
                {sideTab === 'files' ? (
                  <FilesTab
                    tree={mergedTree}
                    gitStatus={treeGitStatus}
                    theme={theme}
                    hiddenFiltered={!showHidden}
                    expanded={expanded}
                    onExpandedChange={setExpanded}
                    reveal={reveal}
                    onRevealed={onRevealed}
                    onPreviewFile={previewFile}
                    onPinFile={pinFile}
                  />
                ) : sideTab === 'git' ? (
                  <GitTab state={git} theme={theme} onSelect={(change) => setDiff({ change })} onRefresh={refreshGit} />
                ) : sideTab === 'changes' ? (
                  <div className="shui-feed">
                    {feed.length === 0 ? (
                      <div className="shui-side-note">
                        · nothing yet — every change under this folder lands here live
                      </div>
                    ) : (
                      feed.map((e) => (
                        <button
                          key={e.rel}
                          type="button"
                          className="shui-feed-row"
                          title={e.rel}
                          onClick={() => openChangeDiff(e.rel, e.kind)}
                        >
                          <span className={`shui-feed-kind ${e.kind}`}>
                            {e.kind === 'created' ? '+' : e.kind === 'deleted' ? '−' : '~'}
                          </span>
                          <span className="shui-feed-path">{e.rel}</span>
                          <span className="shui-feed-time">{timeAgo(e.at)}</span>
                        </button>
                      ))
                    )}
                  </div>
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
            </>
          )}
        </PageSidebar>

        <PageMain>
          {tabs.tabs.length > 0 ? (
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
                      onClick={() => {
                        setDiff(null)
                        setTabs((s) => activateTab(s, tab.path))
                      }}
                      onDoubleClick={() => setTabs((s) => pinTab(s, tab.path))}
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
            <DiffPane host={host} root={root} change={diff.change} baseline={diff.baseline} gitDir={diff.gitDir} />
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
              <span className="t-ghost">select a file to edit — or a git change to diff</span>
            </div>
          )}
        </PageMain>
      </PageBody>
    </PageShell>
  )
}
