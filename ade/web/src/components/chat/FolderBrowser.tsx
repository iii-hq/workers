import {
  AlertCircle,
  ArrowLeft,
  Check,
  ChevronUp,
  CornerDownLeft,
  Folder,
  FolderOpen,
  Search,
} from 'lucide-react'
import { type KeyboardEvent, useEffect, useRef, useState } from 'react'
import { Breadcrumb } from '@/components/ui/Breadcrumb'
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

export function FolderBrowser({
  wide,
  keyboard,
  busy,
  error,
  onUse,
  onBack,
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

  const rowClass = wide
    ? 'flex min-h-8 w-full min-w-0 items-center gap-2 rounded-md px-2 py-1 text-left font-sans text-[12px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus'
    : 'flex min-h-14 w-full min-w-0 items-center gap-3 rounded-md px-3 py-2.5 text-left font-sans text-base focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus'
  const noteClass = cn(
    'rounded-md px-3 py-4 font-sans text-ink-faint',
    wide ? 'text-[11px]' : 'text-base',
  )
  const iconButton = cn(
    'flex shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
    wide ? 'size-7' : 'size-12',
  )
  const banner = error ?? rootsError

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: key routing for the search box and Cmd/Ctrl+Enter anywhere inside
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" onKeyDown={onKeyDown}>
      <div className="mx-4 mb-2 flex shrink-0 items-center gap-1">
        {onBack ? (
          <button
            type="button"
            aria-label="back to projects"
            onClick={onBack}
            className={iconButton}
          >
            <ArrowLeft className="size-4" aria-hidden />
          </button>
        ) : null}
        {!wide && path !== null ? (
          <button
            type="button"
            aria-label="up one level"
            onClick={goUp}
            className={iconButton}
          >
            <ChevronUp className="size-4" aria-hidden />
          </button>
        ) : null}
        <div
          className={cn(
            'flex min-w-0 flex-1 items-center gap-2 rounded-md bg-surface px-3 focus-within:ring-2 focus-within:ring-rule-focus',
            wide ? 'min-h-9' : 'min-h-12',
          )}
        >
          <Search className="size-4 shrink-0 text-ink-ghost" aria-hidden />
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
              'min-w-0 flex-1 bg-transparent text-ink placeholder:text-ink-ghost focus:outline-none',
              wide ? 'text-[12px]' : 'text-base',
            )}
          />
        </div>
      </div>

      <div className="mx-4 mb-2 flex min-w-0 shrink-0">
        {path && root ? (
          <Breadcrumb
            // a `/` separator after a `/` root reads as `/ / Users`
            separator="›"
            // the phone's crumbs are its parent columns: two levels up
            items={trailOf(path, root, wide ? Number.POSITIVE_INFINITY : 3).map(
              (p, i, all) => ({
                key: p,
                label: p === root ? p : basename(p),
                onClick:
                  i < all.length - 1
                    ? () => {
                        enter(p)
                        focusSearch()
                      }
                    : undefined,
              }),
            )}
          />
        ) : (
          <span className="font-sans text-[11px] text-ink-ghost">
            Workspace roots
          </span>
        )}
      </div>

      {banner ? (
        <div
          className={cn(
            'mx-4 mb-2 flex shrink-0 items-start gap-2 rounded-md bg-warn-muted px-3 py-2 font-sans text-warn',
            wide ? 'text-[11px]' : 'text-base',
          )}
        >
          <AlertCircle className="size-4 shrink-0" aria-hidden />
          <span className="min-w-0 [overflow-wrap:anywhere]">{banner}</span>
        </div>
      ) : null}

      <div
        ref={bodyRef}
        className={cn(
          'flex min-h-0 min-w-0 flex-1',
          wide && 'border-y border-rule-2',
        )}
      >
        {trail.slice(0, -1).map((dir, index) => {
          const open = trail[index + 1]
          return (
            <div
              key={dir}
              className="w-52 shrink-0 space-y-px overflow-y-auto overscroll-contain border-r border-rule-2 p-1"
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
                      ? 'bg-surface-selected text-ink'
                      : 'text-ink-faint hover:bg-surface-hover hover:text-ink',
                  )}
                >
                  {e.path === open ? (
                    <FolderOpen className="size-4 shrink-0" aria-hidden />
                  ) : (
                    <Folder className="size-4 shrink-0" aria-hidden />
                  )}
                  <span className="truncate">{e.name}</span>
                </button>
              ))}
            </div>
          )
        })}

        <div
          className={cn(
            'min-w-0 flex-1 space-y-px overflow-y-auto overscroll-contain',
            wide ? 'p-1' : 'px-4 pb-2',
          )}
        >
          {isAbsPath(query) ? (
            <button
              type="button"
              onClick={() => {
                jumpTo(query)
                focusSearch()
              }}
              className={cn(rowClass, 'bg-surface-selected text-ink')}
            >
              <CornerDownLeft className="size-4 shrink-0" aria-hidden />
              <span className="truncate font-mono">Go to {query.trim()}</span>
            </button>
          ) : null}

          {roots === null || (path !== null && !current) ? (
            <div className={noteClass}>Loading folders…</div>
          ) : current?.error ? (
            <div
              className={cn(
                noteClass,
                'flex items-start gap-2 bg-warn-muted text-warn',
              )}
            >
              <AlertCircle className="size-4 shrink-0" aria-hidden />
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
                  'text-ink',
                  index === activeIndex
                    ? 'bg-surface-selected'
                    : 'hover:bg-surface-hover',
                )}
              >
                {path === null ? (
                  <FolderOpen
                    className="size-4 shrink-0 text-ink-faint"
                    aria-hidden
                  />
                ) : (
                  <Folder
                    className="size-4 shrink-0 text-ink-faint"
                    aria-hidden
                  />
                )}
                <span className={cn('truncate', path === null && 'font-mono')}>
                  {e.name}
                </span>
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

      <div
        className={cn(
          'flex shrink-0 items-center gap-3 font-sans text-[11px] text-ink-ghost',
          wide
            ? 'px-4 py-2.5'
            : 'px-4 pt-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]',
        )}
      >
        {keyboard ? (
          <span className="flex min-w-0 items-center gap-3">
            <span className="flex items-center gap-1.5">
              <KeyCombo binding="ArrowUp" />
              <KeyCombo binding="ArrowDown" />
              move
            </span>
            <span className="flex items-center gap-1.5">
              <KeyCombo binding="Enter" />
              open
            </span>
            <span className="flex items-center gap-1.5">
              <KeyCombo binding="ArrowLeft" />
              up
            </span>
          </span>
        ) : null}
        <span
          className={cn(
            'ml-auto flex min-w-0 items-center gap-2',
            !wide && 'flex-1',
          )}
        >
          {keyboard && target ? <KeyCombo binding="Mod+Enter" /> : null}
          <button
            type="button"
            disabled={busy || !target}
            onClick={() => use(target)}
            title={target ?? undefined}
            className={cn(
              'inline-flex min-w-0 items-center justify-center gap-1.5 rounded-sm bg-accent pr-3 pl-2 font-sans font-medium whitespace-nowrap text-accent-fg hover:bg-accent-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-rule-focus disabled:opacity-50',
              wide
                ? 'min-h-7 py-1 text-[11px]'
                : 'min-h-12 flex-1 py-2 text-base',
            )}
          >
            <Check className="size-4 shrink-0" aria-hidden />
            <span className="truncate">
              {target ? `Use ${basename(target)}` : 'Use folder'}
            </span>
          </button>
        </span>
      </div>
    </div>
  )
}
