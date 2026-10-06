import * as PopoverPrimitive from '@radix-ui/react-popover'
import {
  AlertCircle,
  Check,
  CornerDownLeft,
  Folder,
  GitBranch,
  Pencil,
  Plus,
  Save,
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
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/Dialog'
import { StatusDot } from '@/components/ui/StatusDot'
import { DESKTOP_POINTER_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import {
  deleteHarnessProject,
  type HarnessProject,
  listHarnessProjects,
  upsertHarnessProject,
} from '@/lib/backend/projects'
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
import { basename, FolderBrowser, isAbsPath, parentOf } from './FolderBrowser'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * Per-session working-directory picker, project-switcher style.
 *
 * Opens to your harness projects (most-recent first) — pick one in a click,
 * or "Add project" to browse for a new directory (`FolderBrowser`: a dialog
 * with parent columns on desktop, the same sheet on phones). The search box
 * filters projects live; pasting an absolute path selects it. Every
 * selection — pasted, remembered, or browsed — is
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
   * the top of the projects view: it is never forgettable and survives
   * re-scoping away, so the launch folder stays one click away. Deduped
   * against the rest of the project list.
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
  /** Which edge of the default trigger the popover lines up with. */
  popoverAlign?: 'start' | 'end'
}

function withProject(
  projects: HarnessProject[],
  project: HarnessProject,
): HarnessProject[] {
  return [
    project,
    ...projects.filter((item) => item.path !== project.path),
  ].sort((a, b) => b.last_used_at - a.last_used_at)
}

/** Recent-project entry with selection, inline renaming, and optional removal. */
function ProjectRow({
  project,
  selected,
  hint,
  disabled,
  renaming,
  draftName,
  onDraftNameChange,
  onSelect,
  onBeginRename,
  onCancelRename,
  onSaveRename,
  onForget,
}: {
  project: HarnessProject
  selected: boolean
  /** Faint trailing text: "default" for the pinned row, the parent folder
      when two projects share a name. */
  hint?: string
  disabled: boolean
  renaming: boolean
  draftName: string
  onDraftNameChange: (name: string) => void
  onSelect: () => void
  onBeginRename: () => void
  onCancelRename: () => void
  onSaveRename: () => void
  onForget?: () => void
}) {
  if (renaming) {
    return (
      <form
        className="flex min-h-14 min-w-0 items-center gap-2 rounded-md bg-surface-selected p-1.5 md:min-h-8 md:p-1"
        onSubmit={(event) => {
          event.preventDefault()
          onSaveRename()
        }}
      >
        <Folder className="size-4 shrink-0 text-ink-faint" aria-hidden />
        <input
          // biome-ignore lint/a11y/noAutofocus: rename starts from an explicit pointer/keyboard action
          autoFocus
          name="project-name"
          aria-label={`name for ${project.path}`}
          value={draftName}
          onChange={(event) => onDraftNameChange(event.target.value)}
          className="min-w-0 flex-1 rounded-sm bg-panel-raised px-2 py-1.5 font-sans text-base text-ink outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:py-1 md:text-[12px]"
        />
        <button
          type="submit"
          disabled={disabled}
          aria-label={`save name for ${project.path}`}
          className="relative flex size-10 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:size-7"
        >
          <Save className="size-4 shrink-0" aria-hidden />
          <span
            className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
        </button>
        <button
          type="button"
          onClick={onCancelRename}
          aria-label="cancel rename"
          className="relative flex size-10 shrink-0 items-center justify-center rounded-sm text-ink-ghost hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:size-7"
        >
          <X className="size-4 shrink-0" aria-hidden />
          <span
            className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
        </button>
      </form>
    )
  }

  return (
    <div
      className={cn(
        'group flex min-w-0 items-center gap-1 rounded-md pr-1 hover:bg-surface-hover',
        selected && 'bg-surface-selected',
      )}
    >
      <button
        type="button"
        disabled={disabled}
        onClick={onSelect}
        aria-current={selected ? 'true' : undefined}
        className="flex min-h-14 min-w-0 flex-1 items-center gap-3 rounded-md px-3 py-2.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-8 md:gap-2 md:px-2 md:py-1"
        title={project.path}
      >
        <Folder
          className={cn(
            'size-4 shrink-0',
            selected ? 'text-ink' : 'text-ink-faint',
          )}
          aria-hidden
        />
        <span className="flex min-w-0 flex-1 items-baseline gap-2 font-sans text-base md:text-[12px]">
          <span className="min-w-0 truncate font-medium text-ink">
            {project.name}
          </span>
          {hint ? (
            <span className="min-w-0 truncate text-ink-ghost">{hint}</span>
          ) : null}
        </span>
      </button>
      <button
        type="button"
        aria-label={`rename ${project.name}`}
        title="rename project"
        onClick={onBeginRename}
        className="relative flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-ghost hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:size-7 md:opacity-0 md:group-hover:opacity-100 md:group-focus-within:opacity-100"
      >
        <span
          className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
          aria-hidden="true"
        />
        <Pencil className="size-4 shrink-0" aria-hidden />
      </button>
      {onForget ? (
        <button
          type="button"
          aria-label={`forget ${project.name}`}
          title="forget this project"
          onClick={onForget}
          className="relative flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-ghost hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:size-7 md:opacity-0 md:group-hover:opacity-100 md:group-focus-within:opacity-100"
        >
          <span
            className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
          <X className="size-4 shrink-0" aria-hidden />
        </button>
      ) : null}
      {selected ? (
        <span className="flex size-12 shrink-0 items-center justify-center md:size-7">
          <Check className="size-4 shrink-0 text-ink" aria-hidden />
        </span>
      ) : null}
    </div>
  )
}

// Re-exported for existing consumers/tests; canonical home is lib/working-dir.
export {
  WORKSPACE_LIST_FUNCTION_ID,
  WORKSPACE_ROOTS_FUNCTION_ID,
  WORKSPACE_VALIDATE_FUNCTION_ID,
}

/** Select a validated directory from recent projects, folders, or managed worktrees. */
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
  emptyLabel = 'Choose project',
  presentation = 'trigger',
  onSelect,
  popoverAlign = 'end',
}: DirectoryPickerProps) {
  const embedded = presentation === 'embedded'
  const [open, setOpen] = useState(false)
  const [view, setView] = useState<'projects' | 'browse' | 'worktrees'>(
    'projects',
  )
  const [projects, setProjects] = useState<HarnessProject[]>([])
  const [projectsLoading, setProjectsLoading] = useState(true)
  const [query, setQuery] = useState('')
  const [renamingPath, setRenamingPath] = useState<string | null>(null)
  const [projectName, setProjectName] = useState('')
  // desktop "Add project" dialog (phones browse inside the sheet instead)
  const [browseOpen, setBrowseOpen] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [validating, setValidating] = useState<string | null>(null)
  // worktrees state
  const [wtRows, setWtRows] = useState<WorktreeInfo[]>([])
  const [wtLoading, setWtLoading] = useState(false)
  const [wtError, setWtError] = useState<string | null>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const mobileSheet = useMediaQuery('(max-width: 767px)')
  const pointerFine = useMediaQuery(DESKTOP_POINTER_QUERY)
  const browseInDialog = !embedded && !mobileSheet

  const refreshProjects = useCallback(async () => {
    setProjectsLoading(true)
    try {
      setProjects(await listHarnessProjects())
    } catch (err) {
      setError(`can't load projects — ${errMsg(err)}`)
    } finally {
      setProjectsLoading(false)
    }
  }, [])

  const openPanel = useCallback(() => {
    setView('projects')
    setQuery('')
    setRenamingPath(null)
    setError(externalError ?? null)
    setOpen(true)
    void refreshProjects()
  }, [externalError, refreshProjects])

  useEffect(() => {
    void refreshProjects()
  }, [refreshProjects])

  useEffect(() => {
    if (!embedded) return
    setView('projects')
    setQuery('')
    setRenamingPath(null)
    setError(externalError ?? null)
    void refreshProjects()
  }, [embedded, externalError, refreshProjects])

  // Keep an externally detected error available for an explicitly opened
  // picker without interrupting the current chat with a surprise dialog.
  useEffect(() => {
    if (!externalError || locked || disabled) return
    setError(externalError)
  }, [externalError, locked, disabled])

  const addProject = useCallback(() => {
    setError(null)
    setQuery('')
    if (browseInDialog) {
      setOpen(false)
      setBrowseOpen(true)
    } else setView('browse')
  }, [browseInDialog])

  const select = useCallback(
    (dir: string) => {
      onChange(dir)
      setOpen(false)
      setBrowseOpen(false)
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
      // input, so stored projects are stable across symlinks.
      const res = await validateWorkspaceDir(dir)
      if (res.ok) {
        try {
          const project = await upsertHarnessProject(res.path)
          setProjects((current) => withProject(current, project))
          setValidating(null)
          select(res.path)
        } catch (err) {
          setError(`can't save this project — ${errMsg(err)}`)
          setValidating(null)
        }
        return
      }
      setError(`can't use ${dir} — ${res.error}`)
      setValidating(null)
    },
    [select],
  )

  const forget = useCallback(async (dir: string) => {
    setValidating(dir)
    setError(null)
    try {
      await deleteHarnessProject(dir)
      setProjects((current) =>
        current.filter((project) => project.path !== dir),
      )
    } catch (err) {
      setError(`can't remove this project — ${errMsg(err)}`)
    } finally {
      setValidating(null)
    }
  }, [])

  const beginRename = useCallback((project: HarnessProject) => {
    setRenamingPath(project.path)
    setProjectName(project.name)
    setError(null)
  }, [])

  const saveProjectName = useCallback(async () => {
    if (!renamingPath) return
    setValidating(renamingPath)
    setError(null)
    try {
      const project = await upsertHarnessProject(renamingPath, projectName)
      setProjects((current) => withProject(current, project))
      setRenamingPath(null)
    } catch (err) {
      setError(`can't rename this project — ${errMsg(err)}`)
    } finally {
      setValidating(null)
    }
  }, [projectName, renamingPath])

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
  // The pinned default row replaces any identical catalog row (a user who
  // explicitly picked the default lands it in the catalog too — show it once).
  const filteredProjects = useMemo(
    () =>
      projects.filter(
        (project) =>
          project.path !== defaultDir &&
          (project.name.toLowerCase().includes(q) ||
            project.path.toLowerCase().includes(q)),
      ),
    [projects, q, defaultDir],
  )
  const defaultProject = projects.find((project) => project.path === defaultDir)
  // Two projects with the same name are told apart by their parent folder;
  // everything else stays a bare name so the list reads as a short menu.
  const ambiguousNames = useMemo(() => {
    const counts = new Map<string, number>()
    for (const project of projects) {
      counts.set(project.name, (counts.get(project.name) ?? 0) + 1)
    }
    return new Set(
      [...counts.entries()]
        .filter(([, count]) => count > 1)
        .map(([name]) => name),
    )
  }, [projects])
  const hintFor = (project: HarnessProject): string | undefined =>
    ambiguousNames.has(project.name)
      ? `…/${basename(parentOf(project.path))}`
      : undefined
  const showDefaultRow =
    !!defaultDir &&
    (q === '' ||
      defaultDir.toLowerCase().includes(q) ||
      defaultProject?.name.toLowerCase().includes(q) === true)
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
    if (e.key !== 'Enter' || !isAbsPath(query) || view !== 'projects') return
    e.preventDefault()
    void validateAndSelect(query)
  }

  const label = value
    ? (projects.find((project) => project.path === value)?.name ??
      basename(value))
    : emptyLabel

  const validationError = error ? (
    <div className="mb-2 flex items-start gap-2 rounded-md bg-warn-muted px-3 py-2 font-sans text-base text-warn md:text-[11px]">
      <AlertCircle className="size-4 shrink-0" aria-hidden />
      <span className="min-w-0 [overflow-wrap:anywhere]">{error}</span>
    </div>
  ) : null

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
          ? 'flex h-full min-h-0 min-w-0 w-full flex-col'
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
              : 'inline-flex h-12 min-w-0 items-center gap-2 rounded-sm border border-transparent bg-transparent px-3 font-sans text-base text-ink-faint hover:bg-surface-hover hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 sm:h-9 sm:text-[13px]',
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
        align={popoverAlign}
      >
        {/* section tabs (only with the worktree worker present) */}
        {worktrees?.enabled ? (
          <div
            role="tablist"
            className="mx-4 mb-3 flex shrink-0 gap-1 rounded-md bg-surface p-1 font-sans text-base md:mx-0 md:mb-1 md:p-0.5 md:text-[11px]"
          >
            <button
              type="button"
              role="tab"
              aria-selected={view !== 'worktrees'}
              onClick={() => {
                setView('projects')
                void refreshProjects()
                setQuery('')
                setError(null)
              }}
              className={cn(
                'min-h-12 flex-1 rounded-sm px-3 py-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-7 md:py-1',
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
                'min-h-12 flex-1 rounded-sm px-3 py-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-7 md:py-1',
                view === 'worktrees'
                  ? 'bg-panel-raised text-ink ring-1 ring-edge'
                  : 'text-ink-faint hover:bg-surface-hover hover:text-ink',
              )}
            >
              worktrees
            </button>
          </div>
        ) : null}

        {/* search — a boxed field on the mobile sheet, a bare line with a
            magnifier on desktop where the popover itself is the box;
            FolderBrowser brings its own */}
        {view === 'browse' ? null : (
          <div className="mx-4 mb-3 flex min-h-12 shrink-0 items-center gap-2 rounded-md bg-surface px-3 py-2 focus-within:ring-2 focus-within:ring-rule-focus md:mx-0 md:mb-1 md:min-h-8 md:rounded-none md:border-b md:border-rule-2 md:bg-transparent md:px-2 md:py-1 md:focus-within:ring-0">
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
                  : 'Filter worktrees…'
              }
              aria-label="search projects and directories"
              name="directory-search"
              className="min-w-0 flex-1 bg-transparent text-base text-ink placeholder:text-ink-ghost focus:outline-none md:text-[12px]"
            />
          </div>
        )}

        {/* body */}
        {view === 'worktrees' ? (
          <div
            className={cn(
              'min-w-0 space-y-1 overflow-x-hidden overflow-y-auto overscroll-contain px-4 pb-2 md:px-0',
              embedded || mobileSheet ? 'min-h-0 flex-1' : 'max-h-[220px]',
            )}
          >
            {validationError}
            {wtLoading ? (
              <div className="rounded-md bg-surface px-3 py-4 font-sans text-base text-ink-faint md:text-[11px]">
                Loading worktrees…
              </div>
            ) : wtError ? (
              <div className="flex items-start gap-2 rounded-md bg-warn-muted px-3 py-3 font-sans text-base text-warn md:text-[11px]">
                <AlertCircle className="size-4 shrink-0" aria-hidden />
                <span className="min-w-0 [overflow-wrap:anywhere]">
                  {wtError}
                </span>
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
                      <span className="shrink-0 rounded-sm bg-surface px-1.5 py-0.5 font-mono text-[11px] text-ink-ghost tabular-nums">
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
                          className="shrink-0 text-[11px] text-ink-faint tabular-nums"
                          title={`${ahead} commit(s) ahead of base`}
                        >
                          +{ahead}
                        </span>
                      ) : null}
                    </span>
                    <span className="flex shrink-0 items-center gap-1.5 font-sans text-sm md:text-[11px]">
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
              'min-w-0 space-y-1 overflow-x-hidden overflow-y-auto overscroll-contain px-4 pb-2 md:px-0',
              embedded || mobileSheet ? 'min-h-0 flex-1' : 'max-h-[220px]',
            )}
          >
            {validationError}
            {isAbsPath(query) ? (
              <button
                type="button"
                disabled={validating !== null}
                onClick={() => void validateAndSelect(query)}
                className="flex min-h-14 w-full items-center gap-3 rounded-md bg-surface-selected px-3 py-2.5 text-left font-sans text-base text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:opacity-50 md:min-h-9 md:gap-2 md:py-1.5 md:text-[12px]"
              >
                <CornerDownLeft className="size-4 shrink-0" aria-hidden />
                <span className="truncate font-mono">Use {query.trim()}</span>
              </button>
            ) : null}

            {projectsLoading ? (
              <div className="rounded-md bg-surface px-3 py-4 font-sans text-base text-ink-faint md:text-[11px]">
                Loading projects…
              </div>
            ) : null}

            {showDefaultRow && defaultDir ? (
              <ProjectRow
                project={
                  defaultProject ?? {
                    path: defaultDir,
                    name: basename(defaultDir),
                    last_used_at: 0,
                  }
                }
                selected={defaultDir === value}
                hint="default"
                disabled={validating !== null}
                renaming={renamingPath === defaultDir}
                draftName={projectName}
                onDraftNameChange={setProjectName}
                onSelect={() => void validateAndSelect(defaultDir)}
                onBeginRename={() =>
                  beginRename(
                    defaultProject ?? {
                      path: defaultDir,
                      name: basename(defaultDir),
                      last_used_at: 0,
                    },
                  )
                }
                onCancelRename={() => setRenamingPath(null)}
                onSaveRename={() => void saveProjectName()}
              />
            ) : null}

            {filteredProjects.map((project) => {
              const isSelected = project.path === value
              return (
                <ProjectRow
                  key={project.path}
                  project={project}
                  selected={isSelected}
                  hint={hintFor(project)}
                  disabled={validating !== null}
                  renaming={renamingPath === project.path}
                  draftName={projectName}
                  onDraftNameChange={setProjectName}
                  onSelect={() => void validateAndSelect(project.path)}
                  onBeginRename={() => beginRename(project)}
                  onCancelRename={() => setRenamingPath(null)}
                  onSaveRename={() => void saveProjectName()}
                  onForget={() => void forget(project.path)}
                />
              )
            })}

            {!projectsLoading &&
            filteredProjects.length === 0 &&
            !showDefaultRow &&
            !isAbsPath(query) ? (
              <div className="rounded-md bg-surface px-3 py-4 font-sans text-base text-ink-faint md:text-[11px]">
                {projects.length === 0
                  ? 'No projects yet.'
                  : 'No matching projects.'}{' '}
                Add one to get started.
              </div>
            ) : null}

            <div className="mt-1 border-t border-rule-2 pt-1">
              <button
                type="button"
                onClick={addProject}
                className="flex min-h-14 w-full items-center gap-3 rounded-md px-3 py-2.5 text-left font-sans text-base font-medium text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus md:min-h-8 md:gap-2 md:px-2 md:py-1 md:text-[12px]"
              >
                <Plus className="size-4 shrink-0 text-ink-faint" aria-hidden />
                Add project
              </button>
            </div>
          </div>
        ) : (
          <FolderBrowser
            wide={false}
            keyboard={false}
            busy={validating !== null}
            error={error}
            onUse={(dir) => void validateAndSelect(dir)}
            onBack={() => {
              setView('projects')
              void refreshProjects()
              setQuery('')
              setError(null)
            }}
          />
        )}
      </DirectoryPickerSurface>

      {browseInDialog ? (
        <Dialog open={browseOpen} onOpenChange={setBrowseOpen}>
          <DialogContent
            aria-describedby={undefined}
            className="flex h-[min(40rem,85vh)] w-[min(60rem,calc(100vw-2rem))] max-w-none flex-col overflow-hidden p-0"
            onOpenAutoFocus={(event) => {
              // No keyboard to type with: don't raise the on-screen one.
              if (!pointerFine) event.preventDefault()
            }}
            onCloseAutoFocus={(event) => {
              // "Add project" lived in the popover, which is gone.
              event.preventDefault()
              triggerRef.current?.focus()
            }}
          >
            <DialogTitle className="px-4 pt-4 pb-3">Add project</DialogTitle>
            <FolderBrowser
              wide
              keyboard={pointerFine}
              busy={validating !== null}
              error={error}
              onUse={(dir) => void validateAndSelect(dir)}
            />
          </DialogContent>
        </Dialog>
      ) : null}
    </div>
  )
}

/** Host picker content inline, in a mobile sheet, or in a desktop popover. */
function DirectoryPickerSurface({
  open,
  embedded,
  mobileSheet,
  onOpenChange,
  triggerRef,
  alignToInlineTrigger,
  align,
  children,
}: {
  open: boolean
  embedded: boolean
  mobileSheet: boolean
  onOpenChange: (open: boolean) => void
  triggerRef: RefObject<HTMLButtonElement | null>
  alignToInlineTrigger: boolean
  align: 'start' | 'end'
  children: ReactNode
}) {
  if (embedded) {
    return (
      // biome-ignore lint/a11y/useSemanticElements: this flex surface owns scroll layout and cannot use fieldset's rendering semantics
      <div
        role="group"
        aria-label="select working directory"
        className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-transparent"
      >
        {children}
      </div>
    )
  }

  if (mobileSheet) {
    return (
      <BottomSheet open={open} onOpenChange={onOpenChange}>
        <BottomSheetContent
          heading="Projects"
          closeLabel="Close project picker"
          className="h-[min(36rem,calc(100dvh-1.5rem))]"
        >
          <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
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
            align={alignToInlineTrigger ? 'center' : align}
            sideOffset={8}
            collisionPadding={12}
            sticky="always"
            role="dialog"
            aria-label="select working directory"
            className="iii-ui-motion-dropdown z-50 max-h-[var(--radix-popover-content-available-height)] w-[min(320px,calc(100vw-24px))] overflow-y-auto overscroll-contain rounded-lg border border-edge bg-panel-raised p-1 shadow-floating"
          >
            {children}
          </PopoverPrimitive.Content>
        </PortalScope>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  )
}
