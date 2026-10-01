/* The Log's commits, as in WebStorm's log: a filter bar, then one 24px row
   per commit. Each row holds its slice of the graph, its subject with the
   branch and tag chips on its commit, its author and its date.

   The list is windowed and paged: rows load as the scroll nears the end.
   It is a listbox that keeps focus itself; a click selects, and a double
   click or Enter opens the commit's first file. The commits the IDE's
   HEAD has are tinted in its branch's colour, as WebStorm marks the
   current branch; every subject stays in full ink.

   In a narrow pane each row takes two lines: the subject over its author
   and date. */

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  EmptyState,
  SearchField,
  uiClasses,
} from '@iii-dev/console-ui'
import { CaseSensitive, Check, ChevronDown, RefreshCw, Regex, SlidersHorizontal } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { type Glyph, GraphCell, glyphColor, MAX_LANES } from './CommitGraph'
import type { ContextMenuAnchor } from './ContextMenu'
import type { LogCommit, LogFilter, LogRef } from './git-log-window'
import type { GitLogState } from './use-git-log'
import { speedMarks, useRowNav } from './use-row-nav'
import { VirtualList } from './VirtualList'

export const COMMIT_ROW = 24
const COMMIT_ROW_NARROW = 48

const DAY = 86_400
const RANGES: ReadonlyArray<{ label: string; days: number | null }> = [
  { label: 'Any time', days: null },
  { label: 'Last 24 hours', days: 1 },
  { label: 'Last 7 days', days: 7 },
  { label: 'Last 30 days', days: 30 },
  { label: 'Last year', days: 365 },
]

const when = new Intl.DateTimeFormat(undefined, { dateStyle: 'short', timeStyle: 'short' })

/** The listed commits the IDE's HEAD has: the `seeds` (HEAD, or where it
    meets the branch shown) and every parent of one it has. Topo order lists
    a commit before its parents, so one pass finds them. Null while no seed
    is listed: nothing is known yet, so nothing fades. */
function reachable(commits: readonly LogCommit[], seeds: readonly string[]): ReadonlySet<string> | null {
  const have = new Set<string>(seeds)
  if (!commits.some((commit) => have.has(commit.sha))) return null
  for (const commit of commits) {
    if (have.has(commit.sha)) for (const parent of commit.parents) have.add(parent)
  }
  return have
}

/** The chips on one commit: a local branch and its remote side by side
    read as one ("main · origin"), and past two the rest fold into "+n". */
function chipsOf(refs: readonly LogRef[], remotes: ReadonlySet<string>): Array<{ ref: LogRef; remote: string | null }> {
  const locals = new Set(refs.filter((ref) => ref.kind === 'local').map((ref) => ref.name))
  const chips: Array<{ ref: LogRef; remote: string | null }> = []
  for (const ref of refs) {
    if (ref.kind === 'remote') {
      const slash = ref.name.indexOf('/')
      const remote = slash > 0 && remotes.has(ref.name.slice(0, slash)) ? ref.name.slice(0, slash) : null
      const local = remote === null ? null : ref.name.slice(slash + 1)
      if (local !== null && locals.has(local)) {
        const chip = chips.find((candidate) => candidate.ref.kind === 'local' && candidate.ref.name === local)
        if (chip) {
          chip.remote = remote
          continue
        }
      }
    }
    chips.push({ ref, remote: null })
  }
  return chips
}

function Toggle({
  label,
  pressed,
  onChange,
  children,
}: {
  label: string
  pressed: boolean
  onChange(next: boolean): void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      className="shui-git-toggle"
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      onClick={() => onChange(!pressed)}
    >
      {children}
    </button>
  )
}

function Menu({ label, value, children }: { label: string; value: string | null; children: ReactNode }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="shui-git-filter" data-set={value !== null || undefined}>
        {value === null ? label : `${label}: ${value}`}
        <ChevronDown aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="shui-git-filter-menu">
        {children}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export function GitCommitList({
  log,
  filter,
  onFilter,
  branchLabel,
  branches,
  onBranch,
  seeds,
  headColor,
  narrow = false,
  prefix,
  labels,
  remotes,
  glyph,
  rings,
  selected,
  onSelect,
  onAct,
  onMenu,
  onTap,
}: {
  log: GitLogState
  filter: LogFilter
  onFilter(next: LogFilter): void
  /** The branch the tree limits the log to, or null for all of them. */
  branchLabel: string | null
  /** The Branch filter's choices: the tree's node id and its name. */
  branches: ReadonlyArray<{ id: string; name: string }>
  /** Limit the log to a tree node's branch, or null for all of them. */
  onBranch(id: string | null): void
  /** Where the IDE's HEAD history starts in this log: HEAD itself, or its
      merge bases with the branch shown. */
  seeds: readonly string[]
  /** The colour of HEAD's branch: the commits it has are tinted with it. */
  headColor: string
  narrow?: boolean
  /** The IDE's folder below the repository's top (`''` at the top). */
  prefix: string
  labels: ReadonlyMap<string, LogRef[]>
  remotes: ReadonlySet<string>
  glyph(name: string | null): Glyph
  /** Commits a worktree has checked out: `here` for the IDE's own. */
  rings: ReadonlyMap<string, 'here' | 'worktree'>
  selected: string | null
  onSelect(sha: string | null): void
  onAct(commit: LogCommit): void
  onMenu(commit: LogCommit, anchor: ContextMenuAnchor): void
  /** A click on a row, after it is selected (a narrow pane drills in). */
  onTap?(commit: LogCommit): void
}) {
  const { commits, graph } = log
  const rowHeight = narrow ? COMMIT_ROW_NARROW : COMMIT_ROW
  const [scrollTo, setScrollTo] = useState<number | null>(null)
  const domId = useId()
  const authors = useMemo(() => [...new Set(commits.map((commit) => commit.author))].sort().slice(0, 40), [commits])
  const color = (key: string | null) => glyphColor(glyph(key))
  const nav = useRowNav<LogCommit>({
    items: commits,
    idOf: (commit) => commit.sha,
    labelOf: (commit) => commit.subject,
    domId,
    selected,
    onSelect,
    onAct,
    onMenu,
    onClickRow: onTap,
    reveal: setScrollTo,
    pageSize: 20,
    // The log re-picks a selection it lost (GitLogTab); a parent picked in
    // the details may be unlisted and stays selected.
    handOff: false,
  })
  // A new selection shows wherever it came from (the tree, a parent link),
  // once: a reload that only moves its row leaves the scroll alone.
  const revealed = useRef<string | null>(null)
  const selectedIndex = nav.activeIndex
  useEffect(() => {
    if (selected === null || selected === revealed.current || selectedIndex < 0) return
    revealed.current = selected
    setScrollTo(selectedIndex)
  }, [selected, selectedIndex])
  // Near the end, the next page: checked as the view moves and again when a
  // read settles, since a view that reached the end mid-read does not move.
  const lastShown = useRef(0)
  const nearEnd = () => {
    if (!log.done && !log.loading && lastShown.current > commits.length - 200) log.loadMore()
  }
  // Re-checked when a read settles or rows arrive.
  useEffect(nearEnd, [log.loading, log.done, commits.length])
  const listRef = useCallback((node: HTMLDivElement | null) => node?.setAttribute('data-git-focus', ''), [])
  // One graph width for the whole log, so every subject starts at one x.
  const maxLanes = narrow ? 5 : MAX_LANES
  const lanes = useMemo(
    () =>
      graph === null
        ? 0
        : Math.min(
            maxLanes,
            graph.reduce((most, row) => Math.max(most, row.width, row.lane + 1), 1),
          ),
    [graph, maxLanes],
  )
  // The graph's parents are whole only without text, user and date filters.
  const inHead = useMemo(() => (graph === null ? null : reachable(commits, seeds)), [graph, commits, seeds])
  const inFolder = filter.paths !== undefined && filter.paths.length > 0
  const folder = prefix.replace(/\/$/, '')
  // The IDE's folder, or the files a "Show history" narrowed the log to.
  const onlyFolder = inFolder && filter.paths?.length === 1 && filter.paths[0] === folder
  const pathsValue = !inFolder ? null : onlyFolder ? folder : (filter.paths?.[0]?.split('/').pop() ?? null)
  const [filtersOpen, setFiltersOpen] = useState(false)
  const filtersId = useId()
  const activeFilters = [
    filter.regex === true,
    filter.caseSensitive === true,
    branchLabel !== null,
    filter.author !== undefined,
    filter.since !== undefined,
    inFolder,
  ].filter(Boolean).length
  const filtered = (filter.text ?? '') !== '' || filter.author !== undefined || filter.since !== undefined || inFolder

  const filterControls = (
    <>
      <Toggle
        label="Regular expression"
        pressed={filter.regex === true}
        onChange={(regex) => onFilter({ ...filter, regex })}
      >
        <Regex aria-hidden />
      </Toggle>
      <Toggle
        label="Match case"
        pressed={filter.caseSensitive === true}
        onChange={(caseSensitive) => onFilter({ ...filter, caseSensitive })}
      >
        <CaseSensitive aria-hidden />
      </Toggle>
      <Menu label="Branch" value={branchLabel}>
        <DropdownMenuItem onSelect={() => onBranch(null)}>
          All branches
          {branchLabel === null ? <Check aria-hidden className="shui-git-filter-check" /> : null}
        </DropdownMenuItem>
        {branches.map((branch) => (
          <DropdownMenuItem key={branch.id} onSelect={() => onBranch(branch.id)}>
            {branch.name}
            {branchLabel === branch.name ? <Check aria-hidden className="shui-git-filter-check" /> : null}
          </DropdownMenuItem>
        ))}
      </Menu>
      <Menu label="User" value={filter.author ?? null}>
        <DropdownMenuItem onSelect={() => onFilter({ ...filter, author: undefined })}>Anyone</DropdownMenuItem>
        {authors.map((author) => (
          <DropdownMenuItem key={author} onSelect={() => onFilter({ ...filter, author })}>
            {author}
            {filter.author === author ? <Check aria-hidden className="shui-git-filter-check" /> : null}
          </DropdownMenuItem>
        ))}
      </Menu>
      <Menu label="Date" value={filter.since === undefined ? null : (filter.sinceLabel ?? 'Custom')}>
        {RANGES.map((option) => (
          <DropdownMenuItem
            key={option.label}
            onSelect={() =>
              onFilter({
                ...filter,
                since: option.days === null ? undefined : Math.floor(Date.now() / 1000) - option.days * DAY,
                sinceLabel: option.days === null ? undefined : option.label,
              })
            }
          >
            {option.label}
          </DropdownMenuItem>
        ))}
      </Menu>
      {prefix !== '' || inFolder ? (
        <Menu label="Paths" value={pathsValue}>
          <DropdownMenuItem onSelect={() => onFilter({ ...filter, paths: undefined })}>
            Whole repository
            {inFolder ? null : <Check aria-hidden className="shui-git-filter-check" />}
          </DropdownMenuItem>
          {prefix !== '' ? (
            <DropdownMenuItem onSelect={() => onFilter({ ...filter, paths: [folder] })}>
              {folder}
              {onlyFolder ? <Check aria-hidden className="shui-git-filter-check" /> : null}
            </DropdownMenuItem>
          ) : null}
          {inFolder && !onlyFolder ? (
            <DropdownMenuItem onSelect={() => undefined}>
              {filter.paths?.[0]}
              <Check aria-hidden className="shui-git-filter-check" />
            </DropdownMenuItem>
          ) : null}
        </Menu>
      ) : null}
    </>
  )

  return (
    <div className="shui-git-commits" data-pane="commits" style={{ '--head-color': headColor } as React.CSSProperties}>
      {/* biome-ignore lint/a11y/useSemanticElements: a filter bar, not a form: nothing is submitted */}
      <div className="shui-git-filters" role="group" aria-label="Filter the log">
        <SearchField
          className="shui-git-filter-text"
          value={filter.text ?? ''}
          onChange={(text) => onFilter({ ...filter, text: text === '' ? undefined : text })}
          placeholder="Text or hash"
          aria-label="Filter commits by message or hash"
          autoComplete="off"
          spellCheck={false}
        />
        {narrow ? (
          // A narrow pane keeps one row: the filters open below on demand.
          <button
            type="button"
            className="shui-git-filter"
            aria-expanded={filtersOpen}
            aria-controls={filtersId}
            data-set={activeFilters > 0 || undefined}
            onClick={() => setFiltersOpen((open) => !open)}
          >
            <SlidersHorizontal aria-hidden />
            {activeFilters > 0 ? `Filters · ${activeFilters}` : 'Filters'}
          </button>
        ) : (
          filterControls
        )}
        <button
          type="button"
          className="shui-git-toggle shui-git-refresh"
          aria-label="Refresh the log"
          title="Refresh the log"
          onClick={log.refresh}
          aria-busy={log.loading || undefined}
        >
          <RefreshCw aria-hidden className={log.loading ? uiClasses.spin : undefined} />
        </button>
      </div>
      {narrow && filtersOpen ? (
        // biome-ignore lint/a11y/useSemanticElements: the rest of the filter bar, not a form
        <div id={filtersId} className="shui-git-filters shui-git-filters-more" role="group" aria-label="More filters">
          {filterControls}
        </div>
      ) : null}
      {log.error !== null && commits.length === 0 ? (
        <p className="shui-git-note-line warn">{log.error}</p>
      ) : commits.length === 0 && !log.loading && filtered ? (
        <EmptyState
          compact
          title="No commits match"
          description="Nothing in this history matches the filters."
          action={{ label: 'Clear filters', onClick: () => onFilter({}) }}
        />
      ) : (
        // biome-ignore lint/a11y/noStaticElementInteractions: a focus-out listener around the windowed listbox
        <div className="shui-git-commit-scroll" onBlur={nav.listProps.onBlur}>
          <VirtualList
            rows={commits}
            rowHeight={rowHeight}
            rowKey={(commit) => commit.sha}
            className="shui-git-commit-list"
            role="listbox"
            aria-label="Commits"
            tabIndex={0}
            onKeyDown={nav.listProps.onKeyDown}
            aria-activedescendant={nav.listProps['aria-activedescendant']}
            keepIndex={nav.activeIndex}
            listRef={listRef}
            scrollToIndex={scrollTo}
            onRangeChange={(_first, last) => {
              lastShown.current = last
              nearEnd()
            }}
            renderRow={(commit, index) => {
              const row = graph?.[index] ?? null
              const refs = labels.get(commit.sha) ?? []
              const chips = chipsOf(refs, remotes)
              const shownChips = narrow ? 1 : 2
              const refChips = (
                <>
                  {chips.slice(0, shownChips).map(({ ref, remote }) => (
                    <span
                      key={ref.fullName}
                      className="shui-git-ref"
                      data-kind={ref.kind}
                      data-current={ref.current || undefined}
                      style={{ '--ref-color': color(ref.kind === 'tag' ? null : ref.name) } as React.CSSProperties}
                      title={ref.fullName}
                    >
                      <span className="shui-git-ref-name">{ref.name}</span>
                      {remote !== null ? <span className="shui-git-ref-remote">· {remote}</span> : null}
                    </span>
                  ))}
                  {chips.length > shownChips ? (
                    <span
                      className="shui-git-ref"
                      data-more=""
                      title={chips
                        .slice(shownChips)
                        .map((chip) => chip.ref.name)
                        .join('\n')}
                    >
                      +{chips.length - shownChips}
                    </span>
                  ) : null}
                </>
              )
              const byline = (
                <>
                  <span className="shui-git-commit-author" title={commit.email}>
                    {commit.author}
                  </span>
                  <span className="shui-git-commit-date">{when.format(commit.date * 1000)}</span>
                </>
              )
              return (
                // biome-ignore lint/a11y/useFocusableInteractive: the listbox holds focus and names this row through aria-activedescendant
                <div
                  role="option"
                  className="shui-git-commit"
                  data-narrow={narrow || undefined}
                  data-head={inHead?.has(commit.sha) ? '' : undefined}
                  aria-setsize={log.done ? commits.length : -1}
                  aria-posinset={index + 1}
                  {...nav.rowProps(index)}
                >
                  <span className="shui-git-commit-graph">
                    {row !== null ? (
                      <GraphCell
                        row={row}
                        height={rowHeight}
                        color={color}
                        ring={rings.get(commit.sha) ?? null}
                        lanes={lanes}
                        maxLanes={maxLanes}
                      />
                    ) : null}
                  </span>
                  <span className="shui-git-commit-main">
                    <span className="shui-git-commit-subject">
                      <span className="shui-git-commit-text">
                        {speedMarks(commit.subject, nav.query).map((part, at) =>
                          part.hit ? (
                            <mark key={at} className="shui-git-hit">
                              {part.text}
                            </mark>
                          ) : (
                            part.text
                          ),
                        )}
                      </span>
                      {narrow ? null : refChips}
                    </span>
                    {narrow ? (
                      <span className="shui-git-commit-byline">
                        {byline}
                        {refChips}
                      </span>
                    ) : null}
                  </span>
                  {narrow ? null : byline}
                </div>
              )
            }}
          />
        </div>
      )}
      <p
        className="shui-git-status"
        data-warn={(log.error !== null && commits.length > 0) || undefined}
        title={log.error ?? undefined}
        aria-live="polite"
      >
        {commits.length === 0
          ? log.loading
            ? 'reading the log…'
            : '0 commits'
          : log.error !== null
            ? `${commits.length.toLocaleString()} commits · the next page failed: ${log.error}`
            : `${commits.length.toLocaleString()}${log.done ? '' : '+'} commits${log.done ? ' · start of history' : ''}${graph === null ? ' · the graph shows without text, user or date filters' : ''}`}
      </p>
      {nav.query !== '' ? <span className="shui-git-speed">{nav.query}</span> : null}
    </div>
  )
}
