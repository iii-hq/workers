import {
  AlertCircle,
  ArrowLeft,
  ChevronRight,
  ChevronUp,
  CornerDownLeft,
  Folder,
  FolderOpen,
  HardDrive,
  Search,
} from 'lucide-react'
import { type KeyboardEvent, useEffect, useRef, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import { Hint } from '@/components/ui/Hint'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { getIiiClient } from '@/lib/iii-client'
import { cn } from '@/lib/utils'
import {
  errMsg,
  WORKSPACE_LIST_FUNCTION_ID,
  WORKSPACE_ROOTS_FUNCTION_ID,
} from '@/lib/working-dir'

/**
 * Folder browsing for "Add project", one shell workspace listing per level.
 *
 * Wide (the desktop dialog) shows the current folder next to its two parents,
 * Finder column style; narrow (the phone sheet) shows the current folder only.
 * The search box drives everything from the keyboard: ↑/↓ move, ↵ or → open,
 * ← goes up, Cmd/Ctrl+↵ uses the highlighted folder (or the current one when
 * nothing is highlighted). Typing filters the current folder and highlights
 * the first match; an absolute path jumps there on ↵ or is used on Cmd/Ctrl+↵.
 *
 * Styled on the shadcn tokens (`muted`, `border`, `foreground`), so it reads
 * the same inside the neutral dialog and the picker's own sheet.
 */

interface DirEntry {
  name: string
  path: string
}

interface Listing {
  dirs: DirEntry[]
  error?: string
}

export function basename(p: string): string {
  const parts = p.split('/').filter(Boolean)
  return parts.length ? parts[parts.length - 1] : p
}

export function parentOf(p: string): string {
  const trimmed = p.replace(/\/+$/, '')
  const idx = trimmed.lastIndexOf('/')
  return idx <= 0 ? '/' : trimmed.slice(0, idx)
}

export const isAbsPath = (s: string) => s.trim().startsWith('/')

function within(p: string, root: string): boolean {
  return p === root || p.startsWith(root === '/' ? '/' : `${root}/`)
}

/** `path` and up to `depth - 1` of its ancestors, never above `root`. */
export function trailOf(path: string, root: string, depth: number): string[] {
  const trail = [path]
  while (trail.length < depth && trail[0] !== root) {
    const up = parentOf(trail[0])
    if (up === trail[0] || !within(up, root)) break
    trail.unshift(up)
  }
  return trail
}

async function listDirs(dir: string): Promise<DirEntry[]> {
  const client = await getIiiClient()
  const res = await client.trigger<{
    entries?: Array<DirEntry & { kind: string }>
  }>(WORKSPACE_LIST_FUNCTION_ID, { path: dir, page_size: 200 })
  return (res?.entries ?? [])
    .filter((e) => e.kind === 'dir')
    .map(({ name, path }) => ({ name, path }))
    .sort((a, b) => a.name.localeCompare(b.name))
}

/** The footer's key caps: hairline boxes, no fill, so they read as labels. */
const CAP =
  'h-5 min-w-5 rounded-[5px] border border-border bg-transparent px-1 font-inter text-[11px] font-medium text-muted-foreground shadow-none'

export function FolderBrowser({
  wide,
  keyboard,
  busy,
  error,
  onUse,
  onBack,
  onCancel,
}: {
  /** Desktop dialog: parent columns and compact rows. */
  wide: boolean
  /** A physical keyboard is there: key hints, and focus returns to search. */
  keyboard: boolean
  /** The picker is validating a folder. */
  busy: boolean
  /** Why the picker refused the last folder. */
  error: string | null
  onUse: (dir: string) => void
  /** Back to the project list (the sheet has no dialog close). */
  onBack?: () => void
  /** Dismiss without choosing; shown as a Cancel button beside Use. */
  onCancel?: () => void
}) {
  const [roots, setRoots] = useState<string[] | null>(null)
  const [rootsError, setRootsError] = useState<string | null>(null)
  const [root, setRoot] = useState<string | null>(null)
  // null = the roots list (only when there are several roots)
  const [path, setPath] = useState<string | null>(null)
  // ponytail: listings live as long as the browser does (one dialog open);
  // reopen it to see folders created meanwhile.
  const [listings, setListings] = useState<Record<string, Listing>>({})
  const requested = useRef(new Set<string>())
  const [query, setQuery] = useState('')
  const [activePath, setActivePath] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const bodyRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let live = true
    void (async () => {
      try {
        const client = await getIiiClient()
        const info = await client.trigger<{ roots?: string[] }>(
          WORKSPACE_ROOTS_FUNCTION_ID,
          {},
        )
        if (!live) return
        const found = info?.roots ?? []
        setRoots(found)
        if (found.length === 1) {
          setRoot(found[0])
          setPath(found[0])
        }
      } catch (err) {
        if (!live) return
        setRootsError(errMsg(err))
        setRoots([])
      }
    })()
    return () => {
      live = false
    }
  }, [])

  const trail = path && root ? trailOf(path, root, wide ? 3 : 1) : []
  const trailKey = trail.join('\n')

  useEffect(() => {
    for (const dir of trailKey ? trailKey.split('\n') : []) {
      if (requested.current.has(dir)) continue
      requested.current.add(dir)
      listDirs(dir).then(
        (dirs) => setListings((current) => ({ ...current, [dir]: { dirs } })),
        (err) =>
          setListings((current) => ({
            ...current,
            [dir]: { dirs: [], error: errMsg(err) },
          })),
      )
    }
  }, [trailKey])

  const q = query.trim().toLowerCase()
  const filtering = q !== '' && !isAbsPath(query)
  const current = path ? listings[path] : undefined
  const rows: DirEntry[] = (
    path === null
      ? (roots ?? []).map((r) => ({ name: r, path: r }))
      : (current?.dirs ?? [])
  ).filter((e) => !filtering || e.name.toLowerCase().includes(q))
  const found = rows.findIndex((e) => e.path === activePath)
  // Typing highlights the first match until an arrow key picks another.
  const activeIndex = found === -1 && filtering && rows.length ? 0 : found
  const target = rows[activeIndex]?.path ?? path

  // Keep the highlighted row and each parent column's open folder in view.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-run when rows move under the anchors
  useEffect(() => {
    for (const el of bodyRef.current?.querySelectorAll('[data-anchor]') ?? []) {
      el.scrollIntoView({ block: 'nearest' })
    }
  }, [activePath, trailKey, listings])

  const focusSearch = () => {
    if (keyboard) inputRef.current?.focus()
  }

  const enter = (dir: string, highlight: string | null = null) => {
    if (path === null) setRoot(dir)
    setPath(dir)
    setQuery('')
    setActivePath(highlight)
  }

  const goUp = () => {
    if (!path || !root) return
    if (path !== root) enter(parentOf(path), path)
    else if ((roots?.length ?? 0) > 1) {
      setPath(null)
      setRoot(null)
      setQuery('')
      setActivePath(root)
    }
  }

  const jumpTo = (raw: string) => {
    const p = raw.trim().replace(/\/+$/, '') || '/'
    const matched = [...(roots ?? [])]
      .sort((a, b) => b.length - a.length)
      .find((r) => within(p, r))
    setRoot(matched ?? p)
    setPath(p)
    setQuery('')
    setActivePath(null)
  }

  const use = (dir: string | null) => {
    if (dir && !busy) onUse(dir.trim())
  }

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.nativeEvent.isComposing) return
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault()
      use(isAbsPath(query) ? query : target)
      return
    }
    // Everything else belongs to the search box; a focused button keeps its own keys.
    if (e.target !== inputRef.current) return
    const hot = rows[activeIndex]
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      if (!rows.length) return
      const step = e.key === 'ArrowDown' ? 1 : -1
      const next =
        activeIndex === -1
          ? step > 0
            ? 0
            : rows.length - 1
          : (activeIndex + step + rows.length) % rows.length
      setActivePath(rows[next].path)
    } else if (e.key === 'Enter') {
      e.preventDefault()
      if (isAbsPath(query)) jumpTo(query)
      else if (hot) enter(hot.path)
    } else if (e.key === 'ArrowRight' && query === '' && hot) {
      e.preventDefault()
      enter(hot.path)
    } else if (e.key === 'ArrowLeft' && query === '') {
      e.preventDefault()
      goUp()
    }
  }

  /* One row recipe for every column: a rounded wash on hover, a filled
     wash when open or highlighted, the glyph in the muted ink. */
  const rowClass = cn(
    'group/row flex w-full min-w-0 items-center gap-2 rounded-md text-left font-inter transition-colors duration-[var(--motion-duration-control)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
    wide ? 'min-h-8 px-2 py-1 text-[13px]' : 'min-h-12 px-3 py-2 text-base',
  )
  const noteClass = cn(
    'px-3 py-8 text-center font-inter text-muted-foreground',
    wide ? 'text-[13px]' : 'text-base',
  )
  const iconButton = cn(
    'flex shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors duration-[var(--motion-duration-control)] hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
    wide ? 'size-9' : 'size-12',
  )
  const banner = error ?? rootsError
  const crumbs =
    path && root ? trailOf(path, root, wide ? Number.POSITIVE_INFINITY : 3) : []

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: key routing for the search box and Cmd/Ctrl+Enter anywhere inside
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col font-inter"
      onKeyDown={onKeyDown}
    >
      {/* The search box: a bordered field, the only input on the surface. */}
      <div
        className={cn(
          'flex shrink-0 items-center gap-1.5 px-4',
          wide ? 'pt-4 pb-3' : 'pt-3 pb-2',
        )}
      >
        {onBack ? (
          <Hint label="Back to projects" disabled={!keyboard}>
            <button
              type="button"
              aria-label="back to projects"
              onClick={onBack}
              className={iconButton}
            >
              <ArrowLeft className="size-4" aria-hidden />
            </button>
          </Hint>
        ) : null}
        {!wide && path !== null ? (
          <Hint label="Up one level" disabled={!keyboard}>
            <button
              type="button"
              aria-label="up one level"
              onClick={goUp}
              className={iconButton}
            >
              <ChevronUp className="size-4" aria-hidden />
            </button>
          </Hint>
        ) : null}
        <div
          className={cn(
            'flex min-w-0 flex-1 items-center gap-2 rounded-lg border border-border bg-background px-3 transition-[border-color,box-shadow] duration-[var(--motion-duration-control)] focus-within:border-foreground/40 focus-within:ring-3 focus-within:ring-foreground/10',
            wide ? 'h-9' : 'min-h-12',
          )}
        >
          <Search
            className="size-4 shrink-0 text-muted-foreground"
            aria-hidden
          />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              setActivePath(null)
            }}
            placeholder={
              path
                ? `Filter ${basename(path)} or paste a path…`
                : 'Filter roots or paste a path…'
            }
            aria-label="filter folders or paste a path"
            name="folder-search"
            autoCapitalize="none"
            autoCorrect="off"
            autoComplete="off"
            spellCheck={false}
            className={cn(
              'min-w-0 flex-1 bg-transparent text-foreground placeholder:text-muted-foreground focus:outline-none',
              wide ? 'text-[13px]' : 'text-base',
            )}
          />
        </div>
      </div>

      {/* Where you are: the path as crumbs, each ancestor a button. */}
      <div className="flex min-w-0 shrink-0 items-center px-4 pb-3">
        {crumbs.length > 0 ? (
          <nav
            aria-label="Breadcrumb"
            className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
          >
            <ol className="flex items-center gap-0.5">
              {crumbs.map((p, index, all) => {
                const last = index === all.length - 1
                const label = p === root ? p : basename(p)
                return (
                  <li key={p} className="flex min-w-0 items-center gap-0.5">
                    {index > 0 ? (
                      <ChevronRight
                        aria-hidden
                        className="size-4 shrink-0 text-muted-foreground/60"
                      />
                    ) : null}
                    {last ? (
                      <span
                        aria-current="page"
                        className={cn(
                          'inline-flex h-6 max-w-[20rem] items-center truncate rounded-md px-1.5 font-medium text-foreground',
                          wide ? 'text-xs' : 'text-sm',
                          p === root && 'font-mono',
                        )}
                      >
                        {label}
                      </span>
                    ) : (
                      <button
                        type="button"
                        onClick={() => {
                          enter(p)
                          focusSearch()
                        }}
                        className={cn(
                          'inline-flex h-6 max-w-[14rem] items-center truncate rounded-md px-1.5 text-muted-foreground transition-colors duration-[var(--motion-duration-control)] hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                          wide ? 'text-xs' : 'text-sm',
                          p === root && 'font-mono',
                        )}
                      >
                        {label}
                      </button>
                    )}
                  </li>
                )
              })}
            </ol>
          </nav>
        ) : (
          <span className="inline-flex h-6 items-center px-1.5 text-xs font-medium text-muted-foreground">
            Workspace roots
          </span>
        )}
      </div>

      {banner ? (
        <div
          role="alert"
          className={cn(
            'mx-4 mb-3 flex shrink-0 items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-destructive',
            wide ? 'text-xs' : 'text-sm',
          )}
        >
          <AlertCircle className="mt-px size-4 shrink-0" aria-hidden />
          <span className="min-w-0 [overflow-wrap:anywhere]">{banner}</span>
        </div>
      ) : null}

      <div
        ref={bodyRef}
        className={cn(
          'flex min-h-0 min-w-0 flex-1',
          wide ? 'border-t border-border' : 'border-t border-border',
        )}
      >
        {trail.slice(0, -1).map((dir, index) => {
          const open = trail[index + 1]
          return (
            <div
              key={dir}
              className="w-56 shrink-0 space-y-px overflow-y-auto overscroll-contain border-r border-border p-1.5"
            >
              {(listings[dir]?.dirs ?? []).map((e) => (
                <button
                  key={e.path}
                  type="button"
                  data-anchor={e.path === open ? '' : undefined}
                  aria-current={e.path === open ? 'true' : undefined}
                  onClick={() => {
                    enter(e.path)
                    focusSearch()
                  }}
                  title={e.path}
                  className={cn(
                    rowClass,
                    e.path === open
                      ? 'bg-muted text-foreground'
                      : 'text-muted-foreground hover:bg-muted/60 hover:text-foreground',
                  )}
                >
                  {e.path === open ? (
                    <FolderOpen className="size-4 shrink-0" aria-hidden />
                  ) : (
                    <Folder className="size-4 shrink-0" aria-hidden />
                  )}
                  <span className="min-w-0 flex-1 truncate">{e.name}</span>
                  {e.path === open ? (
                    <ChevronRight
                      className="size-4 shrink-0 text-muted-foreground"
                      aria-hidden
                    />
                  ) : null}
                </button>
              ))}
            </div>
          )
        })}

        <div
          className={cn(
            'min-w-0 flex-1 space-y-px overflow-y-auto overscroll-contain',
            wide ? 'p-1.5' : 'px-3 pb-2',
          )}
        >
          {isAbsPath(query) ? (
            <button
              type="button"
              onClick={() => {
                jumpTo(query)
                focusSearch()
              }}
              className={cn(rowClass, 'bg-muted text-foreground')}
            >
              <CornerDownLeft
                className="size-4 shrink-0 text-muted-foreground"
                aria-hidden
              />
              <span className="truncate">
                Go to{' '}
                <span className="font-mono text-[0.95em]">{query.trim()}</span>
              </span>
            </button>
          ) : null}

          {roots === null || (path !== null && !current) ? (
            <div className={noteClass}>Loading folders…</div>
          ) : current?.error ? (
            <div
              className={cn(
                noteClass,
                'flex items-start gap-2 text-left text-destructive',
              )}
            >
              <AlertCircle className="mt-px size-4 shrink-0" aria-hidden />
              <span className="min-w-0 [overflow-wrap:anywhere]">
                {current.error}
              </span>
            </div>
          ) : rows.length > 0 ? (
            rows.map((e, index) => (
              <button
                key={e.path}
                type="button"
                data-anchor={index === activeIndex ? '' : undefined}
                onClick={() => {
                  enter(e.path)
                  focusSearch()
                }}
                title={e.path}
                className={cn(
                  rowClass,
                  'text-foreground',
                  index === activeIndex ? 'bg-muted' : 'hover:bg-muted/60',
                )}
              >
                {path === null ? (
                  <HardDrive
                    className="size-4 shrink-0 text-muted-foreground"
                    aria-hidden
                  />
                ) : (
                  <Folder
                    className="size-4 shrink-0 text-muted-foreground"
                    aria-hidden
                  />
                )}
                <span
                  className={cn(
                    'min-w-0 flex-1 truncate',
                    path === null && 'font-mono text-[0.95em]',
                  )}
                >
                  {e.name}
                </span>
                <ChevronRight
                  className="size-4 shrink-0 text-muted-foreground opacity-0 transition-opacity duration-[var(--motion-duration-control)] group-hover/row:opacity-100"
                  aria-hidden
                />
              </button>
            ))
          ) : isAbsPath(query) ? null : (
            <div className={noteClass}>
              {filtering
                ? 'No matching folders.'
                : 'No subfolders. You can use this folder.'}
            </div>
          )}
        </div>
      </div>

      {/* The footer: how the keys work on the left, the decision on the
          right. The same bar the shadcn dialog footer draws. */}
      <div
        className={cn(
          'flex shrink-0 items-center gap-3 border-t border-border bg-muted/50',
          wide
            ? 'px-4 py-3'
            : 'px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]',
        )}
      >
        {keyboard ? (
          <span className="flex min-w-0 items-center gap-3 text-xs text-muted-foreground">
            <span className="flex items-center gap-1.5">
              <span className="flex items-center gap-0.5">
                <KeyCombo binding="ArrowUp" capClassName={CAP} />
                <KeyCombo binding="ArrowDown" capClassName={CAP} />
              </span>
              Move
            </span>
            <span className="flex items-center gap-1.5">
              <KeyCombo binding="Enter" capClassName={CAP} />
              Open
            </span>
            <span className="flex items-center gap-1.5">
              <KeyCombo binding="ArrowLeft" capClassName={CAP} />
              Up
            </span>
          </span>
        ) : null}
        <span
          className={cn(
            'ml-auto flex min-w-0 items-center gap-2',
            !wide && 'flex-1',
          )}
        >
          {onCancel && wide ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={onCancel}
            >
              Cancel
            </Button>
          ) : null}
          <button
            type="button"
            disabled={busy || !target}
            onClick={() => use(target)}
            title={target ?? undefined}
            className={cn(
              'inline-flex min-w-0 items-center justify-center gap-2 rounded-lg bg-primary font-inter font-medium whitespace-nowrap text-primary-foreground transition-colors duration-[var(--motion-duration-control)] hover:bg-primary/85 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50',
              wide
                ? 'h-8 pr-1.5 pl-3 text-[13px]'
                : 'min-h-12 flex-1 px-4 text-base',
            )}
          >
            <span className="truncate">
              {target ? `Use ${basename(target)}` : 'Use folder'}
            </span>
            {keyboard && target ? (
              <KeyCombo
                binding="Mod+Enter"
                className="gap-0.5"
                capClassName="h-5 min-w-5 rounded-[5px] bg-primary-foreground/15 px-1 font-inter text-[11px] font-medium text-primary-foreground shadow-none"
              />
            ) : null}
          </button>
        </span>
      </div>
    </div>
  )
}
