import {
  AlertCircle,
  ArrowLeft,
  Check,
  ChevronUp,
  CornerDownLeft,
  Folder,
  FolderOpen,
  FolderPlus,
  GitBranch,
  Search,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import { getIiiClient } from '@/lib/iii-client'
import { loadRecentProjects, removeRecentProject } from '@/lib/storage'
import { cn } from '@/lib/utils'
import {
  lifecycleTone,
  lifecycleToneClass,
  listWorktrees,
  shortWorktreeId,
  type WorktreeInfo,
  worktreeIndicators,
} from '@/lib/worktrees'

/**
 * Per-session working-directory picker, project-switcher style.
 *
 * Opens to your remembered projects (most-recent first) — pick one in a click,
 * or "browse to add" a new directory. Browsing uses shell's operator workspace
 * control plane one level at a time. The search box filters the current level
 * live; typing/pasting an absolute path jumps straight there (browse) or
 * selects it (projects). Every selection — pasted, remembered, or browsed — is
 * validated against the live shell worker before it's accepted, and the
 * worker-echoed canonical path is what gets stored. The chosen dir is what the
 * harness scopes the chat to (`fs_scope.root`); it is re-scopable mid-conversation
 * (a change drops a visible transcript marker).
 */

/** Optional worktree section, gated on the worktree worker's presence. */
export interface WorktreePickerOptions {
  enabled: boolean
  /**
   * Picking a worktree row. The caller sets the conversation's workingDir to
   * the worktree's path AND claims it for the session.
   */
  onPick: (worktree: WorktreeInfo) => void
}

interface DirectoryPickerProps {
  value: string | null
  onChange: (dir: string) => void
  locked?: boolean
  disabled?: boolean
  /** Show the worktrees tab next to directory browsing. */
  worktrees?: WorktreePickerOptions
  className?: string
}

interface WorkspaceRootsResult {
  roots?: string[]
}

interface DirEntry {
  name: string
  kind: string
  path: string
}

interface WorkspaceListResult {
  path: string
  entries?: DirEntry[]
}

interface WorkspaceValidateResult {
  path: string
}

function basename(p: string): string {
  const parts = p.split('/').filter(Boolean)
  return parts.length ? parts[parts.length - 1] : p
}

function parentDisplay(p: string): string {
  const idx = p.replace(/\/+$/, '').lastIndexOf('/')
  return idx <= 0 ? '/' : p.slice(0, idx)
}

function parentOf(p: string): string {
  const trimmed = p.replace(/\/+$/, '')
  const idx = trimmed.lastIndexOf('/')
  return idx <= 0 ? '/' : trimmed.slice(0, idx)
}

const isAbsPath = (s: string) => s.trim().startsWith('/')

export const WORKSPACE_ROOTS_FUNCTION_ID = 'shell::workspace::roots'
export const WORKSPACE_LIST_FUNCTION_ID = 'shell::workspace::list'
export const WORKSPACE_VALIDATE_FUNCTION_ID = 'shell::workspace::validate'

/**
 * iii triggers reject with a plain object `{ code, message }`, not an Error, and
 * the message is often a nested `handler error: {"code":"C211","message":"…"}`.
 * Pull out the human-readable inner message.
 */
function errMsg(err: unknown): string {
  const raw =
    err instanceof Error
      ? err.message
      : err && typeof err === 'object' && 'message' in err
        ? String((err as { message: unknown }).message)
        : String(err)
  // Errors nest: `handler error: {"code":"C211","message":"…"}`. Prefer the
  // innermost (last) message and tolerate escaped quotes inside it.
  const matches = [...raw.matchAll(/"message"\s*:\s*"((?:[^"\\]|\\.)*)"/g)]
  if (matches.length === 0) return raw
  return matches[matches.length - 1][1].replace(/\\(.)/g, '$1')
}

export function DirectoryPicker({
  value,
  onChange,
  locked,
  disabled,
  worktrees,
  className,
}: DirectoryPickerProps) {
  const [open, setOpen] = useState(false)
  const [view, setView] = useState<'projects' | 'browse' | 'worktrees'>(
    'projects',
  )
  const [projects, setProjects] = useState<string[]>([])
  const [query, setQuery] = useState('')
  // browse state
  const [roots, setRoots] = useState<string[] | null>(null)
  const [root, setRoot] = useState<string | null>(null)
  const [path, setPath] = useState<string | null>(null)
  const [dirs, setDirs] = useState<string[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [validating, setValidating] = useState<string | null>(null)
  // worktrees state
  const [wtRows, setWtRows] = useState<WorktreeInfo[]>([])
  const [wtLoading, setWtLoading] = useState(false)
  const [wtError, setWtError] = useState<string | null>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open) return
    const onDocClick = (e: MouseEvent) => {
      if (!containerRef.current?.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', onDocClick)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDocClick)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const openPanel = useCallback(() => {
    setProjects(loadRecentProjects())
    setView('projects')
    setQuery('')
    setError(null)
    setOpen(true)
  }, [])

  const ensureRoots = useCallback(async (): Promise<string[]> => {
    if (roots !== null) return roots
    setLoading(true)
    setError(null)
    try {
      const client = await getIiiClient()
      const info = await client.trigger<WorkspaceRootsResult>(
        WORKSPACE_ROOTS_FUNCTION_ID,
        {},
      )
      const r = info?.roots ?? []
      setRoots(r)
      return r
    } catch (err) {
      setError(errMsg(err))
      setRoots([])
      return []
    } finally {
      setLoading(false)
    }
  }, [roots])

  const loadFolder = useCallback(async (target: string) => {
    setLoading(true)
    setError(null)
    try {
      const client = await getIiiClient()
      const res = await client.trigger<WorkspaceListResult>(
        WORKSPACE_LIST_FUNCTION_ID,
        {
          path: target,
          page_size: 200,
        },
      )
      const names = (res?.entries ?? [])
        .filter((e) => e.kind === 'dir')
        .map((e) => e.path)
        .sort((a, b) => a.localeCompare(b))
      setDirs(names)
    } catch (err) {
      setError(errMsg(err))
      setDirs([])
    } finally {
      setLoading(false)
    }
  }, [])

  const enterBrowse = useCallback(async () => {
    setView('browse')
    setQuery('')
    setError(null)
    const r = await ensureRoots()
    if (r.length === 1) {
      setRoot(r[0])
      setPath(r[0])
      void loadFolder(r[0])
    } else {
      setRoot(null)
      setPath(null)
      setDirs([])
    }
  }, [ensureRoots, loadFolder])

  const enterRoot = useCallback(
    (r: string) => {
      setRoot(r)
      setPath(r)
      setQuery('')
      void loadFolder(r)
    },
    [loadFolder],
  )

  const enterDir = useCallback(
    (d: string) => {
      setPath(d)
      setQuery('')
      void loadFolder(d)
    },
    [loadFolder],
  )

  const goUp = useCallback(() => {
    setQuery('')
    if (!path || !root || path === root) {
      // back to the roots list (or projects if there is nowhere up to go)
      if ((roots?.length ?? 0) > 1) {
        setPath(null)
        setRoot(null)
        return
      }
      setView('projects')
      setProjects(loadRecentProjects())
      return
    }
    const next = parentOf(path)
    const clamped = next.length < root.length ? root : next
    setPath(clamped)
    void loadFolder(clamped)
  }, [path, root, roots, loadFolder])

  const jumpTo = useCallback(
    async (raw: string) => {
      const p = raw.trim().replace(/\/+$/, '') || '/'
      setView('browse')
      setQuery('')
      const r = await ensureRoots()
      const matchedRoot =
        [...r]
          .sort((a, b) => b.length - a.length)
          .find((x) => p === x || p.startsWith(`${x}/`)) ??
        r[0] ??
        null
      setRoot(matchedRoot)
      setPath(p)
      await loadFolder(p)
    },
    [ensureRoots, loadFolder],
  )

  const select = useCallback(
    (dir: string) => {
      onChange(dir)
      setOpen(false)
    },
    [onChange],
  )

  // Validate a dir against the LIVE worker before accepting it — a
  // remembered project may be deleted, on another machine, or denylisted, and
  // even a just-browsed dir can vanish between listing and clicking. Every
  // selection path goes through here so the worker-echoed canonical path is
  // what gets stored.
  const validateAndSelect = useCallback(
    async (raw: string) => {
      const dir = raw.trim().replace(/\/+$/, '') || '/'
      setError(null)
      setValidating(dir)
      try {
        const client = await getIiiClient()
        const res = await client.trigger<WorkspaceValidateResult>(
          WORKSPACE_VALIDATE_FUNCTION_ID,
          { path: dir },
        )
        // Select the canonical resolved dir the worker echoes back, not raw
        // input, so stored recent projects are stable across symlinks.
        select(res?.path ?? dir)
      } catch (err) {
        setError(`can't use ${dir} — ${errMsg(err)}`)
      } finally {
        setValidating(null)
      }
    },
    [select],
  )

  const forget = useCallback((dir: string) => {
    removeRecentProject(dir)
    setProjects(loadRecentProjects())
  }, [])

  const enterWorktrees = useCallback(async () => {
    setView('worktrees')
    setQuery('')
    setWtError(null)
    setWtLoading(true)
    try {
      setWtRows(await listWorktrees())
    } catch (err) {
      setWtError(errMsg(err))
      setWtRows([])
    } finally {
      setWtLoading(false)
    }
  }, [])

  const pickWorktree = useCallback(
    (wt: WorktreeInfo) => {
      worktrees?.onPick(wt)
      setOpen(false)
    },
    [worktrees],
  )

  const q = query.trim().toLowerCase()
  const filteredProjects = useMemo(
    () => projects.filter((p) => p.toLowerCase().includes(q)),
    [projects, q],
  )
  const filteredDirs = useMemo(
    () =>
      q
        ? dirs.filter(
            (d) =>
              basename(d).toLowerCase().includes(q) ||
              d.toLowerCase().includes(q),
          )
        : dirs,
    [dirs, q],
  )
  const filteredWorktrees = useMemo(
    () =>
      q
        ? wtRows.filter(
            (w) =>
              w.branch.toLowerCase().includes(q) ||
              w.path.toLowerCase().includes(q) ||
              w.repo_path.toLowerCase().includes(q),
          )
        : wtRows,
    [wtRows, q],
  )

  const onSearchKey = (e: React.KeyboardEvent) => {
    if (e.key !== 'Enter' || !isAbsPath(query)) return
    e.preventDefault()
    if (view === 'projects') void validateAndSelect(query)
    else if (view === 'browse') void jumpTo(query)
  }

  const label = value ? basename(value) : 'choose directory'

  if (locked) {
    return (
      <span
        className={cn(
          'inline-flex items-center gap-1 px-2 py-1 text-[11px] lowercase text-ink-faint',
          className,
        )}
        title={value ?? 'no working directory'}
      >
        <Folder size={12} aria-hidden />
        <span className="max-w-[160px] truncate">{label}</span>
      </span>
    )
  }

  return (
    <div ref={containerRef} className={cn('relative inline-flex', className)}>
      <button
        type="button"
        disabled={disabled}
        aria-label="working directory"
        aria-haspopup="dialog"
        aria-expanded={open}
        title={value ?? 'choose a working directory'}
        onClick={() => (open ? setOpen(false) : openPanel())}
        className={cn(
          'inline-flex items-center gap-1 rounded-sm border border-rule px-2 py-1 text-[11px] lowercase transition-colors',
          value ? 'text-ink' : 'text-ink-faint',
          'hover:text-ink disabled:opacity-50',
        )}
      >
        <Folder size={12} aria-hidden />
        <span className="max-w-[160px] truncate">{label}</span>
      </button>

      {open ? (
        <div
          role="dialog"
          aria-label="select working directory"
          className="absolute bottom-full left-0 z-30 mb-1 w-[360px] border border-rule bg-bg shadow-lg"
        >
          {/* section tabs (only with the worktree worker present) */}
          {worktrees?.enabled ? (
            <div
              role="tablist"
              className="flex border-b border-rule-2 text-[11px] lowercase"
            >
              <button
                type="button"
                role="tab"
                aria-selected={view !== 'worktrees'}
                onClick={() => {
                  setView('projects')
                  setProjects(loadRecentProjects())
                  setQuery('')
                  setError(null)
                }}
                className={cn(
                  'flex-1 px-3 py-1.5 transition-colors',
                  view !== 'worktrees'
                    ? 'bg-panel text-ink'
                    : 'text-ink-faint hover:text-ink',
                )}
              >
                directories
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={view === 'worktrees'}
                onClick={() => void enterWorktrees()}
                className={cn(
                  'flex-1 border-l border-rule-2 px-3 py-1.5 transition-colors',
                  view === 'worktrees'
                    ? 'bg-panel text-ink'
                    : 'text-ink-faint hover:text-ink',
                )}
              >
                worktrees
              </button>
            </div>
          ) : null}

          {/* search */}
          <div className="flex items-center gap-2 border-b border-rule-2 px-2.5 py-1.5">
            <Search size={13} className="shrink-0 text-ink-ghost" aria-hidden />
            <input
              // biome-ignore lint/a11y/noAutofocus: focus the search on open for fast filtering
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onSearchKey}
              placeholder={
                view === 'projects'
                  ? 'search projects, or paste a path…'
                  : view === 'worktrees'
                    ? 'filter worktrees…'
                    : 'filter this folder, or paste a path…'
              }
              aria-label="search directories"
              className="min-w-0 flex-1 bg-transparent text-[12px] text-ink placeholder:text-ink-ghost focus:outline-none"
            />
          </div>

          {/* validation/error (shown in the projects view; browse has its own) */}
          {view === 'projects' && error ? (
            <div className="flex items-start gap-1.5 border-b border-rule-2 px-3 py-2 text-[11px] text-ink-faint">
              <AlertCircle size={12} className="mt-0.5 shrink-0" aria-hidden />
              <span>{error}</span>
            </div>
          ) : null}

          {/* body */}
          {view === 'worktrees' ? (
            <div className="max-h-[280px] overflow-y-auto py-1">
              {wtLoading ? (
                <div className="px-3 py-2 text-[11px] lowercase text-ink-ghost">
                  loading…
                </div>
              ) : wtError ? (
                <div className="flex items-start gap-1.5 px-3 py-2 text-[11px] text-ink-faint">
                  <AlertCircle
                    size={12}
                    className="mt-0.5 shrink-0"
                    aria-hidden
                  />
                  <span>{wtError}</span>
                </div>
              ) : filteredWorktrees.length > 0 ? (
                filteredWorktrees.map((wt) => {
                  const tone = lifecycleTone(wt.lifecycle)
                  const { dirty, ahead } = worktreeIndicators(wt.status)
                  const orphaned = wt.lifecycle === 'orphaned'
                  const landing = wt.lifecycle === 'landing'
                  return (
                    <button
                      key={wt.worktree_id}
                      type="button"
                      disabled={orphaned || landing}
                      onClick={() => pickWorktree(wt)}
                      title={
                        orphaned
                          ? `${wt.path} — directory is missing`
                          : landing
                            ? `${wt.path} — land in progress; not retargetable`
                            : `${wt.path} — claim and use this worktree`
                      }
                      className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-panel disabled:opacity-50"
                    >
                      <GitBranch
                        size={13}
                        className="shrink-0 text-ink-faint"
                        aria-hidden
                      />
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="flex min-w-0 items-center gap-1.5 text-[12px]">
                          <span className="truncate text-ink">{wt.branch}</span>
                          <span className="shrink-0 font-mono text-[10px] text-ink-ghost tabular-nums">
                            {shortWorktreeId(wt.worktree_id)}
                          </span>
                          {dirty ? (
                            <span
                              className="shrink-0 text-warn"
                              title="uncommitted changes"
                            >
                              *
                            </span>
                          ) : null}
                          {ahead > 0 ? (
                            <span
                              className="shrink-0 text-[10px] text-ink-faint tabular-nums"
                              title={`${ahead} commit(s) ahead of base`}
                            >
                              +{ahead}
                            </span>
                          ) : null}
                        </span>
                        <span className="truncate font-mono text-[10px] text-ink-ghost">
                          {wt.repo_path}
                        </span>
                      </span>
                      <span className="flex shrink-0 items-center gap-1.5 text-[10px] lowercase">
                        {wt.session_id ? (
                          <span
                            className="max-w-[80px] truncate text-ink-ghost"
                            title={`claimed by ${wt.session_id}`}
                          >
                            {wt.session_id}
                          </span>
                        ) : null}
                        {wt.lifecycle !== 'active' ? (
                          <span
                            className={cn(
                              'flex items-center gap-1',
                              lifecycleToneClass[tone],
                            )}
                          >
                            <StatusDot
                              tone={tone}
                              pulse={wt.lifecycle === 'landing'}
                            />
                            {wt.lifecycle}
                          </span>
                        ) : null}
                      </span>
                    </button>
                  )
                })
              ) : (
                <div className="px-3 py-3 text-[11px] leading-relaxed text-ink-faint">
                  {q
                    ? 'no matching worktrees.'
                    : 'no managed worktrees yet — create one with worktree::create.'}
                </div>
              )}
            </div>
          ) : view === 'projects' ? (
            <div className="max-h-[280px] overflow-y-auto py-1">
              {isAbsPath(query) ? (
                <button
                  type="button"
                  disabled={validating !== null}
                  onClick={() => void validateAndSelect(query)}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-accent hover:bg-panel disabled:opacity-50"
                >
                  <CornerDownLeft size={13} className="shrink-0" aria-hidden />
                  <span className="truncate font-mono">
                    use this path: {query.trim()}
                  </span>
                </button>
              ) : null}

              {filteredProjects.map((p) => (
                <div
                  key={p}
                  className="group flex items-center gap-1 pr-1.5 hover:bg-panel"
                >
                  <button
                    type="button"
                    disabled={validating !== null}
                    onClick={() => void validateAndSelect(p)}
                    className="flex min-w-0 flex-1 items-center gap-2 px-3 py-1.5 text-left disabled:opacity-50"
                    title={p}
                  >
                    <Folder
                      size={13}
                      className="shrink-0 text-ink-faint"
                      aria-hidden
                    />
                    <span className="flex min-w-0 flex-col">
                      <span className="truncate text-[12px] text-ink">
                        {basename(p)}
                      </span>
                      <span className="truncate font-mono text-[10px] text-ink-ghost">
                        {parentDisplay(p)}
                      </span>
                    </span>
                  </button>
                  <button
                    type="button"
                    aria-label={`forget ${p}`}
                    title="forget this project"
                    onClick={() => forget(p)}
                    className="shrink-0 p-1 text-ink-ghost opacity-0 transition-opacity hover:text-ink group-hover:opacity-100"
                  >
                    <X size={12} aria-hidden />
                  </button>
                </div>
              ))}

              {filteredProjects.length === 0 && !isAbsPath(query) ? (
                <div className="px-3 py-3 text-[11px] leading-relaxed text-ink-faint">
                  {projects.length === 0
                    ? 'no recent projects yet.'
                    : 'no matching projects.'}{' '}
                  browse to add one.
                </div>
              ) : null}

              <div className="mt-1 border-t border-rule-2 pt-1">
                <button
                  type="button"
                  onClick={() => void enterBrowse()}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-ink-faint hover:bg-panel hover:text-ink"
                >
                  <FolderPlus size={13} className="shrink-0" aria-hidden />
                  browse to add a project…
                </button>
              </div>
            </div>
          ) : (
            <div>
              {/* browse header */}
              <div className="flex items-center justify-between gap-2 border-b border-rule-2 px-2 py-1.5">
                <span className="flex min-w-0 items-center gap-1 text-[11px] text-ink-faint">
                  <button
                    type="button"
                    aria-label="back to projects"
                    onClick={() => {
                      setView('projects')
                      setProjects(loadRecentProjects())
                      setQuery('')
                      setError(null)
                    }}
                    className="text-ink-faint hover:text-ink"
                  >
                    <ArrowLeft size={13} aria-hidden />
                  </button>
                  {path ? (
                    <button
                      type="button"
                      aria-label="up one level"
                      onClick={goUp}
                      className="text-ink-faint hover:text-ink"
                    >
                      <ChevronUp size={13} aria-hidden />
                    </button>
                  ) : null}
                  <span className="truncate font-mono">{path ?? 'roots'}</span>
                </span>
                {path ? (
                  <button
                    type="button"
                    disabled={validating !== null}
                    onClick={() => void validateAndSelect(path)}
                    className="inline-flex items-center gap-1 whitespace-nowrap text-[11px] lowercase text-accent hover:underline disabled:opacity-50"
                  >
                    <Check size={12} aria-hidden /> use this folder
                  </button>
                ) : null}
              </div>

              <div className="max-h-[260px] overflow-y-auto py-1">
                {isAbsPath(query) ? (
                  <button
                    type="button"
                    onClick={() => void jumpTo(query)}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-accent hover:bg-panel"
                  >
                    <CornerDownLeft
                      size={13}
                      className="shrink-0"
                      aria-hidden
                    />
                    <span className="truncate font-mono">
                      go to {query.trim()}
                    </span>
                  </button>
                ) : null}

                {loading ? (
                  <div className="px-3 py-2 text-[11px] lowercase text-ink-ghost">
                    loading…
                  </div>
                ) : error ? (
                  <div className="flex items-start gap-1.5 px-3 py-2 text-[11px] text-ink-faint">
                    <AlertCircle
                      size={12}
                      className="mt-0.5 shrink-0"
                      aria-hidden
                    />
                    <span>{error}</span>
                  </div>
                ) : path === null ? (
                  // roots list (only when multiple roots)
                  (roots ?? []).map((r) => (
                    <button
                      key={r}
                      type="button"
                      onClick={() => enterRoot(r)}
                      className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-ink hover:bg-panel"
                    >
                      <FolderOpen size={13} className="shrink-0" aria-hidden />
                      <span className="truncate font-mono">{r}</span>
                    </button>
                  ))
                ) : filteredDirs.length > 0 ? (
                  filteredDirs.map((d) => (
                    <button
                      key={d}
                      type="button"
                      onClick={() => enterDir(d)}
                      className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-ink hover:bg-panel"
                    >
                      <Folder size={13} className="shrink-0" aria-hidden />
                      <span className="truncate font-mono">{basename(d)}</span>
                    </button>
                  ))
                ) : (
                  <div className="px-3 py-2 text-[11px] lowercase text-ink-ghost">
                    {q
                      ? 'no matching sub-folders'
                      : 'no sub-folders — use this folder'}
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Static discoverability footnote — same line in both views, per
              the filesystem-access spec §6: sets expectations before the first
              grant prompt ever fires. */}
          <div className="border-t border-rule-2 px-3 py-1.5 font-mono text-[10px] leading-relaxed text-ink-ghost">
            the agent can use the chosen folder freely — it asks before touching
            anything outside it.
          </div>
        </div>
      ) : null}
    </div>
  )
}
