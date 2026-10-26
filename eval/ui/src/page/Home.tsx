import {
  Button,
  EmptyState,
  Eyebrow,
  IconButton,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  PageBody,
  PageHeader,
  PageMain,
  SearchField,
  SegmentedControl,
  Skeleton,
  StatusBar,
  StatusDot,
  StatusPanel,
  Toolbar,
} from '@iii-dev/console-ui'
import type { WorkerLive } from '@iii-dev/console-ui/hooks'
import { AlertTriangle, Check, ChevronRight, Minus, Plus, RefreshCcw, SlidersHorizontal } from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import {
  badge,
  cycleInput,
  cycleTitle,
  FIX,
  fixFor,
  type Group,
  groupOf,
  isActive,
  nextSetup,
  nowView,
  PHASES,
  phaseOf,
  READY_LINE,
  readiness,
  rowLine,
  shortId,
} from '../model'
import type { CycleListResponse, CycleRecord, LoopConfig } from '../types'
import { ago, DOT, StatusChip, Strip, usd } from './bits'
import type { Nav } from './index'

const PHASE_HINT = [
  'Baseline on the E2E, then a first pass that calls no model',
  'One change inside harness/, in its own worktree',
  'Paired runs on a pinned stack, decided in code',
  'A pull request for an accepted change; you review and merge',
]

type Show = 'all' | Group

const GROUPS: { id: Group; label: string; mark: ReactNode }[] = [
  {
    id: 'you',
    label: 'Needs you',
    mark: <AlertTriangle size={16} className="eval-ui-mark" data-mark="stop" aria-hidden />,
  },
  { id: 'active', label: 'In progress', mark: <StatusDot tone="accent" /> },
  { id: 'done', label: 'Finished', mark: null },
]
const SHOW: { value: Show; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'active', label: 'Active' },
  { value: 'you', label: 'Needs you' },
  { value: 'done', label: 'Finished' },
]
/** The phone's shorter word, so four segments fit one line. */
const SHORT: Partial<Record<Show, string>> = { done: 'Done' }

export function Home({
  list,
  config,
  revision,
  narrow,
  nav,
  onClose,
}: {
  list: WorkerLive<CycleListResponse>
  config: LoopConfig | null
  revision?: string
  narrow: boolean
  nav: Nav
  onClose?: () => void
}) {
  const [query, setQuery] = useState('')
  const [show, setShow] = useState<Show>('all')
  const cycles = list.data?.cycles
  const usage = list.data?.usage ?? {}
  const level = readiness(config)
  const next = nextSetup(config)

  const grouped = useMemo(() => {
    const needle = query.trim().toLowerCase()
    const out: Record<Group, CycleRecord[]> = { you: [], active: [], done: [] }
    for (const cycle of cycles ?? []) {
      const input = 'objective' in cycle.input ? cycle.input.objective : cycle.input.execution_id
      if (needle && ![cycleTitle(cycle), input, cycle.cycle_id].some((field) => field.toLowerCase().includes(needle))) {
        continue
      }
      out[groupOf(cycle, config)].push(cycle)
    }
    return out
  }, [cycles, query, config])

  const count = (group: Show) =>
    group === 'all' ? grouped.you.length + grouped.active.length + grouped.done.length : grouped[group].length
  const spent = Object.values(usage).reduce((sum, entry) => sum + entry.spend_usd, 0)
  const filter = (
    <SegmentedControl<Show>
      aria-label="Show"
      variant="radio"
      value={show}
      onChange={setShow}
      className="eval-ui-seg"
      options={SHOW.map(({ value, label }) => ({
        value,
        icon: false,
        label: (
          <>
            {narrow ? (SHORT[value] ?? label) : label}{' '}
            {narrow ? null : <span className="eval-ui-count">{count(value)}</span>}
          </>
        ),
      }))}
    />
  )

  return (
    <>
      <PageHeader
        icon={<RefreshCcw size={16} aria-hidden />}
        title="Eval loop"
        description={narrow ? undefined : 'Measures, improves and proves Harness changes on the E2E'}
        actions={
          <>
            <IconButton label="Loop settings" onClick={() => nav.settings()}>
              <SlidersHorizontal size={16} aria-hidden />
            </IconButton>
            <Button variant="primary" size={narrow ? 'lg' : 'md'} onClick={nav.create}>
              <Plus size={16} aria-hidden />
              {narrow ? 'New' : 'New cycle'}
            </Button>
          </>
        }
        onClose={onClose}
      />
      <PageBody>
        <PageMain className="eval-ui-main">
          <section className="eval-ui-ready" aria-label="What a new cycle can do">
            <div className="eval-ui-ready-head">
              <Eyebrow as="h2">{narrow ? 'A new cycle can' : 'What a new cycle can do'}</Eyebrow>
              {narrow ? null : <p className="eval-ui-faint">{READY_LINE[level]}</p>}
            </div>
            <ol
              className="eval-ui-ladder"
              aria-label={PHASES.map((phase, at) => `${phase} ${at < level ? 'on' : 'off'}`).join(', ')}
            >
              {PHASES.map((phase, at) => (
                <li key={phase} className="eval-ui-lane" data-on={at < level ? 'true' : undefined}>
                  {at < level ? (
                    <Check size={16} className="eval-ui-mark" data-mark="done" aria-hidden />
                  ) : (
                    <Minus size={16} className="eval-ui-mark" data-mark="todo" aria-hidden />
                  )}
                  <span className="eval-ui-lane-text">
                    <span className="eval-ui-strong">
                      {phase}
                      {at < level || narrow ? null : <span className="eval-ui-faint"> · off</span>}
                    </span>
                    {narrow ? null : <span className="eval-ui-faint">{PHASE_HINT[at]}</span>}
                  </span>
                </li>
              ))}
              {next && !narrow ? (
                <li className="eval-ui-ladder-action">
                  <Button variant="pill" onClick={() => nav.settings(next)}>
                    {FIX[next].action}
                  </Button>
                </li>
              ) : null}
            </ol>
          </section>

          {/* A phone gets the filter alone, full width: the segments are 48px there. */}
          {narrow ? (
            <div className="eval-ui-filter">{filter}</div>
          ) : (
            <Toolbar aria-label="Filter cycles">
              <SearchField
                aria-label="Filter cycles"
                placeholder="Filter by objective, execution or id"
                value={query}
                onChange={setQuery}
                className="eval-ui-search"
              />
              {filter}
            </Toolbar>
          )}

          <div className="eval-ui-scroll">
            {list.error && !cycles ? (
              <StatusPanel
                variant="alert"
                headline="Couldn't read the cycles"
                detail={list.error}
                action={
                  <Button variant="ghost" size="sm" onClick={list.refresh}>
                    Retry
                  </Button>
                }
              />
            ) : !cycles ? (
              <div className="eval-ui-skeleton" role="status" aria-busy="true" aria-label="Loading cycles">
                <Skeleton className="eval-ui-sk-row" />
                <Skeleton className="eval-ui-sk-row" />
                <Skeleton className="eval-ui-sk-row" />
              </div>
            ) : count('all') === 0 ? (
              <div className="eval-ui-groups">
                <EmptyState
                  title={cycles.length ? 'No cycle matches' : 'No cycles yet'}
                  description={
                    cycles.length
                      ? 'Clear the filter to see every cycle.'
                      : 'Start one from a finished E2E execution that lost points, or from an objective.'
                  }
                  action={
                    cycles.length
                      ? { label: 'Clear filter', onClick: () => setQuery('') }
                      : { label: 'New cycle', onClick: nav.create }
                  }
                />
              </div>
            ) : (
              <List className="eval-ui-groups" aria-label="Cycles">
                {GROUPS.filter((group) => (show === 'all' || show === group.id) && grouped[group.id].length).map(
                  (group) => (
                    <ListGroup key={group.id}>
                      <ListGroupLabel className="eval-ui-group">
                        {group.mark}
                        {group.label} <span className="eval-ui-count">{grouped[group.id].length}</span>
                      </ListGroupLabel>
                      {grouped[group.id].map((cycle) => (
                        <CycleRow
                          key={cycle.cycle_id}
                          cycle={cycle}
                          spend={usage[cycle.cycle_id]?.spend_usd ?? 0}
                          current={config}
                          narrow={narrow}
                          onOpen={() => nav.cycle(cycle.cycle_id)}
                        />
                      ))}
                    </ListGroup>
                  ),
                )}
              </List>
            )}
          </div>

          <StatusBar
            aria-label="Loop status"
            end={<span className="eval-ui-mono">{revision ? `policy ${revision.slice(0, 8)}` : 'not configured'}</span>}
          >
            <span className="eval-ui-live">
              <StatusDot tone={list.live ? 'ok' : 'ink'} /> {list.live ? 'Live' : 'Refreshes on focus'}
            </span>
            <span>{cycles?.length ?? 0} cycles</span>
            <span>{usd(spent)} spent across them</span>
          </StatusBar>
        </PageMain>
      </PageBody>
    </>
  )
}

function CycleRow({
  cycle,
  spend,
  current,
  narrow,
  onOpen,
}: {
  cycle: CycleRecord
  spend: number
  current: LoopConfig | null
  narrow: boolean
  onOpen: () => void
}) {
  const view = badge(cycle)
  const fix = fixFor(cycle, current)
  const input = cycleInput(cycle)
  const short = isActive(cycle.status)
    ? `${PHASES[phaseOf(cycle)]} · ${nowView(cycle).headline}`
    : `${view.label}${fix ? ` · ${FIX[fix].why}` : ''}`
  return (
    <ListItem
      onClick={onOpen}
      leading={<StatusDot tone={DOT[view.tone]} pulse={isActive(cycle.status)} />}
      label={<span className="eval-ui-clip eval-ui-strong">{cycleTitle(cycle)}</span>}
      description={
        narrow ? (
          <span className="eval-ui-row-line">
            <Strip record={cycle} />
            <span className="eval-ui-clip">{short}</span>
          </span>
        ) : (
          <span className="eval-ui-clip">
            {input.kind === 'execution' ? (
              <>
                From <span className="eval-ui-mono">{shortId(input.value)}</span> ·{' '}
              </>
            ) : null}
            {rowLine(cycle, current)}
          </span>
        )
      }
      trailing={
        narrow ? (
          <ChevronRight size={16} className="eval-ui-ghost" aria-hidden />
        ) : (
          <span className="eval-ui-row-end">
            <Strip record={cycle} />
            <StatusChip tone={view.tone} label={view.label} />
            <span className="eval-ui-num">{usd(spend)}</span>
            <span className="eval-ui-num eval-ui-ghost">{ago(cycle.updated_at)}</span>
          </span>
        )
      }
    />
  )
}
