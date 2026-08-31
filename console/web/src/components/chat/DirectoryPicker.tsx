import * as PopoverPrimitive from '@radix-ui/react-popover'
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
  type RefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import { StatusDot } from '@/components/ui/StatusDot'
import { useMediaQuery } from '@/hooks/use-media-query'
import { getIiiClient } from '@/lib/iii-client'
import { loadRecentProjects, removeRecentProject } from '@/lib/storage'
import { PortalScope } from '@/lib/ui-scope'
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
   * no longer validates against the live shell). The message is shown only
   * when the user opens the picker.
   */
  externalError?: string | null
  /**
   * The stack's default working directory (harness launch folder). Pinned at
   * the top of the projects view: unlike recents it is never forgettable and
   * survives re-scoping away, so the launch folder stays one click away.
   * Deduped against the recents list.
   */
  defaultDir?: string | null
  /** Show the worktrees tab next to directory browsing. */
  worktrees?: WorktreePickerOptions
  className?: string
  /** Compact text-only trigger used when the picker is part of a sentence. */
  triggerAppearance?: 'default' | 'inline'
  /** Trigger copy while no directory has been resolved yet. */
  emptyLabel?: string
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
  triggerAppearance = 'default',
  emptyLabel = 'Choose directory',
  presentation = 'trigger',
  onSelect,
}: DirectoryPickerProps) {
  const embedded = presentation === 'embedded'
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
  const triggerRef = useRef<HTMLButtonElement>(null)
  const mobileSheet = useMediaQuery('(max-width: 767px)')

  const openPanel = useCallback(() => {
    setProjects(loadRecentProjects())
    setView('projects')
    setQuery('')
    setError(externalError ?? null)
    setOpen(true)
  }, [externalError])

  useEffect(() => {
    if (!embedded) return
    setProjects(loadRecentProjects())
    setView('projects')
    setQuery('')
    setError(externalError ?? null)
  }, [embedded, externalError])

  // Keep an externally detected error available for an explicitly opened
  // picker without interrupting the current chat with a surprise dialog.
  useEffect(() => {
    if (!externalError || locked || disabled) return
    setError(externalError)
  }, [externalError, locked, disabled])

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

  const label = value ? basename(value) : emptyLabel

  if (locked && !embedded) {
    return (
      <span
        className={cn(
          'inline-flex items-center gap-1 px-2 py-1 font-sans text-[11px] text-ink-faint',
          className,
        )}
        title={value ?? 'no working directory'}
      >
        <Folder size={16} aria-hidden />
        <span className="max-w-[160px] truncate">{label}</span>
      </span>
    )
  }

  return (
    <div
      className={cn(
        embedded
          ? 'flex h-full min-h-0 w-full flex-col'
          : 'relative inline-flex min-w-0',
        className,
      )}
    >
      {!embedded ? (
        <button
          ref={triggerRef}
          type="button"
          disabled={disabled}
          aria-label={
            triggerAppearance === 'inline'
              ? `select project folder, current folder: ${label}`
              : 'working directory'
          }
          aria-haspopup="dialog"
          aria-expanded={open}
          title={value ?? 'choose a working directory'}
          onClick={() => (open ? setOpen(false) : openPanel())}
          className={cn(
            triggerAppearance === 'inline'
              ? 'relative inline-flex min-w-0 h-6.5 items-baseline border-dashed border-b border-ink-faint/50 px-0.5 font-sans font-medium text-ink hover:border-ink hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50'
              : 'inline-flex h-12 min-w-0 items-center gap-2 rounded-sm border border-transparent bg-transparent px-3 font-sans text-base text-ink-faint hover:border-ink hover:bg-surface-hover hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 sm:h-9 sm:text-[13px]',
            externalError ? 'text-warn' : value ? 'text-ink' : 'text-ink-faint',
          )}
        >
          {triggerAppearance === 'default' ? (
            <Folder aria-hidden className="size-4 shrink-0" />
          ) : null}
          <span className="min-w-0 max-w-[18rem] truncate">{label}</span>
          {triggerAppearance === 'inline' ? (
            <span
              className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            />
          ) : null}
        </button>
      ) : null}

      <DirectoryPickerSurface
        open={open}
        embedded={embedded}
        mobileSheet={mobileSheet}
        onOpenChange={setOpen}
        triggerRef={triggerRef}
        alignToInlineTrigger={triggerAppearance === 'inline'}
      >
        {/* section tabs (only with the worktree worker present) */}
        {worktrees?.enabled ? (
          <div
            role="tablist"
            className="mx-4 mb-3 flex shrink-0 gap-1 rounded-md bg-surface p-1 font-sans text-base md:mx-0 md:mb-1 md:text-[12px]"
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
        <div className="mx-4 mb-3 flex min-h-12 shrink-0 items-center gap-2 rounded-md bg-surface px-3 py-2 focus-within:ring-2 focus-within:ring-rule-focus md:mx-0 md:mb-1 md:min-h-8 md:px-2.5 md:py-1">
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
              embedded ? 'min-h-0 flex-1' : 'max-h-[220px]',
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
                    className="flex min-h-14 w-full items-center gap-3 rounded-md px-3 py-2.5 text-left hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-9 md:gap-2 md:py-1.5"
                  >
                    <GitBranch
                      className="size-4 shrink-0 text-ink-faint"
                      aria-hidden
                    />
                    <span className="flex min-w-0 flex-1 items-center gap-1.5 font-sans text-base md:text-[12px]">
                      <span className="truncate text-ink">{wt.branch}</span>
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
              embedded ? 'min-h-0 flex-1' : 'max-h-[220px]',
            )}
          >
            {isAbsPath(query) ? (
              <button
                type="button"
                disabled={validating !== null}
                onClick={() => void validateAndSelect(query)}
                className="flex min-h-14 w-full items-center gap-3 rounded-md bg-accent-muted px-3 py-2.5 text-left font-sans text-base text-accent hover:bg-surface-selected focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-9 md:gap-2 md:py-1.5 md:text-[12px]"
              >
                <CornerDownLeft className="size-4 shrink-0" aria-hidden />
                <span className="truncate font-mono">Use {query.trim()}</span>
              </button>
            ) : null}

            {showDefaultRow || filteredProjects.length > 0 ? (
              <p className="px-1 pt-2 pb-1 font-sans text-base font-medium text-ink-faint md:pt-1 md:text-[11px]">
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
                  'flex min-h-14 w-full items-center gap-3 rounded-md px-3 py-2.5 text-left hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-9 md:gap-2 md:py-1.5',
                  defaultDir === value && 'bg-surface-selected',
                )}
                title={`${defaultDir} — the folder the stack was started from`}
              >
                <Folder
                  className={cn(
                    'size-4 shrink-0',
                    defaultDir === value ? 'text-ink' : 'text-ink-faint',
                  )}
                  aria-hidden
                />
                <span className="min-w-0 flex-1 truncate font-sans text-base font-medium text-ink md:text-[12px]">
                  {basename(defaultDir)}
                </span>
                {defaultDir === value ? (
                  <Check className="size-4 shrink-0 text-ink" aria-hidden />
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
                    isSelected && 'bg-surface-selected',
                  )}
                >
                  <button
                    type="button"
                    disabled={validating !== null}
                    onClick={() => void validateAndSelect(p)}
                    aria-current={isSelected ? 'true' : undefined}
                    className="flex min-h-14 min-w-0 flex-1 items-center gap-3 rounded-md px-3 py-2.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-9 md:gap-2 md:py-1.5"
                    title={p}
                  >
                    <Folder
                      className={cn(
                        'size-4 shrink-0',
                        isSelected ? 'text-ink' : 'text-ink-faint',
                      )}
                      aria-hidden
                    />
                    <span className="min-w-0 flex-1 truncate font-sans text-base font-medium text-ink md:text-[12px]">
                      {basename(p)}
                    </span>
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
                    <X className="size-4 shrink-0 md:size-4" aria-hidden />
                  </button>
                  {isSelected ? (
                    <span className="flex size-12 shrink-0 items-center justify-center md:size-8">
                      <Check className="size-4 shrink-0 text-ink" aria-hidden />
                    </span>
                  ) : null}
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
                className="flex min-h-14 w-full items-center gap-3 rounded-md bg-surface px-3 py-2.5 text-left font-sans text-base font-medium text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-9 md:gap-2 md:py-1.5 md:text-[12px]"
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
                <span className="truncate font-mono">{path ?? 'roots'}</span>
              </div>
              {path ? (
                <button
                  type="button"
                  disabled={validating !== null}
                  onClick={() => void validateAndSelect(path)}
                  className="inline-flex min-h-12 shrink-0 items-center gap-1.5 rounded-sm bg-accent py-2 pr-3 pl-2 font-sans text-base font-medium whitespace-nowrap text-accent-fg hover:bg-accent-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-rule-focus disabled:opacity-50 md:min-h-8 md:text-[11px]"
                >
                  <Check className="size-4 shrink-0" aria-hidden />
                  Use folder
                </button>
              ) : null}
            </div>

            <div
              className={cn(
                'space-y-1 overflow-y-auto px-4 pb-2 md:px-0',
                embedded ? 'min-h-0 flex-1' : 'max-h-[220px]',
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
                    <span className="truncate font-mono">{basename(d)}</span>
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
      </DirectoryPickerSurface>
    </div>
  )
}

function DirectoryPickerSurface({
  open,
  embedded,
  mobileSheet,
  onOpenChange,
  triggerRef,
  alignToInlineTrigger,
  children,
}: {
  open: boolean
  embedded: boolean
  mobileSheet: boolean
  onOpenChange: (open: boolean) => void
  triggerRef: RefObject<HTMLButtonElement | null>
  alignToInlineTrigger: boolean
  children: ReactNode
}) {
  if (embedded) {
    return (
      // biome-ignore lint/a11y/useSemanticElements: this flex surface owns scroll layout and cannot use fieldset's rendering semantics
      <div
        role="group"
        aria-label="select working directory"
        className="flex min-h-0 flex-1 flex-col overflow-hidden bg-transparent"
      >
        {children}
      </div>
    )
  }

  if (mobileSheet) {
    return (
      <BottomSheet open={open} onOpenChange={onOpenChange}>
        <BottomSheetContent
          heading="Working directory"
          closeLabel="Close project picker"
        >
          <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
            {children}
          </div>
        </BottomSheetContent>
      </BottomSheet>
    )
  }

  return (
    <PopoverPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <PopoverPrimitive.Anchor
        virtualRef={triggerRef as RefObject<HTMLButtonElement>}
      />
      <PopoverPrimitive.Portal>
        <PortalScope>
          <PopoverPrimitive.Content
            side="top"
            align={alignToInlineTrigger ? 'center' : 'end'}
            sideOffset={8}
            collisionPadding={12}
            sticky="always"
            role="dialog"
            aria-label="select working directory"
            className="iii-ui-motion-dropdown z-50 max-h-[var(--radix-popover-content-available-height)] w-[min(360px,calc(100vw-24px))] overflow-y-auto overscroll-contain rounded-lg border border-edge bg-panel-raised p-1.5 shadow-floating"
          >
            {children}
          </PopoverPrimitive.Content>
        </PortalScope>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  )
}
