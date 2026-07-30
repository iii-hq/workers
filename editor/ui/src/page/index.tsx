/**
 * The `#/ext/editor` page: a file tree, tabs, a reader and an editor.
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
 * Reading and writing are different jobs, so they use different surfaces:
 *
 * - **read** renders `@pierre/diffs`' `File` — real line numbers, a real
 *   syntax theme, and a file header. Render-only, which is exactly right for
 *   the job a file spends most of its time doing.
 * - **edit** keeps the console's shared Monaco `CodeEditor`. It is
 *   intentionally bare (no line numbers, no glyph margin, no minimap) because
 *   it is built to be a form field, and the SOP forbids bundling a second
 *   editor to get the chrome back. It is still the only surface here that can
 *   accept a keystroke.
 *
 * Diffs are pierre's too: `editor::diff` and `editor::git::hunks` both return
 * unified patch text, and pierre parses that into a real diff with its own
 * add/delete colouring rather than the banded rows this page used to paint.
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
  MarkdownPreview,
  StatusDot,
} from '@iii-dev/console-ui'
import { DEFAULT_THEMES, parsePatchFiles } from '@pierre/diffs'
import { File, FileDiff } from '@pierre/diffs/react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import {
  type Api,
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
import { contentHash } from '../lib/cache-key'
import { type ChangedEvent, useWorkspaceEvents } from '../lib/events'

/** How long a row keeps its "just changed" accent after an edit lands. */
const FLASH_MS = 12_000

/**
 * Trailing-edge window for folding pushed events into one refresh, in ms.
 *
 * Events arrive one per write, and the case this page exists for — an agent
 * editing a directory in a loop — delivers them in bursts. A refresh per event
 * costs a workspace read, a git status and a file read each time, and lets a
 * slow pass overlap the next.
 *
 * 100ms is short on purpose. The push rewrite exists so that you watch the
 * agent work, so an isolated event has to land immediately rather than
 * eventually, and 100ms is inside the window where a UI still reads as
 * immediate. Nothing user-visible waits on it either: the row accent and the
 * status line are set straight from the event payload, before the timer is
 * armed. Raising this buys fewer round trips at the cost of the one property
 * the feature exists for.
 */
const COALESCE_MS = 100

/** A cancellable pause, so a sustained stream of events gets a window between
 *  passes instead of back-to-back round trips. */
const settle = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms))

/**
 * pierre renders into its own shadow root and injects its stylesheet there
 * from the resolved theme, so there is no CSS asset to ship for it and no
 * `console:style` entry to add. What it does need is the theme *names* and
 * which of the pair is live, which is why `themeType` is threaded down from
 * the console's own light/dark state rather than left on 'system' — the page
 * is inside the console's theme, not the OS's.
 *
 * `disableWorkerPool` is not a performance choice so much as an honest one:
 * pierre's worker pool needs a `workerFactory` returning a real `Worker`, and
 * a worker script would have to be a second URL the console serves. It only
 * serves assets a worker registers as `console:script`, and its loader
 * requires each of those to export a `default` setup function — a worker
 * script has none. So highlighting runs on the main thread. For the file
 * sizes this worker will open (bounded by `max_file_bytes`) that is a
 * non-issue; a 10k-line file is where it would start to show.
 */
const READ_OPTIONS = { theme: DEFAULT_THEMES, overflow: 'scroll' } as const
const DIFF_OPTIONS = { theme: DEFAULT_THEMES, diffStyle: 'unified', overflow: 'scroll' } as const

/** Editor contents per open path. The worker owns *which* paths are open; the
 *  text being typed is the one thing genuinely local until it is saved. */
interface Draft {
  base: string
  draft: string
  /**
   * The mtime the file had when `base` was read.
   *
   * Per draft rather than in a map of its own so it cannot drift from the text
   * it describes: it is set wherever `base` is and dropped with the tab. Its
   * one job is the gate in `syncWorkspace` — a workspace buffer whose recorded
   * mtime still matches this one has not been written *through the worker*
   * since this page read it, so re-reading it would spend a file body for
   * nothing.
   */
  mtime: number
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
  // Follows the console's own light/dark toggle, not the OS preference.
  const themeType = host.useTheme()

  const [root, setRoot] = useState('')
  const [rootInput, setRootInput] = useState('')
  const [rootOpen, setRootOpen] = useState(false)
  const [sideOpen, setSideOpen] = useState(true)
  const [buffers, setBuffers] = useState<Buffer[]>([])
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [treeRoot, setTreeRoot] = useState<TreeNode | null>(null)
  const [drafts, setDrafts] = useState<Record<string, Draft>>({})
  const [activePath, setActivePath] = useState<string | null>(null)
  // Reading is the default: opening a file should show you the file, not put
  // a cursor in it.
  const [view, setView] = useState<'read' | 'edit' | 'preview' | 'diff' | 'git'>('read')
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
  const [lastChange, setLastChange] = useState<ChangedEvent | null>(null)

  // Read inside a refresh without making it a dependency — rebuilding the
  // refresh callbacks on every keystroke would re-run the effects that bind
  // them and churn the subscriptions.
  const draftsRef = useRef(drafts)
  draftsRef.current = drafts
  const seenRef = useRef<Record<string, string>>({})

  const activeBuffer = buffers.find((b) => b.path === activePath) ?? null
  const activeDraft = activePath ? (drafts[activePath] ?? null) : null
  const dirty = activeDraft !== null && activeDraft.draft !== activeDraft.base
  // The worker classifies the language, so the extension check is only a
  // fallback for a file it did not recognise.
  const markdown = activeBuffer?.language === 'markdown' || /\.(md|markdown|mdx)$/i.test(activePath ?? '')

  // Two of the tabs only exist for some files: `unsaved` while something is
  // unsaved, `preview` on markdown. Saving, undoing back to the original, or
  // switching to a tab of another kind all take a button away, and leaving the
  // view pointed at it would strand the user somewhere with no way back and
  // nothing in it. Fall back to `read`, which every file has.
  useEffect(() => {
    if (view === 'diff' && !dirty) setView('read')
    if (view === 'preview' && !markdown) setView('read')
  }, [view, dirty, markdown])

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
              mtime: file.mtime,
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
            mtime: file.mtime,
            truncated: file.truncated,
            stale: false,
          },
        }))
        applyWorkspace(await api.workspace())
        setActivePath(path)
        setView('read')
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

  /**
   * Pull one open tab forward from disk.
   *
   * An untouched buffer adopts the new text. An edited one keeps the edit and
   * gains `stale`, which raises the "disk moved" badge and leaves the save
   * guard to refuse the write rather than overwriting silently.
   *
   * The mtime is recorded even when the content came back identical, and that
   * is not tidiness: `editor::open` upserts the buffer into the shared
   * workspace, so this read moves the *record's* mtime. A local copy left
   * behind would make the gate in `syncWorkspace` re-read the same file on the
   * state event this very call is about to emit, which re-opens it, which emits
   * another — a loop with no exit.
   */
  const reread = useCallback(
    async (path: string) => {
      const local = draftsRef.current[path]
      if (!local) return
      try {
        const file = await api.open(path)
        const moved = file.content !== local.base
        let next: Draft = { ...local, mtime: file.mtime }
        if (moved) {
          next =
            local.draft === local.base
              ? {
                  base: file.content,
                  draft: file.content,
                  mtime: file.mtime,
                  truncated: file.truncated,
                  stale: false,
                }
              : { ...next, stale: true }
        }
        // Closed while the read was in flight: do not resurrect the tab.
        setDrafts((prev) => (prev[path] ? { ...prev, [path]: next } : prev))
        if (moved) setFlashed((prev) => ({ ...prev, [path]: Date.now() }))
      } catch {
        // A file that cannot be read keeps the text we already have; the error
        // surfaces when the user selects the tab.
      }
    },
    [api],
  )

  /**
   * Re-read the shared workspace and pull forward only the tabs that moved.
   *
   * `mtime` on a workspace buffer is the mtime the *worker* last read that file
   * at — it advances on `editor::open` and `editor::save` and nowhere else — so
   * comparing it against the mtime this page recorded when it read the file
   * identifies exactly which tabs were saved through the worker by another
   * surface or an agent. The rest keep the text they have instead of costing a
   * full file body each, which is what makes this affordable per event rather
   * than per timer tick.
   *
   * The gate is only sound *here*. A write that never went through this worker
   * (`shell::fs::write`) does not move the record's mtime at all; those arrive
   * as `editor::changed` with the path in the payload and are re-read
   * unconditionally, which is why the changed path is passed to `reread`
   * directly instead of being looked for here.
   *
   * Residual: mtime is Unix *seconds*, so a save made through the worker inside
   * the same second as this page's last read of that file is indistinguishable
   * from no change and waits for the next event touching it. Reading
   * unconditionally instead is not an option — see `reread`, it does not
   * terminate. The worker's buffer record carries an opaque `version` that is
   * the same fact without the one-second floor; when the save path starts
   * sending it, this gate should compare that in preference to the mtime and
   * the residual goes away.
   */
  const syncWorkspace = useCallback(async () => {
    try {
      const ws = await api.workspace()
      setBuffers(ws.buffers)
      for (const buffer of ws.buffers) {
        const local = draftsRef.current[buffer.path]
        if (!local || buffer.mtime === local.mtime) continue
        await reread(buffer.path)
      }
    } catch {
      // Transient; the next event tries again.
    }
  }, [api, reread])

  /** Work accumulated since the last pass: paths whose file needs re-reading,
   *  and whether the git overlay is stale. */
  const pendingRef = useRef<{ paths: Set<string>; git: boolean }>({ paths: new Set(), git: false })
  /** Set by every event, cleared as a pass takes the work. */
  const wantedRef = useRef(false)
  const runningRef = useRef(false)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  /**
   * Drain the accumulated work, one pass at a time.
   *
   * Exactly one pass is ever in flight. The guard **defers rather than drops**:
   * an event that lands mid-pass stays in `pendingRef` and the loop picks it up
   * on its next turn, so the last event of a burst is always the one the page
   * settles on. Discarding it instead would leave the UI showing content that
   * is merely old, which is worse than the burst being fixed here.
   */
  const flush = useCallback(async () => {
    if (runningRef.current) return
    runningRef.current = true
    try {
      while (wantedRef.current) {
        wantedRef.current = false
        const work = pendingRef.current
        pendingRef.current = { paths: new Set(), git: false }

        await syncWorkspace()
        // One read per changed path. The event carries a patch, not the new
        // content, and reconstructing the file from a patch here is the hand
        // parsing this page deliberately no longer does.
        for (const path of work.paths) await reread(path)
        if (work.git) await refreshGit()

        // Anything that arrived during the pass gets its own window rather than
        // an immediate re-run, so a sustained stream costs one pass per window
        // instead of one pass per bus round trip.
        if (wantedRef.current) await settle(COALESCE_MS)
      }
    } finally {
      runningRef.current = false
    }
  }, [refreshGit, reread, syncWorkspace])

  /**
   * Record what an event implies and arm the window.
   *
   * The timer is deliberately *not* restarted per event: a stream arriving
   * faster than `COALESCE_MS` would then postpone the refresh for as long as it
   * lasted, which is the one failure this cannot have. Arming once per window
   * makes the window a floor on refresh spacing rather than a ceiling on how
   * long a change can stay invisible.
   */
  const schedule = useCallback(
    (path?: string, git = false) => {
      if (path !== undefined) pendingRef.current.paths.add(path)
      if (git) pendingRef.current.git = true
      wantedRef.current = true
      if (timerRef.current !== null) return
      timerRef.current = setTimeout(() => {
        timerRef.current = null
        void flush()
      }, COALESCE_MS)
    },
    [flush],
  )

  useEffect(
    () => () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current)
    },
    [],
  )

  const { onState, onChanged } = useWorkspaceEvents(host)

  // The workspace record lives in `state`, so every buffer and expansion change
  // arrives as an event. One read on mount seeds the git overlay; after that
  // nothing is polled.
  useEffect(() => {
    void refreshGit()
  }, [refreshGit])

  // A change to this worker's `state` scope means the shared workspace moved:
  // an agent opened or closed something, or saved. The record itself says which
  // buffers moved, so no path is named here.
  useEffect(() => onState(() => schedule()), [onState, schedule])

  // A file changed, however it changed. The accent and the status line are set
  // straight from the payload so the page reacts within the frame; only the
  // re-read and the git marks wait for the window.
  useEffect(
    () =>
      onChanged((event: ChangedEvent) => {
        setFlashed((prev) => ({ ...prev, [event.path]: Date.now() }))
        setLastChange(event)
        schedule(event.path, true)
      }),
    [onChanged, schedule],
  )

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
      const result = await api.save(activePath, activeDraft.draft, activeBuffer.mtime, activeBuffer.version)
      if (result.conflict) {
        setConflict(result)
      } else {
        setDrafts((prev) => ({
          ...prev,
          // `result.mtime` is what the file now has on disk. Recording it keeps
          // the `syncWorkspace` gate quiet about the buffer this save just
          // upserted into the shared workspace.
          [activePath]: { ...activeDraft, base: activeDraft.draft, mtime: result.mtime, stale: false },
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
                      <button type="button" data-active={view === 'read'} onClick={() => setView('read')}>
                        read
                      </button>
                      <button type="button" data-active={view === 'edit'} onClick={() => setView('edit')}>
                        edit
                      </button>
                      {/*
                        Markdown only. `MarkdownPreview` is the console's own
                        component, the documented preview counterpart to
                        `CodeEditor`, and iii-directory already renders skills,
                        prompts and registry READMEs through it. Reusing it
                        keeps this page consistent with those and costs nothing
                        in the bundle, since console-ui resolves at runtime.
                      */}
                      {markdown && (
                        <button type="button" data-active={view === 'preview'} onClick={() => setView('preview')}>
                          preview
                        </button>
                      )}
                      {/*
                        Only while there is something unsaved. This view diffs
                        the draft against what was last read, so on a clean file
                        it is guaranteed empty, and it was showing that empty
                        state most of the time. Note it never carries an agent's
                        edits either: those are written to disk, so they arrive
                        under `head`. Its one real job is reviewing your own
                        typing before you commit to it, which only exists while
                        you are mid-edit.
                      */}
                      {dirty && (
                        <button type="button" data-active={view === 'diff'} onClick={() => setView('diff')}>
                          unsaved
                        </button>
                      )}
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

                  {view === 'read' ? (
                    <FileView path={activePath} contents={activeDraft.draft} themeType={themeType} />
                  ) : view === 'preview' ? (
                    // The draft, not the saved file, so the preview tracks what
                    // is being typed rather than what was last written.
                    <div className="ed-pane">
                      <MarkdownPreview markdown={activeDraft.draft} />
                    </div>
                  ) : view === 'edit' ? (
                    <div className="ed-pane ed-surface">
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
                    <LocalDiff
                      api={api}
                      path={activePath}
                      base={activeDraft.base}
                      draft={activeDraft.draft}
                      themeType={themeType}
                    />
                  ) : (
                    <PatchView patch={delta?.patch ?? ''} themeType={themeType} />
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
                    {lastChange && (
                      <span className="ed-live" title={`${lastChange.cause} → ${lastChange.path}`}>
                        {lastChange.kind} {lastChange.path.split('/').pop()}
                      </span>
                    )}
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
          {/* The one patch still shown as highlighted source rather than as a
              pierre diff. pierre sizes itself against a flex parent it can
              scroll inside; a dialog is neither, and a diff that grows past
              the dialog instead of scrolling in it is worse than this. */}
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

/**
 * One file, read-only, with the chrome a reader wants.
 *
 * `cacheKey` is the memo key pierre hands its render cache, so it has to move
 * whenever the text or the name does or the pane keeps showing the previous
 * file. `path` alone does not move when a file is edited in place, so the
 * contents are hashed — see `contentHash`, which is where the earlier
 * length-only key and what it got wrong are written down.
 */
function FileView({ path, contents, themeType }: { path: string; contents: string; themeType: 'light' | 'dark' }) {
  const file = useMemo(() => ({ name: path, contents, cacheKey: `${path}:${contentHash(contents)}` }), [contents, path])
  return (
    <div className="ed-pane ed-reader">
      <File file={file} options={{ ...READ_OPTIONS, themeType }} disableWorkerPool />
    </div>
  )
}

/**
 * A unified patch, rendered by pierre.
 *
 * Parsed here rather than handed to pierre's `PatchDiff`, which throws unless
 * the text yields exactly one file with exactly one diff — an empty patch (a
 * clean file) and a multi-file patch both take the page down with it.
 * `parsePatchFiles` does not throw (`throwOnError` defaults false), so the
 * empty case is a hint and the multi-file case is a `FileDiff` per file.
 *
 * The `cacheKeyPrefix` argument gives every parsed file a cache key derived
 * from the prefix and its position, which is what keeps pierre from re-reading
 * an unchanged hunk on each render.
 */
function PatchView({ patch, themeType }: { patch: string; themeType: 'light' | 'dark' }) {
  const files = useMemo(
    () => (patch.trim() === '' ? [] : parsePatchFiles(patch, `p${contentHash(patch)}`).flatMap((p) => p.files)),
    [patch],
  )

  if (files.length === 0) return <div className="ed-hint">no changes</div>

  return (
    <div className="ed-pane ed-reader">
      {files.map((file, index) => (
        <FileDiff
          key={file.cacheKey ?? `${file.name}-${index}`}
          fileDiff={file}
          options={{ ...DIFF_OPTIONS, themeType }}
          disableWorkerPool
        />
      ))}
    </div>
  )
}

/** The buffer against its last read: what saving would write. */
function LocalDiff({
  api,
  path,
  base,
  draft,
  themeType,
}: {
  api: Api
  path: string
  base: string
  draft: string
  themeType: 'light' | 'dark'
}) {
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
  return <PatchView patch={patch} themeType={themeType} />
}
