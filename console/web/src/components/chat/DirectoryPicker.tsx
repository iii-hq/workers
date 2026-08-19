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
import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { createPortal } from 'react-dom'
import { StatusDot } from '@/components/ui/StatusDot'
import { useMediaQuery } from '@/hooks/use-media-query'
import { getIiiClient } from '@/lib/iii-client'
import { loadRecentProjects, removeRecentProject } from '@/lib/storage'
import { cn } from '@/lib/utils'
import {
  errMsg,
  validateWorkspaceDir,
  WORKSPACE_LIST_FUNCTION_ID,
  WORKSPACE_ROOTS_FUNCTION_ID,
  WORKSPACE_VALIDATE_FUNCTION_ID,
} from '@/lib/working-dir'
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
  /**
   * Externally-detected problem with the current value (e.g. the saved dir
   * no longer validates against the live shell). Auto-opens the panel with
   * the message shown so the user can pick a replacement.
   */
  externalError?: string | null
  /**
   * The stack's default working directory (harness launch folder). Pinned
   * at the top of the projects view with a "default" tag: unlike recents it
   * is never forgettable and survives re-scoping away, so the launch folder
   * stays one click away. Deduped against the recents list.
   */
  defaultDir?: string | null
  /** Show the worktrees tab next to directory browsing. */
  worktrees?: WorktreePickerOptions
  className?: string
  /** Render only the picker content inside an existing sheet page. */
  presentation?: 'trigger' | 'embedded'
  /** Called after an embedded picker accepts a directory. */
  onSelect?: () => void
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

// Re-exported for existing consumers/tests; canonical home is lib/working-dir.
export {
  WORKSPACE_LIST_FUNCTION_ID,
  WORKSPACE_ROOTS_FUNCTION_ID,
  WORKSPACE_VALIDATE_FUNCTION_ID,
}

export function DirectoryPicker({
  value,
  onChange,
  locked,
  disabled,
  externalError,
  defaultDir,
  worktrees,
  className,
  presentation = 'trigger',
  onSelect,
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
  const panelRef = useRef<HTMLDivElement>(null)
  const mobileSheet = useMediaQuery('(max-width: 767px)')
  const embedded = presentation === 'embedded'

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open || embedded) return
    const onDocClick = (e: MouseEvent) => {
      const target = e.target as Node
      if (
        !containerRef.current?.contains(target) &&
        !panelRef.current?.contains(target)
      ) {
        setOpen(false)
      }
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
  }, [open, embedded])

  const openPanel = useCallback(() => {
    setProjects(loadRecentProjects())
    setView('projects')
    setQuery('')
    setError(null)
    setOpen(true)
  }, [])

  useEffect(() => {
    if (!embedded) return
    setProjects(loadRecentProjects())
    setView('projects')
    setQuery('')
    setError(externalError ?? null)
  }, [embedded, externalError])

  // A stale saved dir (deleted, unmounted, denylisted) surfaces here: open
  // the panel with the failure shown so the user picks a replacement instead
  // of silently chatting against a dead folder.
  useEffect(() => {
    if (!externalError || locked || disabled) return
    setProjects(loadRecentProjects())
    setView('projects')
    setQuery('')
    setError(externalError)
    if (!embedded) setOpen(true)
  }, [externalError, locked, disabled, embedded])

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
      onSelect?.()
    },
    [onChange, onSelect],
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
      // Select the canonical resolved dir the worker echoes back, not raw
      // input, so stored recent projects are stable across symlinks.
      const res = await validateWorkspaceDir(dir)
      if (res.ok) {
        setValidating(null)
        select(res.path)
        return
      }
      setError(`can't use ${dir} — ${res.error}`)
      setValidating(null)
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
      // onPick handles claim bookkeeping; the working dir itself goes
      // through the same live-worker validation as every other selection,
      // so the stored path is the worker-echoed canonical one.
      worktrees?.onPick(wt)
      void validateAndSelect(wt.path)
    },
    [worktrees, validateAndSelect],
  )

  const q = query.trim().toLowerCase()
  // The pinned default row replaces any identical recents row (a user who
  // explicitly picked the default lands it in recents too — show it once).
  const filteredProjects = useMemo(
    () =>
      projects.filter((p) => p !== defaultDir && p.toLowerCase().includes(q)),
    [projects, q, defaultDir],
  )
  const showDefaultRow =
    !!defaultDir && (q === '' || defaultDir.toLowerCase().includes(q))
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

  if (locked && !embedded) {
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
    <div
      ref={containerRef}
      className={cn(
        embedded
          ? 'flex h-full min-h-0 w-full flex-col'
          : 'relative inline-flex min-w-0',
        className,
      )}
    >
      {!embedded ? (
        <button
          type="button"
          disabled={disabled}
          aria-label="working directory"
          aria-haspopup="dialog"
          aria-expanded={open}
          title={value ?? 'choose a working directory'}
          onClick={() => (open ? setOpen(false) : openPanel())}
          className={cn(
            'inline-flex h-12 min-w-0 items-center gap-2 rounded-sm border border-transparent bg-transparent px-3 font-mono text-base lowercase text-ink-faint hover:bg-surface-hover hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus sm:h-9 sm:text-[13px]',
            externalError ? 'text-warn' : value ? 'text-ink' : 'text-ink-faint',
            'hover:border-ink hover:text-ink disabled:opacity-50',
          )}
        >
          <Folder aria-hidden className="size-4 shrink-0" />
          <span className="min-w-0 max-w-[160px] truncate">{label}</span>
        </button>
      ) : null}

      {embedded || open ? (
        <DirectoryPickerPortal enabled={!embedded && mobileSheet}>
          {!embedded ? (
            <button
              type="button"
              aria-label="close project picker"
              onClick={() => setOpen(false)}
              className="fixed inset-0 z-40 bg-black/55 md:hidden"
            />
          ) : null}
          {/* biome-ignore lint/a11y/useAriaPropsSupportedByRole: role is always group or dialog, both support an accessible name. */}
          <div
            ref={panelRef}
            role={embedded ? 'group' : 'dialog'}
            aria-label="select working directory"
            className={cn(
              embedded
                ? 'flex min-h-0 flex-1 flex-col overflow-hidden bg-transparent'
                : 'fixed inset-x-3 bottom-3 z-50 max-h-[calc(100dvh-1.5rem)] overscroll-contain overflow-y-auto rounded-lg border border-edge bg-panel-raised pb-[max(0.75rem,env(safe-area-inset-bottom))] shadow-floating md:absolute md:inset-x-auto md:right-0 md:bottom-full md:left-auto md:z-30 md:mb-2 md:max-h-none md:w-[420px] md:overflow-visible md:rounded-lg md:p-2',
            )}
          >
            {!embedded ? (
              <div className="sticky top-0 z-10 bg-panel-raised px-4 pb-3 md:hidden">
                <div
                  className="flex h-6 items-center justify-center"
                  aria-hidden
                >
                  <span className="h-1 w-10 rounded-full bg-ink-ghost/60" />
                </div>
                <div className="flex min-h-12 items-center justify-between gap-3">
                  <div>
                    <h2 className="font-sans text-lg font-semibold text-ink">
                      Working directory
                    </h2>
                    <p className="font-sans text-base text-ink-faint">
                      {worktrees?.enabled
                        ? 'Choose a recent project, folder, or managed worktree.'
                        : 'Choose a recent project or browse folders.'}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => setOpen(false)}
                    aria-label="close project picker"
                    className="flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
                  >
                    <X className="size-5" aria-hidden />
                  </button>
                </div>
              </div>
            ) : null}

            {/* section tabs (only with the worktree worker present) */}
            {worktrees?.enabled ? (
              <div
                role="tablist"
                className="mx-4 mb-3 flex shrink-0 gap-1 rounded-md bg-surface p-1 font-sans text-base md:mx-0 md:mb-2 md:text-[12px]"
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
                    'min-h-12 flex-1 rounded-sm px-3 py-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-8 md:py-1.5',
                    view !== 'worktrees'
                      ? 'bg-panel-raised text-ink ring-1 ring-edge'
                      : 'text-ink-faint hover:bg-surface-hover hover:text-ink',
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
                    'min-h-12 flex-1 rounded-sm px-3 py-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-8 md:py-1.5',
                    view === 'worktrees'
                      ? 'bg-panel-raised text-ink ring-1 ring-edge'
                      : 'text-ink-faint hover:bg-surface-hover hover:text-ink',
                  )}
                >
                  worktrees
                </button>
              </div>
            ) : null}

            {/* search */}
            <div className="mx-4 mb-3 flex min-h-12 shrink-0 items-center gap-2 rounded-md bg-surface px-3 py-2 focus-within:ring-2 focus-within:ring-rule-focus md:mx-0 md:mb-2 md:min-h-9 md:px-2.5 md:py-1.5">
              <Search className="size-4 shrink-0 text-ink-ghost" aria-hidden />
              <input
                // biome-ignore lint/a11y/noAutofocus: desktop popovers keep keyboard-first filtering; mobile avoids opening the keyboard on entry
                autoFocus={!mobileSheet}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={onSearchKey}
                placeholder={
                  view === 'projects'
                    ? 'Search projects or paste a path…'
                    : view === 'worktrees'
                      ? 'Filter worktrees…'
                      : 'Filter this folder or paste a path…'
                }
                aria-label="search directories"
                name="directory-search"
                className="min-w-0 flex-1 bg-transparent text-base text-ink placeholder:text-ink-ghost focus:outline-none md:text-[12px]"
              />
            </div>

            {/* validation/error (shown in the projects view; browse has its own) */}
            {view !== 'browse' && error ? (
              <div className="mx-4 mb-2 flex items-start gap-2 rounded-md bg-warn-muted px-3 py-2 font-sans text-base text-warn md:mx-0 md:text-[11px]">
                <AlertCircle className="size-4 shrink-0" aria-hidden />
                <span>{error}</span>
              </div>
            ) : null}

            {/* body */}
            {view === 'worktrees' ? (
              <div
                className={cn(
                  'space-y-1 overflow-y-auto px-4 pb-2 md:px-0',
                  embedded ? 'min-h-0 flex-1' : 'max-h-[280px]',
                )}
              >
                {wtLoading ? (
                  <div className="rounded-md bg-surface px-3 py-4 font-sans text-base text-ink-faint md:text-[11px]">
                    Loading worktrees…
                  </div>
                ) : wtError ? (
                  <div className="flex items-start gap-2 rounded-md bg-warn-muted px-3 py-3 font-sans text-base text-warn md:text-[11px]">
                    <AlertCircle className="size-4 shrink-0" aria-hidden />
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
                        className="flex min-h-14 w-full items-start gap-3 rounded-md px-3 py-2.5 text-left hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-10 md:gap-2 md:py-2"
                      >
                        <GitBranch
                          className="size-4 shrink-0 text-ink-faint"
                          aria-hidden
                        />
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span className="flex min-w-0 items-center gap-1.5 font-sans text-base md:text-[12px]">
                            <span className="truncate text-ink">
                              {wt.branch}
                            </span>
                            <span className="shrink-0 rounded-sm bg-surface px-1.5 py-0.5 font-mono text-[10px] text-ink-ghost tabular-nums">
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
                          <span className="truncate font-mono text-sm text-ink-ghost md:text-[10px]">
                            {wt.path}
                          </span>
                        </span>
                        <span className="flex shrink-0 items-center gap-1.5 font-sans text-sm md:text-[10px]">
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
                  <div className="rounded-md bg-surface px-3 py-4 font-sans text-base text-ink-faint md:text-[11px]">
                    {q
                      ? 'No matching worktrees.'
                      : 'No managed worktrees yet. Create one with worktree::create.'}
                  </div>
                )}
              </div>
            ) : view === 'projects' ? (
              <div
                className={cn(
                  'space-y-1 overflow-y-auto px-4 pb-2 md:px-0',
                  embedded ? 'min-h-0 flex-1' : 'max-h-[280px]',
                )}
              >
                {isAbsPath(query) ? (
                  <button
                    type="button"
                    disabled={validating !== null}
                    onClick={() => void validateAndSelect(query)}
                    className="flex min-h-14 w-full items-center gap-3 rounded-md bg-accent-muted px-3 py-2.5 text-left font-sans text-base text-accent hover:bg-surface-selected focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-10 md:gap-2 md:py-2 md:text-[12px]"
                  >
                    <CornerDownLeft className="size-4 shrink-0" aria-hidden />
                    <span className="truncate font-mono">
                      Use {query.trim()}
                    </span>
                  </button>
                ) : null}

                {showDefaultRow || filteredProjects.length > 0 ? (
                  <p className="px-1 pt-2 pb-1 font-sans text-base font-medium text-ink-faint md:text-[11px]">
                    Current and recent
                  </p>
                ) : null}

                {showDefaultRow && defaultDir ? (
                  <button
                    type="button"
                    disabled={validating !== null}
                    onClick={() => void validateAndSelect(defaultDir)}
                    aria-current={defaultDir === value ? 'true' : undefined}
                    className={cn(
                      'flex min-h-14 w-full items-center gap-3 rounded-md px-3 py-2.5 text-left hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-10 md:gap-2 md:py-2',
                      defaultDir === value && 'bg-accent-muted',
                    )}
                    title={`${defaultDir} — the folder the stack was started from`}
                  >
                    <Folder
                      className={cn(
                        'size-4 shrink-0',
                        defaultDir === value ? 'text-accent' : 'text-ink-faint',
                      )}
                      aria-hidden
                    />
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span
                        className={cn(
                          'truncate font-sans text-base font-medium md:text-[12px]',
                          defaultDir === value ? 'text-accent' : 'text-ink',
                        )}
                      >
                        {basename(defaultDir)}
                      </span>
                      <span className="truncate font-mono text-sm text-ink-ghost md:text-[10px]">
                        {parentDisplay(defaultDir)}
                      </span>
                    </span>
                    <span className="shrink-0 rounded-sm bg-surface-active px-1.5 py-0.5 font-sans text-sm text-ink-faint md:text-[9px]">
                      default
                    </span>
                    {defaultDir === value ? (
                      <Check
                        className="size-4 shrink-0 text-accent"
                        aria-hidden
                      />
                    ) : null}
                  </button>
                ) : null}

                {filteredProjects.map((p) => {
                  const isSelected = p === value
                  return (
                    <div
                      key={p}
                      className={cn(
                        'group flex items-center gap-1 rounded-md pr-1 hover:bg-surface-hover',
                        isSelected && 'bg-accent-muted',
                      )}
                    >
                      <button
                        type="button"
                        disabled={validating !== null}
                        onClick={() => void validateAndSelect(p)}
                        aria-current={isSelected ? 'true' : undefined}
                        className="flex min-h-14 min-w-0 flex-1 items-center gap-3 rounded-md px-3 py-2.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-10 md:gap-2 md:py-2"
                        title={p}
                      >
                        <Folder
                          className={cn(
                            'size-4 shrink-0',
                            isSelected ? 'text-accent' : 'text-ink-faint',
                          )}
                          aria-hidden
                        />
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span
                            className={cn(
                              'truncate font-sans text-base font-medium md:text-[12px]',
                              isSelected ? 'text-accent' : 'text-ink',
                            )}
                          >
                            {basename(p)}
                          </span>
                          <span className="truncate font-mono text-sm text-ink-ghost md:text-[10px]">
                            {parentDisplay(p)}
                          </span>
                        </span>
                        {isSelected ? (
                          <Check
                            className="size-4 shrink-0 text-accent"
                            aria-hidden
                          />
                        ) : null}
                      </button>
                      <button
                        type="button"
                        aria-label={`forget ${p}`}
                        title="forget this project"
                        onClick={() => forget(p)}
                        className="relative flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-ghost hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:size-8 md:opacity-0 md:group-hover:opacity-100 md:group-focus-within:opacity-100"
                      >
                        <span
                          className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                          aria-hidden="true"
                        />
                        <X className="size-4 shrink-0 md:size-3" aria-hidden />
                      </button>
                    </div>
                  )
                })}

                {filteredProjects.length === 0 &&
                !showDefaultRow &&
                !isAbsPath(query) ? (
                  <div className="rounded-md bg-surface px-3 py-4 font-sans text-base text-ink-faint md:text-[11px]">
                    {projects.length === 0
                      ? 'No recent projects yet.'
                      : 'No matching projects.'}{' '}
                    Browse to add one.
                  </div>
                ) : null}

                <div className="pt-2">
                  <button
                    type="button"
                    onClick={() => void enterBrowse()}
                    className="flex min-h-14 w-full items-center gap-3 rounded-md bg-surface px-3 py-2.5 text-left font-sans text-base font-medium text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-10 md:gap-2 md:py-2 md:text-[12px]"
                  >
                    <FolderPlus className="size-4 shrink-0" aria-hidden />
                    Browse folders
                  </button>
                </div>
              </div>
            ) : (
              <div className={cn(embedded && 'flex min-h-0 flex-1 flex-col')}>
                {/* browse header */}
                <div className="mx-4 mb-2 flex min-h-14 shrink-0 items-center justify-between gap-2 rounded-md bg-surface p-1 md:mx-0 md:min-h-10">
                  <div className="flex min-w-0 flex-1 items-center gap-1 font-sans text-base text-ink-faint md:text-[11px]">
                    <button
                      type="button"
                      aria-label="back to projects"
                      onClick={() => {
                        setView('projects')
                        setProjects(loadRecentProjects())
                        setQuery('')
                        setError(null)
                      }}
                      className="relative flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:size-8"
                    >
                      <ArrowLeft className="size-4 shrink-0" aria-hidden />
                    </button>
                    {path ? (
                      <button
                        type="button"
                        aria-label="up one level"
                        onClick={goUp}
                        className="relative flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:size-8"
                      >
                        <ChevronUp className="size-4 shrink-0" aria-hidden />
                      </button>
                    ) : null}
                    <span className="truncate font-mono">
                      {path ?? 'roots'}
                    </span>
                  </div>
                  {path ? (
                    <button
                      type="button"
                      disabled={validating !== null}
                      onClick={() => void validateAndSelect(path)}
                      className="inline-flex min-h-12 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-sm bg-accent py-2 pr-3 pl-2 font-sans text-base font-medium text-accent-fg hover:bg-accent-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-rule-focus disabled:opacity-50 md:min-h-8 md:text-[11px]"
                    >
                      <Check className="size-4 shrink-0" aria-hidden />
                      Use folder
                    </button>
                  ) : null}
                </div>

                <div
                  className={cn(
                    'space-y-1 overflow-y-auto px-4 pb-2 md:px-0',
                    embedded ? 'min-h-0 flex-1' : 'max-h-[260px]',
                  )}
                >
                  {isAbsPath(query) ? (
                    <button
                      type="button"
                      onClick={() => void jumpTo(query)}
                      className="flex min-h-14 w-full items-center gap-3 rounded-md bg-accent-muted px-3 py-2.5 text-left font-sans text-base text-accent hover:bg-surface-selected focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-10 md:gap-2 md:py-2 md:text-[12px]"
                    >
                      <CornerDownLeft className="size-4 shrink-0" aria-hidden />
                      <span className="truncate font-mono">
                        Go to {query.trim()}
                      </span>
                    </button>
                  ) : null}

                  {loading ? (
                    <div className="rounded-md bg-surface px-3 py-4 font-sans text-base text-ink-faint md:text-[11px]">
                      Loading folders…
                    </div>
                  ) : error ? (
                    <div className="flex items-start gap-2 rounded-md bg-warn-muted px-3 py-3 font-sans text-base text-warn md:text-[11px]">
                      <AlertCircle className="size-4 shrink-0" aria-hidden />
                      <span>{error}</span>
                    </div>
                  ) : path === null ? (
                    // roots list (only when multiple roots)
                    (roots ?? []).map((r) => (
                      <button
                        key={r}
                        type="button"
                        onClick={() => enterRoot(r)}
                        className="flex min-h-14 w-full items-center gap-3 rounded-md px-3 py-2.5 text-left font-sans text-base text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-10 md:gap-2 md:py-2 md:text-[12px]"
                      >
                        <FolderOpen
                          className="size-4 shrink-0 text-ink-faint"
                          aria-hidden
                        />
                        <span className="truncate font-mono">{r}</span>
                      </button>
                    ))
                  ) : filteredDirs.length > 0 ? (
                    filteredDirs.map((d) => (
                      <button
                        key={d}
                        type="button"
                        onClick={() => enterDir(d)}
                        className="flex min-h-14 w-full items-center gap-3 rounded-md px-3 py-2.5 text-left font-sans text-base text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-10 md:gap-2 md:py-2 md:text-[12px]"
                      >
                        <Folder
                          className="size-4 shrink-0 text-ink-faint"
                          aria-hidden
                        />
                        <span className="truncate font-mono">
                          {basename(d)}
                        </span>
                      </button>
                    ))
                  ) : (
                    <div className="rounded-md bg-surface px-3 py-4 font-sans text-base text-ink-faint md:text-[11px]">
                      {q
                        ? 'No matching subfolders.'
                        : 'No subfolders. You can use this folder.'}
                    </div>
                  )}
                </div>
              </div>
            )}

            {/* Static discoverability footnote — same line in both views, per
              the filesystem-access spec §6: sets expectations before the first
              grant prompt ever fires. */}
            <div className="shrink-0 px-4 pt-2 md:px-0 md:pt-1">
              <div className="rounded-md bg-surface px-3 py-2 font-sans text-base text-pretty text-ink-faint md:font-mono md:text-[10px]">
                The agent can use the chosen folder freely. It asks before
                accessing anything outside it.
              </div>
            </div>
          </div>
        </DirectoryPickerPortal>
      ) : null}
    </div>
  )
}

function DirectoryPickerPortal({
  enabled,
  children,
}: {
  enabled: boolean
  children: ReactNode
}) {
  if (!enabled || typeof document === 'undefined') return children
  return createPortal(children, document.body)
}
