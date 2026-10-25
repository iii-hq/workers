import { ArrowLeft, type LucideIcon, Search, Settings2, X } from 'lucide-react'
import { type ReactNode, useMemo, useRef, useState } from 'react'
import { Input } from '@/components/shadcn/input'
import { Skeleton } from '@/components/ui/Skeleton'
import { cn } from '@/lib/utils'

export interface SettingsNavEntry {
  id: string
  label: string
  description: string
  icon: LucideIcon
}

interface SettingsNavigationProps {
  entries: SettingsNavEntry[]
  generalSelected: boolean
  selectedId: string | null
  onSelectGeneral: () => void
  onSelectWorker: (configurationId: string) => void
  isLoading: boolean
  isError: boolean
  errorMessage?: string
  /** The sidebar is the whole page (narrow flow). */
  narrow: boolean
  /** Leave settings and return to the workspace. */
  onClose?: () => void
}

/**
 * The settings sidebar, after Linear: "Back to app" at the top, a compact
 * search, then grouped single-line links with a small group label. Nothing
 * here animates; selection and hover are instant colour changes.
 */
export function SettingsNavigation({
  entries,
  generalSelected,
  selectedId,
  onSelectGeneral,
  onSelectWorker,
  isLoading,
  isError,
  errorMessage,
  narrow,
  onClose,
}: SettingsNavigationProps) {
  const [query, setQuery] = useState('')
  const searchRef = useRef<HTMLInputElement | null>(null)
  const normalizedQuery = query.trim().toLowerCase()
  const filteredEntries = useMemo(() => {
    if (!normalizedQuery) return entries
    return entries.filter((entry) =>
      `${entry.label} ${entry.id} ${entry.description}`
        .toLowerCase()
        .includes(normalizedQuery),
    )
  }, [entries, normalizedQuery])

  return (
    <aside
      className={cn(
        'flex min-h-0 flex-col bg-neutral-50 dark:bg-black',
        narrow
          ? 'min-w-0 flex-1'
          : 'w-[250px] shrink-0 border-r border-neutral-200 dark:border-neutral-800',
      )}
      aria-label="Settings navigation"
    >
      <div className="shrink-0 px-3 pt-3">
        {onClose ? (
          <div className="flex justify-end">
            <button
              type="button"
              onClick={onClose}
              className="inline-flex h-8 max-w-full items-center gap-1.5 rounded-md px-2 text-[15px] text-muted-foreground transition-colors duration-150 ease-out outline-none hover:bg-neutral-200/60 hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 pointer-coarse:h-11 dark:hover:bg-neutral-900"
            >
              <ArrowLeft className="size-4 shrink-0" aria-hidden />
              Back
            </button>
          </div>
        ) : (
          <h1 className="px-2 text-sm font-medium leading-8 text-foreground">
            Settings
          </h1>
        )}
        <div className="relative mt-3">
          <Search
            className="pointer-events-none absolute left-2.5 top-1/2 size-4 shrink-0 -translate-y-1/2 text-neutral-400 dark:text-neutral-500"
            aria-hidden
          />
          <Input
            ref={searchRef}
            type="search"
            name="settings-search"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder="Search settings"
            aria-label="Search settings"
            autoComplete="off"
            className="h-9 rounded-md border-neutral-200 bg-white pl-8 pr-8 text-[15px] shadow-none placeholder:text-neutral-400 pointer-coarse:h-11 dark:border-neutral-800 dark:bg-neutral-900/60 dark:placeholder:text-neutral-500 [&::-webkit-search-cancel-button]:hidden"
            onKeyDown={(event) => {
              if (event.key === 'Escape' && query) setQuery('')
            }}
          />
          {query ? (
            <button
              type="button"
              aria-label="Clear settings search"
              onClick={() => {
                setQuery('')
                searchRef.current?.focus()
              }}
              className="absolute right-1 top-1/2 flex size-6 -translate-y-1/2 items-center justify-center rounded text-muted-foreground outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50"
            >
              <span
                className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                aria-hidden="true"
              />
              <X className="size-4" aria-hidden />
            </button>
          ) : null}
        </div>
      </div>

      <nav
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-3 pt-5 pb-6"
        aria-label="Settings sections"
      >
        <NavGroup label="ADE">
          <NavItem
            icon={Settings2}
            label="General"
            selected={generalSelected}
            onClick={onSelectGeneral}
          />
        </NavGroup>

        <NavGroup
          label="Workers"
          aside={
            !isLoading && !isError ? (
              <span
                className="tabular-nums text-neutral-400 dark:text-neutral-500"
                aria-live="polite"
              >
                {filteredEntries.length}
              </span>
            ) : null
          }
        >
          {isLoading ? <NavigationLoading /> : null}
          {isError ? (
            <p className="px-2 py-2 text-sm leading-5 text-destructive">
              {errorMessage ?? 'Could not load worker settings.'}
            </p>
          ) : null}
          {!isLoading && !isError && filteredEntries.length === 0 ? (
            <p className="px-2 py-2 text-sm leading-5 text-muted-foreground">
              {entries.length === 0
                ? 'No workers with a settings UI are registered.'
                : `No settings match “${query.trim()}”.`}
            </p>
          ) : null}
          {filteredEntries.map((entry) => (
            <NavItem
              key={entry.id}
              icon={entry.icon}
              label={entry.label}
              title={entry.description}
              selected={selectedId === entry.id}
              onClick={() => onSelectWorker(entry.id)}
            />
          ))}
        </NavGroup>
      </nav>
    </aside>
  )
}

function NavGroup({
  label,
  aside,
  children,
}: {
  label: string
  aside?: ReactNode
  children: ReactNode
}) {
  return (
    <div className="mb-6">
      <h2 className="flex items-baseline gap-2 px-2 pb-1 text-sm font-medium leading-5 text-muted-foreground">
        {label}
        {aside}
      </h2>
      {/* biome-ignore lint/a11y/noRedundantRoles: list-none removes native list semantics in Safari/VoiceOver. */}
      <ul role="list" className="flex list-none flex-col gap-0.5">
        {children}
      </ul>
    </div>
  )
}

interface NavItemProps {
  icon: LucideIcon
  label: string
  title?: string
  selected: boolean
  onClick: () => void
}

function NavItem({
  icon: Icon,
  label,
  title,
  selected,
  onClick,
}: NavItemProps) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        title={title}
        aria-current={selected ? 'page' : undefined}
        className={cn(
          'flex h-9 w-full items-center gap-2.5 rounded-md px-2 text-left text-[15px] leading-5 outline-none transition-colors duration-100 ease-out select-none focus-visible:ring-3 focus-visible:ring-ring/50 pointer-coarse:h-11',
          selected
            ? 'bg-neutral-200/70 text-foreground dark:bg-neutral-800/70'
            : 'text-neutral-600 hover:bg-neutral-200/50 hover:text-foreground dark:text-neutral-400 dark:hover:bg-neutral-900',
        )}
      >
        <Icon
          className={cn(
            'size-4 shrink-0',
            selected
              ? 'text-foreground'
              : 'text-neutral-500 dark:text-neutral-500',
          )}
          aria-hidden
        />
        <span className="min-w-0 flex-1 truncate">{label}</span>
      </button>
    </li>
  )
}

function NavigationLoading() {
  return (
    <>
      {[0, 1, 2].map((item) => (
        <li key={item} className="flex h-9 items-center gap-2.5 px-2">
          <Skeleton className="size-4 shrink-0" />
          <Skeleton className="h-4 w-32" />
        </li>
      ))}
    </>
  )
}
