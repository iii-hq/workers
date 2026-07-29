/**
 * The `#/ext/editor` page: a file tree, tabs, and the shared Monaco editor.
 *
 * A **folder** is the unit, not a repository — tree, tabs, editor and search
 * all work in a plain directory, and git only adds a branch label, change
 * marks and the action strip when the root happens to be a repo.
 *
 * The workspace itself lives in the worker, not here. This page reads it and
 * writes to it, which makes it a second view rather than a second editor: an
 * agent that opens a file puts a tab on your screen, and closing this tab
 * closes it for the agent too.
 *
 * Layout note: this page shares the viewport with the console's chat pane, so
 * it gets about half the window. The sidebar is deliberately narrow and
 * collapsible — at these widths, every pixel spent on chrome is taken from the
 * code.
 *
 * Chrome note: the shared `CodeEditor` is intentionally bare (no line numbers,
 * no glyph margin, no minimap), and the SOP forbids bundling another editor.
 * So the page carries the chrome instead — the status line below the editor is
 * where the language, size and git deltas live, which is the honest substitute
 * for a gutter we cannot paint.
 */

import {
  Badge,
  Button,
  CodeEditor,
  CodeHighlight,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  EmptyState,
  type Host,
  Input,
  StatusDot,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import {
  type Buffer,
  createApi,
  errorText,
  isDir,
  isNotARepo,
  type SaveResult,
  type SearchResult,
  type StatusEntry,
  type StatusReport,
  type SyncAction,
  statusMark,
  type TreeNode,
  visibleRows,
} from '../lib/api'

const POLL_MS = 3_000
/** How long a row keeps its "just changed" accent after an edit lands. */
const FLASH_MS = 12_000

/** Editor contents per open path. The worker owns *which* paths are open; the
 *  text being typed is the one thing genuinely local until it is saved. */
interface Draft {
  base: string
  draft: string
  truncated: boolean
  stale: boolean
}

interface Delta {
  added: number
  removed: number
  untracked: boolean
  patch: string
}

export function EditorPage({ host }: { host: Host }) {
  const api = useMemo(() => createApi(host), [host])

  const [root, setRoot] = useState('')
  const [rootInput, setRootInput] = useState('')
  const [rootOpen, setRootOpen] = useState(false)
  const [sideOpen, setSideOpen] = useState(true)
  const [buffers, setBuffers] = useState<Buffer[]>([])
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [treeRoot, setTreeRoot] = useState<TreeNode | null>(null)
  const [drafts, setDrafts] = useState<Record<string, Draft>>({})
  const [activePath, setActivePath] = useState<string | null>(null)
  const [view, setView] = useState<'edit' | 'diff' | 'git'>('edit')
  const [mode, setMode] = useState<'files' | 'search'>('files')
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<string[]>([])
  const [searchResult, setSearchResult] = useState<SearchResult | null>(null)
  const [status, setStatus] = useState<StatusReport | null>(null)
  const [noRepo, setNoRepo] = useState(false)
  const [delta, setDelta] = useState<Delta | null>(null)
  const [flashed, setFlashed] = useState<Record<string, number>>({})
  const [conflict, setConflict] = useState<SaveResult | null>(null)
  const [commitMessage, setCommitMessage] = useState('')
  const [gitNote, setGitNote] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  // Read inside the poll without making it a dependency — rebuilding the
  // interval on every keystroke would reset the timer and stall the feed.
  const draftsRef = useRef(drafts)
  draftsRef.current = drafts
  const seenRef = useRef<Record<string, string>>({})

  const activeBuffer = buffers.find((b) => b.path === activePath) ?? null
  const activeDraft = activePath ? (drafts[activePath] ?? null) : null
  const dirty = activeDraft !== null && activeDraft.draft !== activeDraft.base

  const applyWorkspace = useCallback((ws: { root: string; buffers: Buffer[]; expanded: string[] }) => {
    setRoot(ws.root)
    setBuffers(ws.buffers)
    setExpanded(new Set(ws.expanded))
  }, [])

  const loadTree = useCallback(
    async (opts: { expand?: string[]; collapse?: string[] } = {}) => {
      try {
        const result = await api.tree(opts)
        setTreeRoot(result.tree.root)
        setRoot(result.root)
        setExpanded(new Set(result.expanded))
        setError(null)
      } catch (e) {
        setError(errorText(e))
      }
    },
    [api],
  )

  useEffect(() => {
    let cancelled = false
    api
      .workspace()
      .then((ws) => {
        if (cancelled) return
        applyWorkspace(ws)
        setRootInput(ws.root)
        if (ws.buffers.length > 0) setActivePath(ws.buffers[ws.buffers.length - 1].path)
      })
      .catch((e) => {
        if (!cancelled) setError(errorText(e))
      })
    void loadTree()
    return () => {
      cancelled = true
    }
  }, [api, applyWorkspace, loadTree])

  /** Pull contents for any buffer we do not have text for yet. */
  const hydrate = useCallback(
    async (list: Buffer[]) => {
      for (const buffer of list) {
        if (draftsRef.current[buffer.path]) continue
        try {
          const file = await api.open(buffer.path)
          setDrafts((prev) => ({
            ...prev,
            [buffer.path]: {
              base: file.content,
              draft: file.content,
              truncated: file.truncated,
              stale: false,
            },
          }))
        } catch {
          // A buffer we cannot read keeps its tab; the error shows on select.
        }
      }
    },
    [api],
  )

  useEffect(() => {
    void hydrate(buffers)
  }, [buffers, hydrate])

  const openPath = useCallback(
    async (path: string) => {
      setError(null)
      try {
        const file = await api.open(path)
        setDrafts((prev) => ({
          ...prev,
          [path]: {
            base: file.content,
            draft: file.content,
            truncated: file.truncated,
            stale: false,
          },
        }))
        applyWorkspace(await api.workspace())
        setActivePath(path)
        setView('edit')
      } catch (e) {
        setError(errorText(e))
      }
    },
    [api, applyWorkspace],
  )

  const toggleFolder = useCallback(
    (path: string) => {
      // The worker owns expansion (it collapses descendants with the parent),
      // so the toggle is a call and the response is the truth.
      void loadTree(expanded.has(path) ? { collapse: [path] } : { expand: [path] })
    },
    [expanded, loadTree],
  )

  /** git is an overlay — a folder with no repository is not an error state. */
  const refreshGit = useCallback(async () => {
    try {
      const report = await api.status()
      setStatus(report)
      setNoRepo(false)

      const now = Date.now()
      const seen = seenRef.current
      const fresh: Record<string, number> = {}
      const nextSeen: Record<string, string> = {}
      for (const entry of report.entries) {
        const signature = `${entry.index}/${entry.worktree}`
        nextSeen[entry.path] = signature
        if (seen[entry.path] !== undefined && seen[entry.path] !== signature) {
          fresh[entry.path] = now
        }
      }
      seenRef.current = nextSeen
      if (Object.keys(fresh).length > 0) setFlashed((prev) => ({ ...prev, ...fresh }))
    } catch (e) {
      const message = errorText(e)
      setStatus(null)
      setNoRepo(isNotARepo(message))
      if (!isNotARepo(message)) setError(message)
    }
  }, [api])

  /** Pull open tabs forward when their file moved on disk. */
  const refreshBuffers = useCallback(async () => {
    try {
      const ws = await api.workspace()
      setBuffers(ws.buffers)
      for (const buffer of ws.buffers) {
        const local = draftsRef.current[buffer.path]
        if (!local) continue
        const file = await api.open(buffer.path)
        if (file.content === local.base) continue
        const edited = local.draft !== local.base
        setDrafts((prev) => ({
          ...prev,
          [buffer.path]: edited
            ? { ...local, stale: true }
            : { base: file.content, draft: file.content, truncated: file.truncated, stale: false },
        }))
        setFlashed((prev) => ({ ...prev, [buffer.path]: Date.now() }))
      }
    } catch {
      // Transient; the next tick tries again.
    }
  }, [api])

  useEffect(() => {
    const id = setInterval(() => {
      void refreshGit()
      void refreshBuffers()
    }, POLL_MS)
    void refreshGit()
    return () => clearInterval(id)
  }, [refreshGit, refreshBuffers])

  useEffect(() => {
    if (Object.keys(flashed).length === 0) return
    const id = setInterval(() => {
      const cutoff = Date.now() - FLASH_MS
      setFlashed((prev) => {
        const next = Object.fromEntries(Object.entries(prev).filter(([, at]) => at > cutoff))
        return Object.keys(next).length === Object.keys(prev).length ? prev : next
      })
    }, 2_000)
    return () => clearInterval(id)
  }, [flashed])

  /** The status line's git deltas — the gutter we cannot paint. */
  useEffect(() => {
    if (activePath === null || noRepo) {
      setDelta(null)
      return
    }
    let cancelled = false
    api
      .hunks(activePath)
      .then((h) => {
        if (!cancelled)
          setDelta({
            added: h.added,
            removed: h.removed,
            untracked: h.untracked,
            patch: h.patch,
          })
      })
      .catch(() => {
        if (!cancelled) setDelta(null)
      })
    return () => {
      cancelled = true
    }
  }, [activePath, api, noRepo, status])

  useEffect(() => {
    if (mode !== 'files' || query.trim() === '') {
      setResults([])
      return
    }
    let cancelled = false
    const id = setTimeout(() => {
      api
        .find(query, 20)
        .then((r) => {
          if (!cancelled) setResults(r.matches.map((m) => m.path))
        })
        .catch((e) => {
          if (!cancelled) setError(errorText(e))
        })
    }, 120)
    return () => {
      cancelled = true
      clearTimeout(id)
    }
  }, [api, mode, query])

  const runSearch = useCallback(async () => {
    if (query.trim() === '') return
    setBusy(true)
    try {
      setSearchResult(await api.search(query, true))
      setError(null)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }, [api, query])

  const save = useCallback(async () => {
    if (!activePath || !activeBuffer || !activeDraft) return
    setBusy(true)
    try {
      const result = await api.save(activePath, activeDraft.draft, activeBuffer.mtime)
      if (result.conflict) {
        setConflict(result)
      } else {
        setDrafts((prev) => ({
          ...prev,
          [activePath]: { ...activeDraft, base: activeDraft.draft, stale: false },
        }))
        applyWorkspace(await api.workspace())
        void refreshGit()
      }
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }, [activeBuffer, activeDraft, activePath, api, applyWorkspace, refreshGit])

  const closeTab = useCallback(
    async (path: string) => {
      try {
        const result = await api.closeBuffer(path)
        setBuffers(result.buffers)
        setDrafts((prev) => {
          const next = { ...prev }
          delete next[path]
          return next
        })
        setActivePath((current) =>
          current === path ? (result.buffers[result.buffers.length - 1]?.path ?? null) : current,
        )
      } catch (e) {
        setError(errorText(e))
      }
    },
    [api],
  )

  const changeRoot = useCallback(async () => {
    if (rootInput.trim() === '') return
    try {
      const ws = await api.openWorkspace(rootInput.trim())
      applyWorkspace(ws)
      setDrafts({})
      setActivePath(ws.buffers[ws.buffers.length - 1]?.path ?? null)
      setRootOpen(false)
      setError(null)
      await loadTree()
      void refreshGit()
    } catch (e) {
      setError(errorText(e))
    }
  }, [api, applyWorkspace, loadTree, refreshGit, rootInput])

  const gitAction = useCallback(
    async (run: () => Promise<{ summary: string }>) => {
      setBusy(true)
      try {
        setGitNote((await run()).summary || 'done')
        void refreshGit()
      } catch (e) {
        setGitNote(errorText(e))
      } finally {
        setBusy(false)
      }
    },
    [refreshGit],
  )

  const marks = useMemo(() => {
    const map = new Map<string, StatusEntry>()
    for (const entry of status?.entries ?? []) map.set(entry.path, entry)
    return map
  }, [status])

  const rows = useMemo(() => (treeRoot ? visibleRows(treeRoot, expanded) : []), [treeRoot, expanded])

  const lineCount = activeDraft ? activeDraft.draft.split('\n').length : 0

  return (
    <div className="ed-root">
      <header className="ed-head">
        <span className="ed-brand">editor</span>
        {rootOpen ? (
          <>
            <span className="ed-commit">
              <Input
                value={rootInput}
                onChange={setRootInput}
                placeholder="folder to open…"
                aria-label="Workspace folder"
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void changeRoot()
                }}
                preserveCase
              />
            </span>
            <Button onClick={() => void changeRoot()}>open</Button>
            <button type="button" className="ed-icon" onClick={() => setRootOpen(false)}>
              ×
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              className="ed-rootpath"
              title={`${root} — click to change`}
              onClick={() => setRootOpen(true)}
              style={{ border: 0, background: 'transparent', cursor: 'pointer', textAlign: 'left' }}
            >
              {root || '…'}
            </button>
            <button
              type="button"
              className="ed-icon"
              onClick={() => setSideOpen((v) => !v)}
              aria-label={sideOpen ? 'Hide the sidebar' : 'Show the sidebar'}
              title={sideOpen ? 'Hide the sidebar' : 'Show the sidebar'}
            >
              {sideOpen ? '⟨' : '⟩'}
            </button>
          </>
        )}
      </header>

      <div className="ed-body">
        {sideOpen && (
          <aside className="ed-side">
            <div className="ed-seg">
              <button type="button" data-active={mode === 'files'} onClick={() => setMode('files')}>
                files
              </button>
              <button type="button" data-active={mode === 'search'} onClick={() => setMode('search')}>
                search
              </button>
            </div>

            <Input
              value={query}
              onChange={setQuery}
              placeholder={mode === 'files' ? 'find a file…' : 'search contents…'}
              aria-label={mode === 'files' ? 'Find a file' : 'Search file contents'}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && mode === 'search') void runSearch()
              }}
              preserveCase
            />

            <div className="ed-scroll">
              {mode === 'search' ? (
                searchResult === null ? (
                  <div className="ed-hint">enter to search contents</div>
                ) : searchResult.files.length === 0 ? (
                  <div className="ed-hint">no matches</div>
                ) : (
                  <ul className="ed-list">
                    <li className="ed-hint">
                      {searchResult.total} in {searchResult.files.length} files
                      {searchResult.truncated && ' · capped'}
                    </li>
                    {searchResult.files.map((file) => (
                      <li key={file.path}>
                        <button
                          type="button"
                          className="ed-row"
                          data-active={file.path === activePath}
                          onClick={() => void openPath(file.path)}
                          title={file.path}
                        >
                          <span className="ed-name">{file.path}</span>
                          <span className="ed-count">{file.hits.length}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                )
              ) : results.length > 0 ? (
                <ul className="ed-list">
                  {results.map((path) => (
                    <li key={path}>
                      <button
                        type="button"
                        className="ed-row"
                        data-active={path === activePath}
                        onClick={() => void openPath(path)}
                        title={path}
                      >
                        <span className="ed-name">{path}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              ) : treeRoot === null ? (
                <div className="ed-hint">loading…</div>
              ) : rows.length === 0 ? (
                <div className="ed-hint">empty folder</div>
              ) : (
                <ul className="ed-list">
                  {rows.map((row) => {
                    const entry = marks.get(row.path)
                    const dir = isDir(row)
                    return (
                      <li key={row.path}>
                        <button
                          type="button"
                          className="ed-row"
                          data-active={row.path === activePath}
                          data-fresh={flashed[row.path] !== undefined}
                          style={{ paddingLeft: `${6 + row.depth * 11}px` }}
                          onClick={() => (dir ? toggleFolder(row.path) : void openPath(row.path))}
                          title={row.path}
                        >
                          <span className="ed-caret">{dir ? (expanded.has(row.path) ? '▾' : '▸') : ''}</span>
                          <span className={`ed-name${dir ? ' ed-dir' : ''}`}>{row.name}</span>
                          {entry && (
                            <span
                              className="ed-mark"
                              data-s={entry.worktree !== 'unchanged' ? entry.worktree : entry.index}
                            >
                              {statusMark(entry)}
                            </span>
                          )}
                        </button>
                      </li>
                    )
                  })}
                </ul>
              )}
            </div>
          </aside>
        )}

        <section className="ed-main">
          {buffers.length === 0 ? (
            <EmptyState
              title="No file open"
              description="Pick a file from the tree, or search for one. Files an agent opens appear here too."
            />
          ) : (
            <>
              <div className="ed-tabs">
                {buffers.map((buffer) => {
                  const local = drafts[buffer.path]
                  return (
                    <div key={buffer.path} className="ed-tab" data-active={buffer.path === activePath}>
                      <button
                        type="button"
                        className="ed-tab-name"
                        onClick={() => setActivePath(buffer.path)}
                        title={buffer.path}
                      >
                        {buffer.path.split('/').pop()}
                      </button>
                      {local && local.draft !== local.base && <span className="ed-dot">•</span>}
                      <button
                        type="button"
                        className="ed-close"
                        onClick={() => void closeTab(buffer.path)}
                        aria-label={`Close ${buffer.path}`}
                      >
                        ×
                      </button>
                    </div>
                  )
                })}
              </div>

              {activePath && activeDraft && (
                <>
                  <div className="ed-bar">
                    <div className="ed-seg">
                      <button type="button" data-active={view === 'edit'} onClick={() => setView('edit')}>
                        edit
                      </button>
                      <button type="button" data-active={view === 'diff'} onClick={() => setView('diff')}>
                        unsaved
                      </button>
                      <button type="button" data-active={view === 'git'} onClick={() => setView('git')}>
                        head
                      </button>
                    </div>
                    <span className="ed-spacer" />
                    {activeDraft.stale && <Badge variant="warn">disk moved</Badge>}
                    <Button onClick={() => void save()} disabled={!dirty || busy}>
                      {busy ? 'saving…' : 'save'}
                    </Button>
                  </div>

                  {activeDraft.truncated && (
                    <div className="ed-warn">
                      Larger than <code>max_file_bytes</code> — only the beginning was read, so saving is refused.
                    </div>
                  )}

                  {view === 'edit' ? (
                    <div className="ed-surface">
                      <CodeEditor
                        value={activeDraft.draft}
                        language={activeBuffer?.language ?? 'plaintext'}
                        readOnly={activeDraft.truncated}
                        aria-label={activePath}
                        onChange={(next) =>
                          setDrafts((prev) => ({
                            ...prev,
                            [activePath]: { ...activeDraft, draft: next },
                          }))
                        }
                      />
                    </div>
                  ) : view === 'diff' ? (
                    <LocalDiff host={host} path={activePath} base={activeDraft.base} draft={activeDraft.draft} />
                  ) : (
                    <DiffPane path={activePath} patch={delta?.patch ?? ''} />
                  )}

                  <div className="ed-status">
                    <span className="ed-status-path" title={activePath}>
                      {activePath}
                    </span>
                    <span>{activeBuffer?.language ?? 'plaintext'}</span>
                    <span>{lineCount} ln</span>
                    {delta &&
                      (delta.untracked ? (
                        <span className="ed-add">new</span>
                      ) : (
                        (delta.added > 0 || delta.removed > 0) && (
                          <span>
                            <span className="ed-add">+{delta.added}</span>{' '}
                            <span className="ed-del">−{delta.removed}</span>
                          </span>
                        )
                      ))}
                    <span className={dirty ? 'ed-unsaved' : undefined}>{dirty ? 'unsaved' : 'saved'}</span>
                  </div>
                </>
              )}
            </>
          )}
        </section>
      </div>

      {status !== null && (
        <footer className="ed-git">
          <span className="ed-branch">
            <StatusDot tone={status.clean ? 'ink' : 'accent'} pulse={!status.clean} />
            {status.branch ?? 'detached'}
            {(status.ahead > 0 || status.behind > 0) && (
              <span className="ed-ab">
                {status.ahead > 0 && `↑${status.ahead}`}
                {status.behind > 0 && `↓${status.behind}`}
              </span>
            )}
          </span>
          <span className="ed-commit">
            <Input
              value={commitMessage}
              onChange={setCommitMessage}
              placeholder="commit message…"
              aria-label="Commit message"
              preserveCase
            />
          </span>
          <Button
            disabled={busy || commitMessage.trim() === ''}
            onClick={() =>
              void gitAction(async () => {
                const r = await api.commit(commitMessage.trim())
                setCommitMessage('')
                return r
              })
            }
          >
            commit
          </Button>
          {(['fetch', 'pull', 'push'] as SyncAction[]).map((action) => (
            <Button key={action} variant="ghost" disabled={busy} onClick={() => void gitAction(() => api.sync(action))}>
              {action}
            </Button>
          ))}
          <Button variant="ghost" disabled={busy} onClick={() => void gitAction(() => api.stash('push'))}>
            stash
          </Button>
          <Button variant="ghost" disabled={busy} onClick={() => void gitAction(() => api.stash('pop'))}>
            pop
          </Button>
        </footer>
      )}

      {gitNote && (
        <div className="ed-gitnote">
          {/* A plain div, not a button: git's output is the thing you most
              want to select and copy, and wrapping it in a click target makes
              that impossible. The dismiss affordance is its own control. */}
          <pre className="ed-gitnote-text">{gitNote}</pre>
          <button
            type="button"
            className="ed-gitnote-x"
            onClick={() => setGitNote(null)}
            aria-label="Dismiss this git message"
          >
            ×
          </button>
        </div>
      )}
      {noRepo && !error && <div className="ed-hint">not a git repository — tree and editor still work</div>}
      {error && <div className="ed-error">{error}</div>}

      <Dialog open={conflict !== null} onOpenChange={(open) => !open && setConflict(null)}>
        <DialogContent>
          <DialogTitle>This file changed while you were editing it</DialogTitle>
          <DialogDescription>
            Nothing was written. Below is the difference between what is on disk now and what you tried to save.
          </DialogDescription>
          <CodeHighlight code={conflict?.conflict_patch ?? ''} language="diff" />
          <div className="ed-bar">
            <Button variant="ghost" onClick={() => setConflict(null)}>
              keep editing
            </Button>
            <Button
              onClick={() => {
                if (activePath) void openPath(activePath)
                setConflict(null)
              }}
            >
              discard mine and reload
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}

/** Rendered lines a patch may contribute before it is cut short. */
const MAX_PATCH_LINES = 600

/**
 * A unified patch, rendered in the worker.
 *
 * Deliberately hand-rolled rather than borrowed from the console. The console's
 * own diff cards are backed by a library that weighs megabytes; a worker asset
 * is capped at 8 MiB and bundling a second copy of it measured at 10.3 MB, so
 * the only way to share it would be to widen `@iii-dev/console-ui`. This page
 * is not worth changing the shared contract for.
 *
 * What it gives up is syntax highlighting inside the diff. What it keeps is the
 * part that makes a diff readable: one row per line, added and removed lines
 * banded rather than marked by a leading character, hunk headers separating
 * them, and the file's real line numbers down the left — taken from the `@@`
 * headers, not counted off the patch rows.
 */
function DiffPane({ path, patch }: { path: string; patch: string }) {
  if (patch.trim() === '') return <div className="ed-hint">no changes</div>

  const all = patch.split('\n')
  const shown = all.slice(0, MAX_PATCH_LINES)
  const overflow = all.length - shown.length

  let lineNo = 0
  const rows = shown.map((line, index) => {
    let kind = 'ctx'
    let num: number | null = null
    if (line.startsWith('@@')) {
      kind = 'hunk'
      const m = /\+(\d+)/.exec(line.split('@@')[1] ?? '')
      lineNo = m ? Number(m[1]) : lineNo
    } else if (
      line.startsWith('+++') ||
      line.startsWith('---') ||
      line.startsWith('diff ') ||
      line.startsWith('index ')
    ) {
      kind = 'meta'
    } else if (line.startsWith('+')) {
      kind = 'add'
      num = lineNo++
    } else if (line.startsWith('-')) {
      kind = 'del'
    } else {
      num = lineNo++
    }
    return { line, kind, num, key: `${index}` }
  })

  return (
    <div className="ed-patch">
      <div className="ed-patch-head">
        <span className="ed-card-path">{path}</span>
      </div>
      <div className="ed-patch-body">
        {rows.map((row) => (
          <div key={row.key} className="ed-dline" data-k={row.kind}>
            <span className="ed-dnum">{row.num ?? ''}</span>
            <span className="ed-dtext">{row.line || ' '}</span>
          </div>
        ))}
        {overflow > 0 && <div className="ed-hint">… {overflow} more lines (truncated)</div>}
      </div>
    </div>
  )
}

/** The buffer against its last read: what saving would write. */
function LocalDiff({ host, path, base, draft }: { host: Host; path: string; base: string; draft: string }) {
  const api = useMemo(() => createApi(host), [host])
  const [patch, setPatch] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    api
      .diff(base, draft, path)
      .then((r) => {
        if (!cancelled) setPatch(r.identical ? '' : r.patch)
      })
      .catch(() => {
        if (!cancelled) setPatch(null)
      })
    return () => {
      cancelled = true
    }
  }, [api, base, draft, path])

  if (patch === null) return <div className="ed-hint">computing…</div>
  return <DiffPane path={path} patch={patch} />
}
