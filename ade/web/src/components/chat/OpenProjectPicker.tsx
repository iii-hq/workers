import {
  AlertCircle,
  ChevronRight,
  Folder,
  FolderGit2,
  FolderOpen,
  FolderPlus,
  History,
  Loader2,
} from 'lucide-react'
import {
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { DESKTOP_POINTER_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import {
  type HarnessProject,
  listHarnessProjects,
  upsertHarnessProject,
} from '@/lib/backend/projects'
import { getIiiClient } from '@/lib/iii-client'
import { bindingMatchesEvent } from '@/lib/keybindings/bindings'
import {
  cachedProjectIndex,
  expandHome,
  type FolderListing,
  type IndexedFolder,
  listFolder,
  loadProjectIndex,
  type ProjectIndex,
  rankFolders,
  tildify,
} from '@/lib/project-index'
import { cn } from '@/lib/utils'
import { errMsg, validateWorkspaceDir } from '@/lib/working-dir'
import { basename, parentOf } from './FolderBrowser'

interface OpenProjectPickerProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The worker-echoed canonical path of the folder just added. */
  onAdded: (path: string) => void
}

interface Row {
  path: string
  name: string
  git: boolean
  /** Show the parent folder beside the name (rows from outside the listing). */
  showWhere?: boolean
  ranges?: Array<[number, number]>
}

interface Section {
  id: string
  label: string | null
  rows: Row[]
}

const RECENT_CHIPS = 4
/** New folder in the listed folder. Finder's ⇧⌘N opens Chrome's incognito
    window and can't be caught, so it is ⌥⌘N here. */
const NEW_FOLDER_BINDING = 'Mod+Alt+N'
const HOME_REPOS = 6
const ELSEWHERE = 6
const SEARCH_LIMIT = 30

const isPathText = (text: string) =>
  text.startsWith('/') || text.startsWith('~')

/** Footer key caps: hairline boxes that read as labels. */
const CAP =
  'h-5 min-w-5 rounded-[5px] border border-border bg-transparent px-1 font-inter text-[11px] font-medium text-muted-foreground shadow-none'

/**
 * "Add project": a path field first, the way Finder's Go to Folder and a
 * shell prompt work, with the folder it points at listed underneath.
 *
 * - The field holds a path (`~/conductor/repos/`). Typing narrows the
 *   folder's children and the best completion shows as ghost text, fish
 *   style; Tab or → takes it, Backspace over a `/` climbs a level.
 * - The listing under it is that folder: repositories first, each row
 *   opens on click and offers Add, the way Vercel's Import repository
 *   list does. A name that isn't in the folder is looked up across the
 *   background index (`lib/project-index`) under "Elsewhere".
 * - Recent projects sit as chips above the listing: one click adds.
 * - The footer always names the folder Add project will add.
 *
 * Adding validates the folder with the shell worker and remembers it as a
 * project before `onAdded` fires with the canonical path.
 */
export function OpenProjectPicker({
  open,
  onOpenChange,
  onAdded,
}: OpenProjectPickerProps) {
  const pointerFine = useMediaQuery(DESKTOP_POINTER_QUERY)
  const [text, setText] = useState('')
  const [index, setIndex] = useState<ProjectIndex | null>(cachedProjectIndex)
  const [recent, setRecent] = useState<HarnessProject[]>([])
  const [listings, setListings] = useState<
    Record<string, FolderListing | { error: string }>
  >({})
  const [active, setActive] = useState(0)
  const [busyPath, setBusyPath] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  // New folder: the name being typed in the listing's inline row (null when
  // the row is closed), and whether the mkdir is in flight.
  const [newFolder, setNewFolder] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)

  const home = index?.home ?? null
  const known = useMemo(
    () => new Map((index?.folders ?? []).map((f) => [f.path, f])),
    [index],
  )

  // Each open starts at home with fresh projects and index; a cached index
  // paints at once and a stale one is swapped in when its crawl lands.
  useEffect(() => {
    if (!open) return
    const startHome = cachedProjectIndex()?.home ?? null
    setText(startHome ? '~/' : '')
    setActive(0)
    setError(null)
    setListings({})
    setNewFolder(null)
    let live = true
    listHarnessProjects().then(
      (projects) => live && setRecent(projects),
      () => undefined,
    )
    loadProjectIndex().then(
      (next) => {
        if (!live) return
        setIndex(next)
        // The first open has no cached home yet: start there once known.
        if (!startHome && next.home) setText((t) => (t === '' ? '~/' : t))
      },
      () => undefined,
    )
    return () => {
      live = false
    }
  }, [open])

  // The field, split into the folder it names and the segment being typed.
  const pathMode = isPathText(text)
  const expanded = pathMode ? expandHome(text, home) : ''
  const cut = expanded.lastIndexOf('/')
  const dir = pathMode ? (cut <= 0 ? '/' : expanded.slice(0, cut)) : null
  const segment = pathMode ? expanded.slice(cut + 1) : text.trim()

  useEffect(() => {
    if (!open || !dir || listings[dir]) return
    let live = true
    listFolder(dir, known).then(
      (listing) =>
        live && setListings((current) => ({ ...current, [dir]: listing })),
      (err) =>
        live &&
        setListings((current) => ({
          ...current,
          [dir]: { error: errMsg(err) },
        })),
    )
    return () => {
      live = false
    }
  }, [open, dir, listings, known])

  const listing = dir ? listings[dir] : undefined
  const folderListing = listing && 'folders' in listing ? listing : null
  const listingError = listing && 'error' in listing ? listing.error : null

  const sections: Section[] = useMemo(() => {
    const s = segment.toLowerCase()
    const pool = new Map<string, IndexedFolder>()
    for (const p of recent) {
      pool.set(p.path, {
        path: p.path,
        name: p.name || basename(p.path),
        git: known.get(p.path)?.git ?? false,
      })
    }
    for (const f of index?.folders ?? [])
      if (!pool.has(f.path)) pool.set(f.path, f)

    if (!pathMode) {
      if (!s) return []
      const matches = rankFolders([...pool.values()], s).slice(0, SEARCH_LIMIT)
      return [
        {
          id: 'search',
          label: null,
          rows: matches.map(({ folder, ranges }) => ({
            ...folder,
            showWhere: true,
            ranges,
          })),
        },
      ]
    }

    if (!folderListing) return []
    const children = folderListing.folders
      .filter((f) => !f.name.startsWith('.') || s.startsWith('.'))
      .map((f) => ({ ...f, git: f.git || (known.get(f.path)?.git ?? false) }))
    const starts = children.filter((f) => f.name.toLowerCase().startsWith(s))
    const contains = s
      ? children.filter(
          (f) =>
            !f.name.toLowerCase().startsWith(s) &&
            f.name.toLowerCase().includes(s),
        )
      : []
    const here = [...starts, ...contains]
      // Repositories lead: they are what people come here to add.
      .sort((a, b) => Number(b.git) - Number(a.git))
      .map((f) => {
        const at = f.name.toLowerCase().indexOf(s)
        return {
          ...f,
          ranges:
            s && at >= 0
              ? ([[at, at + s.length]] as Array<[number, number]>)
              : undefined,
        }
      })
    const out: Section[] = []

    // Home with nothing typed: the repositories the crawl found come first.
    if (!s && dir === home) {
      const repos = [...pool.values()]
        // Repos inside hidden folders (a tool's scratch space) are noise.
        .filter(
          (f) => f.git && parentOf(f.path) !== dir && !f.path.includes('/.'),
        )
        .sort((a, b) => a.name.localeCompare(b.name))
        .slice(0, HOME_REPOS)
      if (repos.length) {
        out.push({
          id: 'repos',
          label: 'Repositories',
          rows: repos.map((f) => ({ ...f, showWhere: true })),
        })
      }
    }
    if (here.length) {
      out.push({
        id: 'here',
        label: out.length ? `In ${tildify(dir ?? '/', home)}` : null,
        rows: here,
      })
    }
    // A name the folder doesn't have may live somewhere else.
    if (s && !s.startsWith('.')) {
      const inHere = new Set(here.map((r) => r.path))
      const elsewhere = rankFolders([...pool.values()], s)
        .filter(({ folder }) => !inHere.has(folder.path))
        .slice(0, ELSEWHERE)
      if (elsewhere.length) {
        out.push({
          id: 'elsewhere',
          label: 'Elsewhere',
          rows: elsewhere.map(({ folder, ranges }) => ({
            ...folder,
            showWhere: true,
            ranges,
          })),
        })
      }
    }
    return out
  }, [segment, recent, index, known, pathMode, folderListing, dir, home])

  const rows = useMemo(() => sections.flatMap((s) => s.rows), [sections])
  const activeIndex = rows.length ? Math.min(active, rows.length - 1) : -1
  const activeRow = activeIndex >= 0 ? rows[activeIndex] : null

  // Ghost completion: the first child whose name starts with the segment.
  const ghostFor =
    pathMode && segment && folderListing
      ? folderListing.folders.find((f) =>
          f.name.toLowerCase().startsWith(segment.toLowerCase()),
        )
      : undefined
  const ghost = ghostFor ? `${ghostFor.name.slice(segment.length)}/` : ''

  /** The folder "Add project" adds right now. */
  const target = activeRow?.path ?? (pathMode && !segment ? dir : null)
  const targetIsProject =
    activeRow?.git ?? (target === dir ? folderListing?.project : false)

  // biome-ignore lint/correctness/useExhaustiveDependencies: follow the highlight as it moves
  useEffect(() => {
    listRef.current
      ?.querySelector('[data-active="true"]')
      ?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex, rows])

  // A refused folder hands the keyboard back to the field.
  useEffect(() => {
    if (open && busyPath === null && pointerFine) inputRef.current?.focus()
  }, [open, busyPath, pointerFine])

  const add = useCallback(
    async (raw: string | null) => {
      if (!raw || busyPath) return
      const path = raw.trim().replace(/\/+$/, '') || '/'
      setError(null)
      setBusyPath(path)
      const res = await validateWorkspaceDir(path)
      if (!res.ok) {
        setError(`Can't add ${tildify(path, home)}: ${res.error}`)
        setBusyPath(null)
        return
      }
      try {
        await upsertHarnessProject(res.path)
      } catch (err) {
        setError(`Can't save this project: ${errMsg(err)}`)
        setBusyPath(null)
        return
      }
      setBusyPath(null)
      onOpenChange(false)
      onAdded(res.path)
    },
    [busyPath, home, onAdded, onOpenChange],
  )

  /** Point the field at a folder, ready to list its children. */
  const go = (path: string) => {
    const shown = tildify(path, home)
    setText(shown === '/' ? '/' : `${shown}/`)
    setActive(0)
    setError(null)
    if (pointerFine) inputRef.current?.focus()
  }

  /**
   * Make a folder inside the one being listed (the shell worker's
   * `shell::fs::mkdir`, held to the same workspace roots as the listing),
   * then step into it so "Add project" points at it.
   */
  const createFolder = async (raw: string) => {
    const name = raw.trim()
    if (!dir || !name || creating) return
    if (name === '.' || name === '..' || /[/\\]/.test(name)) {
      setError('A folder name can’t contain / or be . or ..')
      return
    }
    const path = dir === '/' ? `/${name}` : `${dir}/${name}`
    setError(null)
    setCreating(true)
    try {
      const client = await getIiiClient()
      const res = await client.trigger<{ code?: string; message?: string }>(
        'shell::fs::mkdir',
        { path },
      )
      if (res?.code) throw new Error(res.message ?? res.code)
    } catch (err) {
      setError(`Can't create ${tildify(path, home)}: ${errMsg(err)}`)
      setCreating(false)
      return
    }
    setCreating(false)
    setNewFolder(null)
    // Re-read the parent so the new folder is listed when you come back up.
    setListings(({ [dir]: _stale, ...rest }) => rest)
    go(path)
  }
  const startNewFolder = (name = '') => {
    if (!pathMode || !dir) return
    setError(null)
    setNewFolder(name)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return
    const atEnd = event.currentTarget.selectionStart === text.length
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault()
      void add(target)
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      if (!rows.length) return
      const step = event.key === 'ArrowDown' ? 1 : -1
      setActive((activeIndex + step + rows.length) % rows.length)
    } else if (event.key === 'Enter') {
      event.preventDefault()
      // A repository is a destination; any other folder is a way through.
      if (activeRow) {
        if (activeRow.git) void add(activeRow.path)
        else go(activeRow.path)
      } else if (pathMode && !segment) void add(dir)
    } else if (
      (event.key === 'Tab' && !event.shiftKey) ||
      (event.key === 'ArrowRight' && atEnd)
    ) {
      const into = ghostFor ?? activeRow
      if (!into) return
      event.preventDefault()
      go(into.path)
    } else if (
      event.key === 'Backspace' &&
      pathMode &&
      atEnd &&
      text.endsWith('/') &&
      text.length > 1 &&
      event.currentTarget.selectionEnd === event.currentTarget.selectionStart
    ) {
      // Backspace over a trailing slash climbs a level.
      event.preventDefault()
      go(parentOf(expandHome(text.slice(0, -1), home)))
    }
  }

  const crumbs = dir ? crumbsOf(dir, home) : []
  const loading = pathMode && listing === undefined
  const recentChips = recent.slice(0, RECENT_CHIPS)

  let note: ReactNode = null
  if (!loading && rows.length === 0) {
    note = listingError ? (
      <span className="flex items-start gap-2 text-left text-destructive">
        <AlertCircle className="mt-0.5 size-4 shrink-0" aria-hidden />
        <span className="min-w-0 [overflow-wrap:anywhere]">{listingError}</span>
      </span>
    ) : !pathMode && !segment ? (
      <>
        Start with <Mono>~/</Mono> or <Mono>/</Mono> to browse, or type a folder
        name to search.
      </>
    ) : segment && pathMode && dir ? (
      <span className="flex flex-col items-center gap-3">
        <span>No folder matches “{segment}”.</span>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="gap-1.5"
          disabled={creating}
          onClick={() => void createFolder(segment)}
        >
          <FolderPlus className="size-4" aria-hidden />
          Create “{segment}” here
        </Button>
      </span>
    ) : segment ? (
      <>No folder matches “{segment}”.</>
    ) : (
      <>No subfolders here. Add this folder, or go up with ⌫.</>
    )
  }

  let rowNumber = -1
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!busyPath) onOpenChange(next)
      }}
    >
      <DialogContent
        className="flex h-[min(34rem,calc(100dvh-2rem))] w-[min(38rem,calc(100vw-2rem))] max-w-none flex-col gap-0 overflow-hidden p-0 [--dialog-max-width:none]"
        initialFocus={pointerFine ? inputRef : () => false}
        onKeyDown={(event) => {
          if (bindingMatchesEvent(NEW_FOLDER_BINDING, event)) {
            event.preventDefault()
            startNewFolder()
          }
        }}
      >
        <div className="flex shrink-0 flex-col gap-1 px-5 pt-5 pr-12">
          <DialogTitle className="text-base font-medium">
            Add project
          </DialogTitle>
          <DialogDescription className="text-sm text-muted-foreground">
            Choose the folder a chat works in. Type a path, or pick a
            repository.
          </DialogDescription>
        </div>

        {/* The path field: the surface's one control. */}
        <div className="shrink-0 px-5 pt-4">
          <div className="relative flex h-11 items-center gap-2.5 rounded-xl border border-border bg-background px-3.5 transition-[border-color,box-shadow] duration-[var(--motion-duration-control)] focus-within:border-foreground/30 focus-within:ring-3 focus-within:ring-foreground/10">
            {busyPath ? (
              <Loader2
                className="size-4 shrink-0 animate-spin text-muted-foreground"
                aria-hidden
              />
            ) : (
              <FolderOpen
                className="size-4 shrink-0 text-muted-foreground"
                aria-hidden
              />
            )}
            <div className="relative min-w-0 flex-1">
              {/* Ghost text: the typed path, invisible, then the completion. */}
              {ghost ? (
                <div
                  aria-hidden
                  className="pointer-events-none absolute inset-0 flex items-center overflow-hidden font-mono text-sm whitespace-pre"
                >
                  <span className="invisible">{text}</span>
                  <span className="text-muted-foreground/60">{ghost}</span>
                </div>
              ) : null}
              <input
                ref={inputRef}
                value={text}
                onChange={(event) => {
                  setText(event.target.value)
                  setActive(0)
                  setError(null)
                }}
                onKeyDown={onKeyDown}
                disabled={busyPath !== null}
                placeholder="~/path/to/project or a folder name"
                aria-label="Folder path or name"
                role="combobox"
                aria-expanded="true"
                aria-controls="add-project-list"
                aria-autocomplete="both"
                aria-activedescendant={
                  activeIndex >= 0
                    ? `add-project-row-${activeIndex}`
                    : undefined
                }
                name="project-path"
                autoCapitalize="none"
                autoCorrect="off"
                autoComplete="off"
                spellCheck={false}
                className="relative h-10 w-full bg-transparent font-mono text-sm text-foreground placeholder:font-inter placeholder:text-muted-foreground focus:outline-none disabled:opacity-60"
              />
            </div>
            {ghost && pointerFine ? (
              <KeyCombo binding="Tab" capClassName={CAP} className="shrink-0" />
            ) : null}
          </div>
        </div>

        {error ? (
          <div
            role="alert"
            className="mx-5 mt-3 flex shrink-0 items-start gap-2 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive"
          >
            <AlertCircle className="mt-0.5 size-4 shrink-0" aria-hidden />
            <span className="min-w-0 [overflow-wrap:anywhere]">{error}</span>
          </div>
        ) : null}

        {recentChips.length ? (
          <div className="flex shrink-0 items-center gap-1.5 overflow-x-auto px-5 pt-3 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
            <span className="mr-1 shrink-0 text-xs font-medium text-muted-foreground">
              Recent
            </span>
            {recentChips.map((project) => (
              <button
                key={project.path}
                type="button"
                title={tildify(project.path, home)}
                disabled={busyPath !== null}
                onClick={() => void add(project.path)}
                className="inline-flex h-7 max-w-44 shrink-0 items-center gap-1.5 rounded-lg border border-border px-2.5 text-sm text-foreground transition-colors duration-[var(--motion-duration-control)] hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
              >
                <History
                  className="size-4 shrink-0 text-muted-foreground"
                  aria-hidden
                />
                <span className="truncate">
                  {project.name || basename(project.path)}
                </span>
              </button>
            ))}
          </div>
        ) : null}

        {/* Where the listing is: crumbs that jump, and what it holds. */}
        <div className="flex min-h-9 shrink-0 items-center gap-2 px-5 pt-4 pb-1.5">
          {pathMode ? (
            <nav
              aria-label="Folder"
              className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
            >
              <ol className="flex items-center gap-0.5">
                {crumbs.map((crumb, i) => {
                  const last = i === crumbs.length - 1
                  return (
                    <li key={crumb.path} className="flex items-center gap-0.5">
                      {i > 0 ? (
                        <ChevronRight
                          className="size-4 shrink-0 text-muted-foreground/50"
                          aria-hidden
                        />
                      ) : null}
                      <button
                        type="button"
                        onClick={() => go(crumb.path)}
                        aria-current={last ? 'location' : undefined}
                        className={cn(
                          'inline-flex h-6 items-center rounded-md px-1.5 text-sm transition-colors duration-[var(--motion-duration-control)] hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                          last
                            ? 'font-medium text-foreground'
                            : 'text-muted-foreground hover:text-foreground',
                        )}
                      >
                        {crumb.label}
                      </button>
                    </li>
                  )
                })}
              </ol>
            </nav>
          ) : (
            <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
              {segment ? `Folders named “${segment}”` : 'Search'}
            </span>
          )}
          <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
            {pathMode && folderListing
              ? countLabel(folderListing.folders)
              : !pathMode && segment
                ? `${rows.length} found`
                : null}
          </span>
          {pathMode && dir ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="-mr-2 h-7 shrink-0 gap-1.5 px-2 text-muted-foreground hover:text-foreground"
              disabled={busyPath !== null || creating}
              onClick={() => startNewFolder()}
              title={`New folder in ${tildify(dir, home)}`}
            >
              <FolderPlus className="size-4" aria-hidden />
              New folder
              {pointerFine ? (
                <KeyCombo
                  binding={NEW_FOLDER_BINDING}
                  className="gap-0.5"
                  capClassName="h-5 min-w-5 rounded-[5px] border border-border bg-transparent px-1 font-inter text-[11px] font-medium text-muted-foreground shadow-none"
                />
              ) : null}
            </Button>
          ) : null}
        </div>

        <div
          ref={listRef}
          id="add-project-list"
          role="listbox"
          aria-label="Folders"
          className="mx-3 mb-3 min-h-0 flex-1 overflow-y-auto overscroll-contain rounded-xl border border-border bg-background/40 p-1"
        >
          {newFolder !== null && dir ? (
            <form
              className="flex h-9 items-center gap-2.5 rounded-lg bg-muted px-2.5"
              onSubmit={(event) => {
                event.preventDefault()
                void createFolder(newFolder)
              }}
            >
              {creating ? (
                <Loader2
                  className="size-4 shrink-0 animate-spin text-muted-foreground"
                  aria-hidden
                />
              ) : (
                <FolderPlus
                  className="size-4 shrink-0 text-muted-foreground"
                  aria-hidden
                />
              )}
              <input
                autoFocus
                value={newFolder}
                onChange={(event) => setNewFolder(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') {
                    event.preventDefault()
                    event.stopPropagation()
                    setNewFolder(null)
                    if (pointerFine) inputRef.current?.focus()
                  }
                }}
                onBlur={() => {
                  if (!creating && !newFolder.trim()) setNewFolder(null)
                }}
                disabled={creating}
                placeholder="Folder name"
                aria-label={`New folder in ${tildify(dir, home)}`}
                spellCheck={false}
                autoCapitalize="off"
                autoComplete="off"
                className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
              />
              <span className="shrink-0 text-xs text-muted-foreground">
                {creating ? 'Creating…' : '↵ create · esc cancel'}
              </span>
            </form>
          ) : null}
          {loading ? (
            <div
              className="space-y-1 p-1"
              role="status"
              aria-label="Reading folder"
            >
              {[0, 1, 2, 3].map((i) => (
                <div
                  key={i}
                  className="h-9 animate-pulse rounded-lg bg-muted/60"
                  style={{ opacity: 1 - i * 0.2 }}
                />
              ))}
            </div>
          ) : note ? (
            <div className="flex h-full min-h-32 items-center justify-center px-6 text-center text-sm text-muted-foreground">
              <span>{note}</span>
            </div>
          ) : (
            sections.map((section) => (
              // biome-ignore lint/a11y/useSemanticElements: a listbox groups its options with role=group; a fieldset is a form control group
              <div
                key={section.id}
                role="group"
                aria-label={section.label ?? undefined}
                className="pb-1 last:pb-0"
              >
                {section.label ? (
                  <div className="px-2.5 pt-2 pb-1 text-xs font-medium text-muted-foreground">
                    {section.label}
                  </div>
                ) : null}
                {section.rows.map((row) => {
                  rowNumber += 1
                  const i = rowNumber
                  return (
                    <FolderRow
                      key={`${section.id}:${row.path}`}
                      id={`add-project-row-${i}`}
                      row={row}
                      home={home}
                      active={i === activeIndex}
                      busy={busyPath === row.path}
                      disabled={busyPath !== null}
                      onHover={() => setActive(i)}
                      onOpen={() => go(row.path)}
                      onAdd={() => void add(row.path)}
                    />
                  )
                })}
              </div>
            ))
          )}
        </div>

        {/* The decision: exactly which folder, then Cancel / Add. */}
        <div className="flex shrink-0 items-center gap-3 border-t border-border bg-muted/40 px-5 py-3">
          <span className="flex min-w-0 flex-1 items-center gap-2 text-sm text-muted-foreground">
            {target ? (
              <>
                {targetIsProject ? (
                  <FolderGit2 className="size-4 shrink-0" aria-hidden />
                ) : (
                  <Folder className="size-4 shrink-0" aria-hidden />
                )}
                <span className="truncate font-mono text-[13px] text-foreground">
                  {tildify(target, home)}
                </span>
              </>
            ) : (
              <span className="truncate">Pick a folder to add</span>
            )}
          </span>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="pointer-coarse:hidden"
            disabled={busyPath !== null}
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={!target || busyPath !== null}
            onClick={() => void add(target)}
            className="gap-2 pr-1.5"
          >
            {busyPath ? 'Adding…' : 'Add project'}
            {pointerFine ? (
              <KeyCombo
                binding="Mod+Enter"
                className="gap-0.5"
                capClassName="h-5 min-w-5 rounded-[5px] bg-primary-foreground/15 px-1 font-inter text-[11px] font-medium text-primary-foreground shadow-none"
              />
            ) : null}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function FolderRow({
  id,
  row,
  home,
  active,
  busy,
  disabled,
  onHover,
  onOpen,
  onAdd,
}: {
  id: string
  row: Row
  home: string | null
  active: boolean
  busy: boolean
  disabled: boolean
  onHover: () => void
  onOpen: () => void
  onAdd: () => void
}) {
  const Icon = busy ? Loader2 : row.git ? FolderGit2 : Folder
  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: the path field owns the keys (aria-activedescendant); the row only takes the pointer
    <div
      id={id}
      role="option"
      tabIndex={-1}
      aria-selected={active}
      data-active={active}
      title={row.path}
      onPointerMove={onHover}
      onClick={onOpen}
      className={cn(
        'group/row flex min-h-12 w-full cursor-pointer scroll-my-1 items-center gap-3 rounded-lg pr-1.5 pl-2.5 transition-colors duration-[var(--motion-duration-control)] pointer-fine:min-h-9',
        active && 'bg-muted',
      )}
    >
      <Icon
        className={cn(
          'size-4 shrink-0',
          busy && 'animate-spin',
          row.git && !busy ? 'text-foreground' : 'text-muted-foreground',
        )}
        aria-hidden
      />
      <span className="flex min-w-0 flex-1 items-baseline gap-2">
        <span className="max-w-[65%] shrink-0 truncate text-sm text-foreground">
          <Highlighted text={row.name} ranges={row.ranges} />
        </span>
        {row.showWhere ? (
          <span className="min-w-0 truncate text-sm text-muted-foreground">
            {tildify(parentOf(row.path), home)}
          </span>
        ) : null}
      </span>
      {/* Add shows on the highlighted row; a click elsewhere on the row
          opens the folder instead. */}
      <Button
        type="button"
        variant="secondary"
        size="xs"
        tabIndex={-1}
        disabled={disabled}
        onClick={(event) => {
          event.stopPropagation()
          onAdd()
        }}
        className={cn(
          'shrink-0 transition-opacity duration-[var(--motion-duration-control)]',
          active ? 'opacity-100' : 'opacity-0 pointer-coarse:opacity-100',
        )}
      >
        Add
      </Button>
      <ChevronRight
        className="size-4 shrink-0 text-muted-foreground/60"
        aria-hidden
      />
    </div>
  )
}

function Highlighted({
  text,
  ranges,
}: {
  text: string
  ranges?: Array<[number, number]>
}) {
  if (!ranges?.length) return <>{text}</>
  const parts: ReactNode[] = []
  let at = 0
  for (const [start, end] of ranges) {
    if (start > at) parts.push(text.slice(at, start))
    parts.push(
      <mark
        key={start}
        className="bg-transparent font-semibold text-foreground"
      >
        {text.slice(start, end)}
      </mark>,
    )
    at = end
  }
  if (at < text.length) parts.push(text.slice(at))
  return <>{parts}</>
}

function Mono({ children }: { children: ReactNode }) {
  return <span className="font-mono text-foreground">{children}</span>
}

/** `~ › conductor › repos` as jump targets, home collapsed to `~`. */
function crumbsOf(
  dir: string,
  home: string | null,
): Array<{ path: string; label: string }> {
  const out: Array<{ path: string; label: string }> = []
  let base = '/'
  let rest = dir
  if (home && (dir === home || dir.startsWith(`${home}/`))) {
    out.push({ path: home, label: '~' })
    base = home
    rest = dir.slice(home.length)
  } else {
    out.push({ path: '/', label: '/' })
  }
  for (const part of rest.split('/').filter(Boolean)) {
    base = base === '/' ? `/${part}` : `${base}/${part}`
    out.push({ path: base, label: part })
  }
  return out
}

function countLabel(folders: IndexedFolder[]): string {
  const visible = folders.filter((f) => !f.name.startsWith('.'))
  const repos = visible.filter((f) => f.git).length
  const n = visible.length
  const base = `${n} folder${n === 1 ? '' : 's'}`
  return repos ? `${base} · ${repos} repo${repos === 1 ? '' : 's'}` : base
}
