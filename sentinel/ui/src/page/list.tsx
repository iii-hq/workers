import {
  Button,
  Checkbox,
  Chip,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  List,
  ListItem,
  Panel,
  SearchField,
  SegmentedControl,
  Select,
  Skeleton,
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
  total: number
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
  total,
  workers,
}: Props) {
  const scope = scopeOf(filters.statuses)
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

  const within = WINDOW_WORDS[filters.window]
  const shown = `${groups.length} ${groups.length === 1 ? 'group' : 'groups'}${within ? ` ${within}` : ''}`
  const hidden =
    scope === 'open' && counts && counts.resolved + counts.ignored > 0
      ? `${counts.resolved} resolved · ${counts.ignored} ignored hidden by this filter`
      : null

  return (
    <div className="sentinel-ui-list">
      <div className="sentinel-ui-filters" role="toolbar" aria-label="Filter groups">
        {narrow ? (
          <>
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
          {counts ? <Chip>{spaced(counts.open)} open groups</Chip> : null}
          {status && status.engine.trace_store !== 'unknown' ? (
            <Chip tone={status.engine.trace_store === 'disabled' ? 'warning' : 'neutral'}>
              trace store: {status.engine.trace_store === 'disabled' ? 'off' : status.engine.trace_store}
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
        <Panel className="sentinel-ui-empty">
          <Inbox size={16} aria-hidden="true" />
          <strong>Nothing here</strong>
          <p>
            No group matches this filter. Sentinel keeps counting every occurrence either way — resolved
            and ignored groups are one click away.
          </p>
        </Panel>
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
                        {Array.from({ length: 9 }, (_, cell) => (
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

      {groups.length > 0 ? (
        <div className="sentinel-ui-foot">
          <span>
            {shown}
            {total > groups.length ? ` · showing ${groups.length} of ${spaced(total)}` : ''} · sorted
            by priority (regressions first, then last seen)
          </span>
          {hidden ? <span className="sentinel-ui-quiet">{hidden}</span> : null}
        </div>
      ) : null}
    </div>
  )
}

/** The issue cell: the exception type in bold, the message after it, and
    where it failed on the line below. */
function Issue({ group }: { group: GroupSummary }) {
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
        <span className="sentinel-ui-issue-title">
          {type ? <b>{type}</b> : null} {rest}
          {group.source === 'log' ? <Chip className="sentinel-ui-source">log</Chip> : null}
        </span>
        <span className="sentinel-ui-issue-where">{where}</span>
      </div>
    </div>
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
    <ListItem
      onClick={() => onOpen(group.id)}
      data-regressed={group.status === 'regressed' ? 'true' : undefined}
      label={<Issue group={group} />}
      trailing={
        <span className="sentinel-ui-row-trailing">
          <StatusBadge status={group.status} />
          <span className="sentinel-ui-number">{spaced(group.occurrence_count)}</span>
          <span className="sentinel-ui-when">{ago(group.last_seen_ms, now)}</span>
        </span>
      }
    />
  )
}
