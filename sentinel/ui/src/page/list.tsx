import {
  Button,
  Checkbox,
  Chip,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  EmptyState,
  List,
  ListItem,
  SearchField,
  SegmentedControl,
  Select,
  Skeleton,
  StatusBar,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { ChevronRight, Inbox } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { GroupSummary, StatusResponse } from '../api'
import type { Filters } from './index'
import { Dot, Sparkline, StatusBadge } from './marks'
import {
  KIND_LABEL,
  PAGE_SIZE,
  RELEVANCE,
  SCOPES,
  SCOPE_STATES,
  WINDOWS,
  ago,
  bulkActions,
  scopeOf,
  spaced,
  splitTitle,
  statusLook,
  versionRange,
} from './present.js'

interface Props {
  busy: boolean
  onBulk: (action: string, groupIds: string[]) => void
  status: StatusResponse | null
  filters: Filters
  groups: GroupSummary[]
  loading: boolean
  narrow: boolean
  now: number
  onFilters: (next: Filters) => void
  onOpen: (groupId: string) => void
  onMore: () => void
  total: number
  /** Both sides of triage under the current filters. */
  relevantTotal: number
  noiseTotal: number
  workers: string[]
}

const WINDOW_WORDS: Record<string, string> = {
  '24h': 'in the last 24 h',
  '7d': 'in the last 7 d',
  '30d': 'in the last 30 d',
}

export function GroupsListView({
  busy,
  onBulk,
  status,
  filters,
  groups,
  loading,
  narrow,
  now,
  onFilters,
  onOpen,
  onMore,
  total,
  relevantTotal,
  noiseTotal,
  workers,
}: Props) {
  const scope = scopeOf(filters.statuses)
  const relevance = filters.relevance ?? 'relevant'
  const counts = status?.groups
  const [picked, setPicked] = useState<string[]>([])

  // A row that left the filter is no longer selectable: keeping it would
  // apply an action to something the person cannot see.
  const visible = new Set(groups.map((group) => group.id))
  const selection = picked.filter((id) => visible.has(id))
  const actions = bulkActions(
    selection.map((id) => groups.find((group) => group.id === id)?.status ?? 'new'),
  )
  useEffect(() => {
    if (narrow) setPicked([])
  }, [narrow])

  const toggle = (id: string) =>
    setPicked((previous) =>
      previous.includes(id) ? previous.filter((other) => other !== id) : [...previous, id],
    )
  const apply = (action: string) => {
    onBulk(action, selection)
    setPicked([])
  }

  const setScope = (next: string) => onFilters({ ...filters, statuses: SCOPE_STATES[next] ?? [] })
  const setWindow = (next: string) => onFilters({ ...filters, window: next })
  const setWorker = (next: string) => onFilters({ ...filters, service: next })
  const setSearch = (next: string) => onFilters({ ...filters, search: next })
  const setRelevance = (next: string) => onFilters({ ...filters, relevance: next })
  // Each side says how much it holds, so the noise is one click away and
  // never a mystery.
  const sideCount = (value: string) =>
    value === 'relevant' ? relevantTotal : value === 'noise' ? noiseTotal : relevantTotal + noiseTotal
  // "All workers" is the Select's own empty option: an option whose value is
  // an empty string is not allowed.
  const worker = {
    value: filters.service || undefined,
    placeholder: 'All workers',
    allowEmpty: true,
    emptyLabel: 'All workers',
    onClear: () => setWorker(''),
    onChange: setWorker,
    options: workers.map((name) => ({ value: name, label: name })),
  }
  const search = (
    <SearchField
      aria-label="Search groups"
      className="sentinel-ui-search"
      placeholder="Search title, message or function"
      value={filters.search}
      onChange={setSearch}
    />
  )

  // One way out of an empty list: undo the most specific filter. If the list
  // is still empty, the next empty state offers the next one.
  const widen =
    filters.search || filters.service
      ? { label: 'Clear search and worker', onClick: () => onFilters({ ...filters, search: '', service: '' }) }
      : relevance === 'relevant' && noiseTotal > 0
        ? { label: `Show the noise (${spaced(noiseTotal)})`, onClick: () => setRelevance('noise') }
        : filters.window !== 'all'
        ? { label: 'Show all time', onClick: () => setWindow('all') }
        : scope !== 'open'
          ? { label: 'Show open groups', onClick: () => setScope('open') }
          : undefined

  return (
    <div className="sentinel-ui-list">
      <div className="sentinel-ui-filters" role="toolbar" aria-label="Filter groups">
        {narrow ? (
          <>
            <Select
              aria-label="Relevance"
              sheetTitle="Relevance"
              className="sentinel-ui-filter"
              value={relevance}
              onChange={setRelevance}
              options={RELEVANCE.map((option) => ({
                ...option,
                label: `${option.label} · ${spaced(sideCount(option.value))}`,
              }))}
            />
            <Select
              aria-label="Which groups"
              sheetTitle="Which groups"
              className="sentinel-ui-filter"
              value={scope}
              onChange={setScope}
              options={SCOPES}
            />
            <Select
              aria-label="Time window"
              sheetTitle="Time window"
              className="sentinel-ui-filter"
              value={filters.window}
              onChange={setWindow}
              options={WINDOWS}
            />
            <Select aria-label="Worker" sheetTitle="Worker" className="sentinel-ui-filter" {...worker} />
            {search}
          </>
        ) : (
          <>
            <SegmentedControl
              variant="radio"
              aria-label="Relevance"
              value={relevance}
              onChange={setRelevance}
              options={RELEVANCE.map((option) => ({
                value: option.value,
                icon: false as const,
                label: (
                  <>
                    {option.label}
                    <span className="sentinel-ui-tab-count">{spaced(sideCount(option.value))}</span>
                  </>
                ),
              }))}
            />
            <SegmentedControl
              variant="radio"
              aria-label="Which groups"
              value={scope}
              onChange={setScope}
              options={SCOPES.map((option) => ({ ...option, icon: false as const }))}
            />
            <Select aria-label="Worker" className="sentinel-ui-worker" {...worker} />
            <SegmentedControl
              variant="radio"
              aria-label="Time window"
              value={filters.window}
              onChange={setWindow}
              options={WINDOWS.map((option) => ({ ...option, icon: false as const }))}
            />
            {search}
          </>
        )}
        <div className="sentinel-ui-summary">
          {counts && counts.regressed > 0 ? (
            <Chip tone="danger">
              {counts.regressed} {counts.regressed === 1 ? 'regression' : 'regressions'}
            </Chip>
          ) : null}
        </div>
      </div>

      {selection.length > 0 ? (
        <div className="sentinel-ui-bulk" role="region" aria-label="Selected groups">
          <span>{selection.length} selected</span>
          {actions.length === 0 ? (
            <span className="sentinel-ui-quiet">nothing applies to all of them — narrow the selection</span>
          ) : null}
          {actions.includes('resolve') ? (
            <Button size="sm" variant="pill" disabled={busy} onClick={() => apply('resolve')}>
              Resolve
            </Button>
          ) : null}
          {actions.includes('ignore') ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="sm" variant="pill" disabled={busy}>
                  Ignore
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent>
                <DropdownMenuItem onSelect={() => apply('ignore')}>Forever</DropdownMenuItem>
                <DropdownMenuItem onSelect={() => apply('ignore-version')}>
                  Until the worker version changes
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
          {actions.includes('unignore') ? (
            <Button size="sm" variant="pill" disabled={busy} onClick={() => apply('unignore')}>
              Unignore
            </Button>
          ) : null}
          {actions.includes('reopen') ? (
            <Button size="sm" variant="pill" disabled={busy} onClick={() => apply('reopen')}>
              Reopen
            </Button>
          ) : null}
          <Button size="sm" variant="ghost" onClick={() => setPicked([])}>
            Clear
          </Button>
        </div>
      ) : null}

      {!loading && groups.length === 0 ? (
        relevance === 'relevant' && noiseTotal > 0 && !filters.search && !filters.service ? (
          <EmptyState
            icon={Inbox}
            title="Nothing here needs attention"
            description="Every group in this view was triaged as noise: restarts, closed tabs, bad arguments a caller corrected. They keep counting."
            action={widen}
          />
        ) : (
          <EmptyState
            icon={Inbox}
            title="No group matches this filter"
            description="Sentinel keeps counting every occurrence either way; resolved and ignored groups have their own filter."
            action={widen}
          />
        )
      ) : narrow ? (
        <List aria-label="Error groups">
          {loading && groups.length === 0
            ? Array.from({ length: 5 }, (_, index) => (
                <ListItem key={index} as="div" label={<Skeleton />} description={<Skeleton />} />
              ))
            : groups.map((group) => (
                <GroupRowNarrow key={group.id} group={group} now={now} onOpen={onOpen} />
              ))}
        </List>
      ) : (
        <TableViewport>
          <TableFrame>
            <Table className="sentinel-ui-groups">
              <TableHeader>
                <TableRow>
                  <TableHead className="sentinel-ui-pick">
                    <Checkbox
                      aria-label="Select every group shown"
                      checked={selection.length > 0 && selection.length === groups.length}
                      indeterminate={selection.length > 0 && selection.length < groups.length}
                      onChange={(event) =>
                        setPicked(event.target.checked ? groups.map((group) => group.id) : [])
                      }
                    />
                  </TableHead>
                  <TableHead className="sentinel-ui-grow">Issue</TableHead>
                  <TableHead align="right">Count</TableHead>
                  <TableHead align="right">Sessions</TableHead>
                  <TableHead>First seen</TableHead>
                  <TableHead>Last seen</TableHead>
                  <TableHead>24 h</TableHead>
                  <TableHead>Triage</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead className="sentinel-ui-chevron">
                    <span className="sentinel-ui-visually-hidden">Open</span>
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {loading && groups.length === 0
                  ? Array.from({ length: 5 }, (_, index) => (
                      <TableRow key={index}>
                        {Array.from({ length: 10 }, (_, cell) => (
                          <TableCell key={cell}>
                            <Skeleton />
                          </TableCell>
                        ))}
                      </TableRow>
                    ))
                  : groups.map((group) => (
                      <GroupRow
                        key={group.id}
                        group={group}
                        now={now}
                        onOpen={onOpen}
                        picked={selection.includes(group.id)}
                        onPick={() => toggle(group.id)}
                      />
                    ))}
              </TableBody>
            </Table>
          </TableFrame>
        </TableViewport>
      )}

      {total > groups.length ? (
        // Without this every group past the first page was unreachable.
        <div className="sentinel-ui-more">
          <Button size="sm" variant="pill" disabled={loading} onClick={onMore}>
            Show {Math.min(PAGE_SIZE, total - groups.length)} more
          </Button>
        </div>
      ) : null}
    </div>
  )
}

/**
 * What the list is showing, under it rather than in the header: counts and
 * engine facts belong to the quiet strip, and it stays put while the list
 * scrolls.
 */
export function ListStatus({
  answered,
  narrow,
  now,
  shown,
  status,
  total,
}: {
  /** The filters the counts answer — not the ones being fetched right now. */
  answered: Pick<Filters, 'statuses' | 'window' | 'relevance'>
  narrow: boolean
  now: number
  shown: number
  status: StatusResponse | null
  total: number
}) {
  const counts = status?.groups
  const within = WINDOW_WORDS[answered.window]
  const store = status?.engine.trace_store
  // Only the parts that are not zero: "2 ignored", not "0 resolved · 2 ignored".
  const hidden = counts
    ? [counts.resolved && `${spaced(counts.resolved)} resolved`, counts.ignored && `${spaced(counts.ignored)} ignored`]
        .filter(Boolean)
        .join(' · ')
    : ''
  return (
    <StatusBar
      className="sentinel-ui-statusbar"
      end={
        narrow ? null : (
          <>
            {/* Off is a banner above the list; here it is only a fact. */}
            {store === 'memory' ? <span>trace store: memory</span> : null}
            {counts?.last_seen_ms ? <span>ingested {ago(counts.last_seen_ms, now)}</span> : null}
          </>
        )
      }
    >
      <span>
        {total > shown ? `${spaced(shown)} of ${spaced(total)} groups` : `${spaced(shown)} ${shown === 1 ? 'group' : 'groups'}`}
        {within ? ` ${within}` : ''}
      </span>
      {narrow ? null : (
        <span>
          {answered.relevance === 'noise' ? 'by triage kind, then last seen' : 'regressions first, then last seen'}
        </span>
      )}
      {/* All-time counts, not the window's: said so, rather than "hidden" beside "in the last 24 h". */}
      {hidden && scopeOf(answered.statuses) === 'open' ? <span>all time: {hidden}, not in Open</span> : null}
    </StatusBar>
  )
}

/** The issue cell: the exception type in bold, the message after it, and
    where it failed on the line below. */
function Issue({ group, extra }: { group: GroupSummary; extra?: string }) {
  const { type, rest } = splitTitle(group.title, group.exception_type)
  const where = [
    group.service_name,
    group.function_id,
    versionRange(group.first_version, group.last_version),
  ]
    .filter(Boolean)
    .join(' · ')
  return (
    <div className="sentinel-ui-issue">
      <Dot tone={statusLook(group.status).dot} pulse={group.status === 'investigating'} />
      <div className="sentinel-ui-issue-copy">
        <span className="sentinel-ui-issue-title" title={group.title}>
          {type ? <b>{type}</b> : null} {rest}
          {group.source === 'log' ? <Chip className="sentinel-ui-source">log</Chip> : null}

        </span>
        <span className="sentinel-ui-issue-where">{where}</span>
        {extra ? <span className="sentinel-ui-issue-extra">{extra}</span> : null}
      </div>
    </div>
  )
}

/** What triage made of the group, or a dash while it waits. */
function Kind({ group }: { group: GroupSummary }) {
  if (!group.triage) return <span className="sentinel-ui-quiet">—</span>
  return (
    <Chip
      title={
        group.triage.source === 'rule'
          ? 'triaged by rule'
          : `triaged by the judge, ${Math.round(group.triage.confidence * 100)}% sure`
      }
    >
      {KIND_LABEL[group.triage.kind] ?? group.triage.kind}
    </Chip>
  )
}

function GroupRow({
  group,
  now,
  onOpen,
  onPick,
  picked,
}: {
  group: GroupSummary
  now: number
  onOpen: (groupId: string) => void
  onPick: () => void
  picked: boolean
}) {
  return (
    <TableRow
      interactive
      selected={picked}
      // A regression carries a wash of its own: it is the one row in the
      // list that says somebody's fix did not hold.
      data-regressed={group.status === 'regressed' ? 'true' : undefined}
      onClick={() => onOpen(group.id)}
    >
      <TableCell
        className="sentinel-ui-pick"
        // The checkbox is not the row: clicking it selects, clicking the row
        // opens.
        onClick={(event) => event.stopPropagation()}
      >
        <Checkbox aria-label={`Select ${group.title}`} checked={picked} onChange={onPick} />
      </TableCell>
      <TableCell className="sentinel-ui-grow">
        <Issue group={group} />
      </TableCell>
      <TableCell align="right" className="sentinel-ui-number">
        {spaced(group.occurrence_count)}
      </TableCell>
      <TableCell align="right" className="sentinel-ui-number">
        {group.sessions_affected > 0 ? spaced(group.sessions_affected) : '—'}
      </TableCell>
      <TableCell className="sentinel-ui-when sentinel-ui-quiet">{ago(group.first_seen_ms, now)}</TableCell>
      <TableCell className="sentinel-ui-when">{ago(group.last_seen_ms, now)}</TableCell>
      <TableCell>
        <Sparkline group={group} now={now} />
      </TableCell>
      <TableCell>
        <Kind group={group} />
      </TableCell>
      <TableCell>
        <StatusBadge status={group.status} />
      </TableCell>
      <TableCell className="sentinel-ui-chevron">
        <ChevronRight size={16} aria-hidden="true" />
      </TableCell>
    </TableRow>
  )
}

function GroupRowNarrow({
  group,
  now,
  onOpen,
}: {
  group: GroupSummary
  now: number
  onOpen: (groupId: string) => void
}) {
  return (
    // The title is the task; the facts that sat beside it go under it, where
    // they cost a line instead of half the width. The dot carries the tone.
    <ListItem
      onClick={() => onOpen(group.id)}
      data-regressed={group.status === 'regressed' ? 'true' : undefined}
      label={
        <Issue
          group={group}
          extra={[
            group.status,
            group.triage ? (KIND_LABEL[group.triage.kind] ?? group.triage.kind) : null,
            spaced(group.occurrence_count),
            ago(group.last_seen_ms, now),
          ]
            .filter(Boolean)
            .join(' · ')}
        />
      }
    />
  )
}
