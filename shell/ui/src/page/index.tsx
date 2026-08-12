/**
 * The shell explorer page (#/ext/shell) — an IDE-shaped surface over the
 * worker's own functions: a collapsible sidebar (files / git / search,
 * icon tabs) beside a Monaco editor with VS Code-style file tabs
 * (single click previews, double click pins) and a FileDiff pane for
 * git selections.
 *
 * A target picker beside the root affordance retargets the same surface
 * at a live sandbox microVM: fs calls then carry
 * `target: { kind: "sandbox", sandbox_id }` (host mode omits the field —
 * the wire default), the tree roots at `/`, and the host-coupled
 * affordances (git tab, workspace roots, editor writes) drop out.
 *
 * The sidebar hugs the pane's OUTER edge (`panelSide`), and the whole
 * UI state — browsed root, open tabs, expanded folders — persists per
 * workspace tab (`tabId`) in the `shell-ui` configuration entry (host
 * mode only; guest state is per-VM and ephemeral).
 */

import {
  EmptyState,
  type Host,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  StatusDot,
} from '@iii-dev/console-ui'
import type { GitStatusEntry } from '@pierre/trees'
import {
  Check,
  CircleAlert,
  Copy,
  FolderTree,
  GitBranch,
  RefreshCw,
  Search,
  SquareTerminal,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage, truncateMiddle } from '../lib/format'
import { type CoderInfo, coderInfo, coderTree, type FlatTree, flattenTree } from './coder'
import { DiffPane } from './DiffPane'
import { type EditorCache, EditorPane } from './EditorPane'
import { FilesTab } from './FilesTab'
import { GitTab } from './GitTab'
import { type GitChange, type GitState, gitChanges } from './git'
import { createTabUiStateSaver, loadTabUiState, type TabUiState } from './persist'
import { GUEST_ROOT, sandboxGone } from './sandbox'
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
import { useGuestTree } from './useGuestTree'
import { useSandboxFleet } from './useSandboxFleet'

type SideTab = 'files' | 'git' | 'search'

const SIDE_TABS: { id: SideTab; label: string; Icon: typeof FolderTree }[] = [
  { id: 'files', label: 'files', Icon: FolderTree },
  { id: 'git', label: 'git', Icon: GitBranch },
  { id: 'search', label: 'search', Icon: Search },
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
  const [sandboxId, setSandboxId] = useState<string | null>(null)
  const [sideTab, setSideTab] = useState<SideTab>('files')
  const [collapsed, setCollapsed] = useState(false)
  const [tree, setTree] = useState<FlatTree | null>(null)
  const [git, setGit] = useState<GitState | null>(null)
  const [tabs, setTabs] = useState<TabsState>(EMPTY_TABS)
  const [expanded, setExpanded] = useState<string[]>([])
  const [dirtyPaths, setDirtyPaths] = useState<ReadonlySet<string>>(new Set())
  const [diff, setDiff] = useState<GitChange | null>(null)
  const cacheRef = useRef<EditorCache>(new Map())

  const { fleet, refreshFleet } = useSandboxFleet(host)
  const guestTree = useGuestTree(host, sandboxId, expanded)
  const gone = sandboxGone(sandboxId, fleet)

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
    } else if (restored && !restored.root && next === info.primary_root) {
      // Legacy/first save without a root: restore against the primary.
      setTabs(restoreTabs(restored.open, restored.active))
      setExpanded(restored.expanded)
    }
  }, [info, restored, root, workingDir])

  // ── data loads (gated on the resolved root; host mode only — the
  // guest tree loads through useGuestTree) ──
  const gitSeqRef = useRef(0)
  const refreshGit = useCallback(() => {
    if (!root || sandboxId !== null) return
    const seq = ++gitSeqRef.current
    gitChanges(host, root)
      .then((state) => {
        if (gitSeqRef.current === seq) setGit(state)
      })
      .catch((err: unknown) => {
        if (gitSeqRef.current === seq) {
          setGit({
            kind: 'error',
            message: errorMessage(err),
          })
        }
      })
  }, [host, root, sandboxId])

  const treeSeqRef = useRef(0)
  const refreshTree = useCallback(() => {
    if (!root || sandboxId !== null) return
    const seq = ++treeSeqRef.current
    coderTree(host, root)
      .then((out) => {
        if (treeSeqRef.current === seq) setTree(flattenTree(out.root))
      })
      .catch(() => {
        if (treeSeqRef.current === seq) {
          setTree({ paths: [], kinds: new Map(), truncations: [] })
        }
      })
  }, [host, root, sandboxId])

  useEffect(() => {
    setTree(null)
    setGit(null)
    refreshTree()
    refreshGit()
  }, [refreshTree, refreshGit])

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
    // Guest state is per-VM and ephemeral — only host state persists.
    if (sandboxId !== null) return
    saver.save({
      root,
      open: tabs.tabs,
      active: tabs.active,
      expanded,
    })
  }, [saver, root, tabs, expanded, sandboxId])

  // ── open/close/pin actions ──
  const previewFile = useCallback((relPath: string) => {
    setDiff(null)
    setTabs((s) => openPreview(s, relPath))
  }, [])

  const pinFile = useCallback((relPath: string) => {
    setDiff(null)
    setTabs((s) => openPinned(s, relPath))
  }, [])

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

  const changeRoot = useCallback((nextRoot: string) => {
    cacheRef.current.clear()
    setDirtyPaths(new Set())
    setTabs(EMPTY_TABS)
    setExpanded([])
    setDiff(null)
    setRoot(nextRoot)
  }, [])

  // Host tabs/expanded survive a round-trip through a sandbox: snapshot
  // on the way in, restore on the way out — in the same commit, so the
  // persistence effect never sees (and saves) cleared guest state over
  // the host state it stored.
  const hostUiRef = useRef<{ tabs: TabsState; expanded: string[] } | null>(null)

  const changeTarget = useCallback(
    (next: string | null) => {
      cacheRef.current.clear()
      setDirtyPaths(new Set())
      setDiff(null)
      if (next !== null) {
        if (sandboxId === null) hostUiRef.current = { tabs, expanded }
        setTabs(EMPTY_TABS)
        setExpanded([])
        // The git tab is host-only chrome — land on files when entering a VM.
        setSideTab((t) => (t === 'git' ? 'files' : t))
      } else {
        const snap = hostUiRef.current
        hostUiRef.current = null
        setTabs(snap?.tabs ?? EMPTY_TABS)
        setExpanded(snap?.expanded ?? [])
      }
      setSandboxId(next)
    },
    [sandboxId, tabs, expanded],
  )

  // ── follow the chat's working directory ──
  // Picking another folder in chat re-roots the explorer (the split-screen
  // sync). Only CHANGES sync: the boot-resolved root wins on mount, and a
  // manual root pick sticks until the chat's folder moves again.
  const lastWorkingDirRef = useRef(workingDir ?? null)
  useEffect(() => {
    const next = workingDir ?? null
    if (next === lastWorkingDirRef.current) return
    lastWorkingDirRef.current = next
    if (next === null || root === null || next === root) return
    if (sandboxId !== null) {
      // The chat's folder is a host concept — remember it for the return
      // to host without evicting the sandbox view. The snapshotted tabs
      // belong to the old root, so the return starts clean instead.
      hostUiRef.current = null
      setRoot(next)
      return
    }
    changeRoot(next)
  }, [workingDir, root, sandboxId, changeRoot])

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

  // Same rule for the target select: a selected sandbox that left the
  // fleet stays present as a labeled option until the user moves off it.
  const targetOptions = useMemo(() => {
    const options = fleet.sandboxes.map((s) => ({
      value: s.sandbox_id,
      label: `${s.name ?? truncateMiddle(s.sandbox_id, 16)}${s.stopped ? ' · stopped' : ''}`,
      title: s.sandbox_id,
    }))
    if (sandboxId !== null && !fleet.sandboxes.some((s) => s.sandbox_id === sandboxId)) {
      options.push({
        value: sandboxId,
        label: `${truncateMiddle(sandboxId, 16)} · gone`,
        title: sandboxId,
      })
    }
    return options
  }, [fleet.sandboxes, sandboxId])

  // Function-not-found on sandbox::list = no sandbox daemon on this
  // engine — the picker hides entirely and the page stays pure host.
  const showTargetPicker = fleet.status === 'ready' || fleet.status === 'error'
  const selected =
    sandboxId !== null
      ? fleet.sandboxes.find((s) => s.sandbox_id === sandboxId)
      : undefined
  const effectiveRoot = sandboxId !== null ? GUEST_ROOT : root

  // Click-to-copy with a copied/failed flash — the clipboard write can
  // reject (denied permission) or be missing entirely (insecure context),
  // and a silent no-op reads as a working copy.
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'failed'>('idle')
  const copyTimerRef = useRef<number | null>(null)
  useEffect(
    () => () => {
      if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current)
    },
    [],
  )
  const copySandboxId = useCallback(() => {
    if (sandboxId === null) return
    const flash = (state: 'copied' | 'failed') => {
      setCopyState(state)
      if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current)
      copyTimerRef.current = window.setTimeout(() => {
        copyTimerRef.current = null
        setCopyState('idle')
      }, 1400)
    }
    if (!navigator.clipboard?.writeText) {
      flash('failed')
      return
    }
    navigator.clipboard.writeText(sandboxId).then(
      () => flash('copied'),
      () => flash('failed'),
    )
  }, [sandboxId])

  const sideTabs =
    sandboxId === null ? SIDE_TABS : SIDE_TABS.filter((t) => t.id !== 'git')

  const header = (
    <PageHeader
      icon={<SquareTerminal />}
      title="shell"
      description={
        sandboxId !== null ? (
          <span className="shui-target-desc" title={sandboxId}>
            <StatusDot tone={gone ? 'warn' : selected?.stopped ? 'ink' : 'accent'} />
            <span className="id">{truncateMiddle(sandboxId, 24)}</span>
            <button
              type="button"
              className={`shui-copy-btn${copyState === 'idle' ? '' : ` ${copyState}`}`}
              onClick={copySandboxId}
              aria-label="copy sandbox id"
              title={
                copyState === 'copied'
                  ? 'copied'
                  : copyState === 'failed'
                    ? 'copy failed'
                    : 'copy sandbox id'
              }
            >
              {copyState === 'copied' ? (
                <Check aria-hidden className="shui-copy-icon" />
              ) : copyState === 'failed' ? (
                <CircleAlert aria-hidden className="shui-copy-icon" />
              ) : (
                <Copy aria-hidden className="shui-copy-icon" />
              )}
            </button>
          </span>
        ) : root ? (
          <span title={root}>{root}</span>
        ) : undefined
      }
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
  if (!info || !root || !effectiveRoot) {
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
        <PageSidebar width={collapsed ? 34 : 280} className={`shui-sidebar${collapsed ? ' collapsed' : ''}`}>
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
                {sideTabs.map(({ id, label, Icon }) => (
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

              {showTargetPicker ? (
                <div className="shui-target-row">
                  <select
                    className="shui-target-select"
                    value={sandboxId ?? 'host'}
                    onChange={(e) => {
                      const value = e.target.value
                      changeTarget(value === 'host' ? null : value)
                    }}
                    aria-label="target"
                    title={sandboxId ?? 'host'}
                  >
                    <option value="host">host</option>
                    {targetOptions.map((o) => (
                      <option key={o.value} value={o.value} title={o.title}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className="shui-target-refresh"
                    onClick={refreshFleet}
                    aria-label="refresh sandbox list"
                    title="refresh sandbox list"
                  >
                    <RefreshCw aria-hidden className="shui-refresh-icon" />
                  </button>
                </div>
              ) : null}

              {sandboxId === null ? (
                rootOptions.length > 1 ? (
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
                )
              ) : null}

              <div className="shui-side-body">
                {gone ? (
                  <EmptyState
                    title="sandbox gone"
                    description="reaped or stopped — the microVM left the fleet"
                    action={{ label: 'back to host', onClick: () => changeTarget(null) }}
                  />
                ) : sideTab === 'files' ? (
                  <FilesTab
                    tree={sandboxId !== null ? guestTree : tree}
                    gitStatus={treeGitStatus}
                    theme={theme}
                    expanded={expanded}
                    onExpandedChange={setExpanded}
                    onPreviewFile={previewFile}
                    onPinFile={pinFile}
                  />
                ) : sideTab === 'git' ? (
                  <GitTab state={git} theme={theme} onSelect={(change) => setDiff(change)} onRefresh={refreshGit} />
                ) : (
                  <SearchTab
                    host={host}
                    root={effectiveRoot}
                    sandboxId={sandboxId}
                    onPreviewFile={previewFile}
                    onPinFile={pinFile}
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
            <DiffPane host={host} root={root} change={diff} />
          ) : tabs.active !== null ? (
            <EditorPane
              key={tabs.active}
              host={host}
              root={effectiveRoot}
              relPath={tabs.active}
              sandboxId={sandboxId}
              cache={cacheRef.current}
              onSaved={onSaved}
              onDirtyChange={onDirtyChange}
            />
          ) : (
            <div className="shui-main-empty">
              <span className="t-ghost">
                {sandboxId !== null
                  ? 'select a file to view — sandbox target is read-only'
                  : 'select a file to edit — or a git change to diff'}
              </span>
            </div>
          )}
        </PageMain>
      </PageBody>
    </PageShell>
  )
}
