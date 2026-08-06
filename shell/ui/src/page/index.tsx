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
import { FolderTree, GitBranch, Search, SquareTerminal, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { type CoderInfo, coderInfo, coderTree, type FlatTree, flattenTree } from './coder'
import { DiffPane } from './DiffPane'
import { type EditorCache, EditorPane } from './EditorPane'
import { FilesTab } from './FilesTab'
import { GitTab } from './GitTab'
import { type GitChange, type GitState, gitChanges } from './git'
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

type SideTab = 'files' | 'git' | 'search'

const SIDE_TABS: { id: SideTab; label: string; Icon: typeof FolderTree }[] = [
  { id: 'files', label: 'files', Icon: FolderTree },
  { id: 'git', label: 'git', Icon: GitBranch },
  { id: 'search', label: 'search', Icon: Search },
]

export function ShellExplorerPage({ host, panelSide, tabId, onRequestClose }: { host: Host } & PageRenderProps) {
  const theme = host.useTheme()
  const [info, setInfo] = useState<CoderInfo | null>(null)
  const [infoError, setInfoError] = useState<string | null>(null)
  const [restored, setRestored] = useState<TabUiState | null | 'loading'>('loading')
  const [root, setRoot] = useState<string | null>(null)
  const [sideTab, setSideTab] = useState<SideTab>('files')
  const [collapsed, setCollapsed] = useState(false)
  const [tree, setTree] = useState<FlatTree | null>(null)
  const [git, setGit] = useState<GitState | null>(null)
  const [tabs, setTabs] = useState<TabsState>(EMPTY_TABS)
  const [expanded, setExpanded] = useState<string[]>([])
  const [dirtyPaths, setDirtyPaths] = useState<ReadonlySet<string>>(new Set())
  const [diff, setDiff] = useState<GitChange | null>(null)
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

  // Root resolution waits for BOTH: a persisted root only counts while
  // it is still an allowed base path.
  useEffect(() => {
    if (!info || restored === 'loading' || root !== null) return
    const persisted = restored?.root && info.base_paths.includes(restored.root) ? restored.root : null
    setRoot(persisted ?? info.primary_root)
    if (persisted) {
      setTabs(restoreTabs(restored?.open, restored?.active))
      setExpanded(restored?.expanded ?? [])
    } else if (restored && !restored.root) {
      // Legacy/first save without a root: restore against the primary.
      setTabs(restoreTabs(restored.open, restored.active))
      setExpanded(restored.expanded)
    }
  }, [info, restored, root])

  // ── data loads (gated on the resolved root) ──
  const gitSeqRef = useRef(0)
  const refreshGit = useCallback(() => {
    if (!root) return
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
  }, [host, root])

  const treeSeqRef = useRef(0)
  const refreshTree = useCallback(() => {
    if (!root) return
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
  }, [host, root])

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
    saver.save({
      root,
      open: tabs.tabs,
      active: tabs.active,
      expanded,
    })
  }, [saver, root, tabs, expanded])

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

  const onSaved = useCallback(() => {
    refreshGit()
  }, [refreshGit])

  const treeGitStatus = useMemo<readonly GitStatusEntry[]>(() => {
    if (git?.kind !== 'ready') return []
    return git.changes.map((c) => ({ path: c.path, status: c.status }))
  }, [git])

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

              {info.base_paths.length > 1 ? (
                <select
                  className="shui-root-select"
                  value={root}
                  onChange={(e) => changeRoot(e.target.value)}
                  aria-label="browsed root"
                  title={root}
                >
                  {info.base_paths.map((p) => (
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
                    tree={tree}
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
                  <SearchTab host={host} root={root} onPreviewFile={previewFile} onPinFile={pinFile} />
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
