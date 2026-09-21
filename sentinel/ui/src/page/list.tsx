import {
  Badge,
  Button,
  Checkbox,
  Chip,
  Eyebrow,
  SearchField,
  SegmentedControl,
  Select,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Skeleton,
  StatusDot,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  Toolbar,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { useState } from 'react'
import type { GroupStatus, GroupSummary, StatusResponse } from '../api'
import type { Filters } from './index'
import {
  OPEN_STATES,
  STATUS_PRESENTATION,
  bulkActions,
  ignoreSummary,
  sparklineBars,
} from './present.js'

// `icon: false` throughout: these are words, and the control's inferred
// glyphs would be decoration that means nothing here.
const WINDOWS = [
  { value: '1h', label: '1h', icon: false as const },
  { value: '24h', label: '24h', icon: false as const },
  { value: '7d', label: '7d', icon: false as const },
  { value: '30d', label: '30d', icon: false as const },
  { value: 'all', label: 'all', icon: false as const },
]

const SCOPES = [
  { value: 'open', label: 'open', icon: false as const },
  { value: 'resolved', label: 'resolved', icon: false as const },
  { value: 'ignored', label: 'ignored', icon: false as const },
  { value: 'everything', label: 'everything', icon: false as const },
]

const SCOPE_STATES: Record<string, GroupStatus[]> = {
  open: OPEN_STATES as GroupStatus[],
  resolved: ['resolved'],
  ignored: ['ignored'],
  everything: [],
}

interface Props {
  busy: boolean
  onBulk: (action: string, groupIds: string[]) => void
  counts?: StatusResponse['groups']
  filters: Filters
  groups: GroupSummary[]
  live: boolean
  loading: boolean
  narrow: boolean
  onFilters: (next: Filters) => void
  onOpen: (groupId: string) => void
  panelSide: 'left' | 'right'
  total: number
}

export function GroupsListView({
  busy,
  onBulk,
  counts,
  filters,
  groups,
  live,
  loading,
  narrow,
  onFilters,
  onOpen,
  total,
}: Props) {
  const scope = scopeOf(filters.statuses)
  const services = [...new Set(groups.map((group) => group.service_name))].sort()
  const [picked, setPicked] = useState<string[]>([])

  // A row that scrolled out of the filter is no longer selectable: keeping it
  // would apply an action to something the person cannot see.
  const visible = new Set(groups.map((group) => group.id))
  const selection = picked.filter((id) => visible.has(id))
  const actions = bulkActions(
    selection.map((id) => groups.find((group) => group.id === id)?.status ?? 'new'),
  )
  const toggle = (id: string) =>
    setPicked((previous) =>
      previous.includes(id) ? previous.filter((other) => other !== id) : [...previous, id],
    )

  return (
    <div className="sentinel-ui-list">
      <Toolbar aria-label="error filters">
        <SegmentedControl
          aria-label="which groups"
          value={scope}
          onChange={(next) => onFilters({ ...filters, statuses: SCOPE_STATES[next] ?? [] })}
          options={SCOPES}
        />
        <SegmentedControl
          aria-label="time window"
          value={filters.window}
          onChange={(next) => onFilters({ ...filters, window: next })}
          options={WINDOWS}
        />
        {narrow ? null : (
          <Select
            aria-label="worker"
            value={filters.service || undefined}
            placeholder="every worker"
            allowEmpty
            emptyLabel="every worker"
            onClear={() => onFilters({ ...filters, service: '' })}
            options={services.map((service) => ({ value: service, label: service }))}
            onChange={(next) => onFilters({ ...filters, service: next })}
          />
        )}
        <SearchField
          aria-label="search errors"
          className="sentinel-ui-search"
          placeholder="title, message or function"
          value={filters.search}
          onChange={(next) => onFilters({ ...filters, search: next })}
        />
      </Toolbar>

      <div className="sentinel-ui-counts">
        {counts ? (
          <>
            <Chip tone={counts.regressed > 0 ? 'danger' : 'neutral'}>
              {counts.regressed} regressed
            </Chip>
            <Chip tone="neutral">{counts.open} open</Chip>
            <Chip tone="neutral">{counts.resolved} resolved</Chip>
            <Chip tone="neutral">{counts.ignored} ignored</Chip>
          </>
        ) : null}
        <span className="sentinel-ui-liveness">
          <StatusDot tone={live ? 'ok' : 'ink'} />
          <span>{live ? 'live' : 'polling'}</span>
        </span>
      </div>

      {selection.length > 0 ? (
        <div className="sentinel-ui-bulk" role="region" aria-label="selected groups">
          <span>{selection.length} selected</span>
          {actions.length === 0 ? (
            <span className="sentinel-ui-bulk-none">
              nothing applies to all of them — narrow the selection
            </span>
          ) : null}
          {actions.includes('resolve') ? (
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => {
                onBulk('resolve', selection)
                setPicked([])
              }}
            >
              Resolve
            </Button>
          ) : null}
          {actions.includes('ignore') ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="sm" variant="ghost" disabled={busy}>
                  Ignore
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent>
                <DropdownMenuItem
                  onSelect={() => {
                    onBulk('ignore', selection)
                    setPicked([])
                  }}
                >
                  Forever
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() => {
                    onBulk('ignore-version', selection)
                    setPicked([])
                  }}
                >
                  Until the worker version changes
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
          {actions.includes('unignore') ? (
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => {
                onBulk('unignore', selection)
                setPicked([])
              }}
            >
              Stop ignoring
            </Button>
          ) : null}
          {actions.includes('reopen') ? (
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => {
                onBulk('reopen', selection)
                setPicked([])
              }}
            >
              Reopen
            </Button>
          ) : null}
          <Button size="sm" variant="ghost" onClick={() => setPicked([])}>
            Clear
          </Button>
        </div>
      ) : null}

      <TableViewport>
        <TableFrame>
          <Table density="compact">
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
                <TableHead className="sentinel-ui-grow">error</TableHead>
                {narrow ? null : <TableHead>worker</TableHead>}
                <TableHead>last 24h</TableHead>
                <TableHead align="right">count</TableHead>
                <TableHead align="right">last seen</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {loading && groups.length === 0
                ? Array.from({ length: 5 }, (_, index) => (
                    <TableRow key={index}>
                      <TableCell colSpan={narrow ? 5 : 6}>
                        <Skeleton />
                      </TableCell>
                    </TableRow>
                  ))
                : groups.map((group) => (
                    <GroupRow
                      key={group.id}
                      group={group}
                      narrow={narrow}
                      onOpen={onOpen}
                      picked={selection.includes(group.id)}
                      onPick={() => toggle(group.id)}
                    />
                  ))}
            </TableBody>
          </Table>
        </TableFrame>
      </TableViewport>
      {total > groups.length ? (
        <Eyebrow className="sentinel-ui-more">
          showing {groups.length} of {total}
        </Eyebrow>
      ) : null}
    </div>
  )
}

function GroupRow({
  group,
  narrow,
  onOpen,
  onPick,
  picked,
}: {
  group: GroupSummary
  narrow: boolean
  onOpen: (groupId: string) => void
  onPick: () => void
  picked: boolean
}) {
  const presentation = STATUS_PRESENTATION[group.status]
  return (
    <TableRow
      selected={picked}
      // A regression carries a wash of its own: it is the one row in the
      // list that says somebody's fix did not hold.
      data-regressed={group.status === 'regressed' ? 'true' : undefined}
      onClick={() => onOpen(group.id)}
      tabIndex={0}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onOpen(group.id)
        }
      }}
    >
      <TableCell
        className="sentinel-ui-pick"
        // The checkbox is not the row: clicking it selects, clicking the row
        // opens.
        onClick={(event) => event.stopPropagation()}
      >
        <Checkbox
          aria-label={`Select ${group.title}`}
          checked={picked}
          onChange={onPick}
        />
      </TableCell>
      <TableCell className="sentinel-ui-grow">
        <div className="sentinel-ui-row-title">
          <Badge variant={presentation.tone === 'danger' ? 'alert' : 'default'}>
            {presentation.label}
          </Badge>
          <span className="sentinel-ui-title">{group.title}</span>
          {group.has_diagnosis && group.status !== 'diagnosed' ? (
            <Chip tone="accent">diagnosed</Chip>
          ) : null}
        </div>
        <div className="sentinel-ui-row-detail">
          {group.function_id ?? group.service_name}
          {group.status === 'ignored' ? ` · ${ignoreSummary(group.ignore_rule)}` : ''}
          {group.sessions_affected > 0 ? ` · ${group.sessions_affected} sessions` : ''}
        </div>
      </TableCell>
      {narrow ? null : (
        <TableCell>
          <span className="sentinel-ui-worker">{group.service_name}</span>
          {group.last_version ? (
            <span className="sentinel-ui-version">{group.last_version}</span>
          ) : null}
        </TableCell>
      )}
      <TableCell>
        <Sparkline counts={group.sparkline} />
      </TableCell>
      <TableCell align="right">
        <span className="sentinel-ui-count">{group.occurrence_count}</span>
      </TableCell>
      <TableCell align="right">
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="sentinel-ui-when">{formatRelative(group.last_seen_ms)}</span>
          </TooltipTrigger>
          <TooltipContent>{new Date(group.last_seen_ms).toISOString()}</TooltipContent>
        </Tooltip>
      </TableCell>
    </TableRow>
  )
}

/** Twenty-four bars of CSS, oldest first. No SVG: the design system asks
    for none, and a bar chart this small does not need one. */
function Sparkline({ counts }: { counts: number[] }) {
  const bars = sparklineBars(counts)
  const total = counts.reduce((sum, count) => sum + count, 0)
  return (
    <span
      className="sentinel-ui-sparkline"
      role="img"
      aria-label={`${total} in the last ${counts.length} hours`}
    >
      {bars.map((bar, index) => (
        <span key={index} style={{ height: `${bar.height}%` }} data-empty={bar.count === 0} />
      ))}
    </span>
  )
}

function scopeOf(statuses: GroupStatus[]): string {
  for (const [scope, states] of Object.entries(SCOPE_STATES)) {
    if (states.length === statuses.length && states.every((state) => statuses.includes(state))) {
      return scope
    }
  }
  return 'everything'
}
