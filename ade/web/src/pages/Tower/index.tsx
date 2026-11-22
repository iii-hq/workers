import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import { RadioTower, RefreshCw } from 'lucide-react'
import { type ReactNode, useEffect, useMemo, useState } from 'react'
import { decisionChatCapabilities } from '@/components/chat/decision/host'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Eyebrow } from '@/components/ui/Eyebrow'
import { IconButton } from '@/components/ui/IconButton'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import {
  PageBody,
  PageHeader,
  PageMain,
  PageShell,
} from '@/components/ui/PageChrome'
import { Select } from '@/components/ui/Select'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { Card, CardBody, CardHeader } from '@/components/ui/Surface'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { StatusBar } from '@/components/ui/Toolbar'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { requestPanelOpen, requestScreenOpen } from '@/lib/panel-context'
import { formatTokenCount } from '@/lib/token-estimate'
import { chatSessionScreen } from '@/lib/workspace-tabs'
import type { Host } from '@/types/injectable-ui'
import { type TowerData, useTowerData } from './data'
import {
  ALL_PROJECTS,
  type NeedsYouItem,
  needUrgency,
  type TowerPullRequest,
  type TowerRun,
  type TowerTicket,
} from './model'
import {
  AskAboutButton,
  NeedsYou,
  needLabel,
  QuestionLog,
  type TowerActions,
} from './NeedsYou'
import { resolveSummary, summaryIsFresh, type TowerSummary } from './summary'
import { useTowerSummary } from './use-summary'
import './tower.css'

type TowerView = 'overview' | 'needs-you' | 'assumptions' | 'decided' | 'runs'
const VIEWS: Array<{ value: TowerView; label: string }> = [
  { value: 'overview', label: 'Overview' },
  { value: 'needs-you', label: 'Needs you' },
  { value: 'assumptions', label: 'Assumptions' },
  { value: 'decided', label: 'Decided' },
  { value: 'runs', label: 'Runs' },
]

function openCard(card: TowerTicket) {
  requestPanelOpen({ pageId: 'kanban-ticket', context: { id: card.key } })
}
function openBoard(scope: string) {
  requestPanelOpen({
    pageId: 'kanban-board',
    context:
      scope === ALL_PROJECTS
        ? { layout: 'list' }
        : { project: scope, layout: 'list' },
  })
}
function openSession(id: string) {
  requestScreenOpen({ screen: chatSessionScreen(id), relativeTo: 'tower' })
}

export function Tower({ onRequestClose }: { onRequestClose?: () => void }) {
  const [client, setClient] = useState<IiiClient | null>(null)
  const [error, setError] = useState<unknown>(null)
  useEffect(() => {
    let active = true
    void getIiiClient().then(
      (iii) => {
        if (active) setClient(iii)
      },
      (err) => {
        if (active) setError(err)
      },
    )
    return () => {
      active = false
    }
  }, [])
  if (client)
    return <ConnectedTower iii={client} onRequestClose={onRequestClose} />
  return (
    <PageShell className="tower">
      <PageHeader
        icon={<RadioTower />}
        title="Control tower"
        onClose={onRequestClose}
      />
      <PageBody>
        <PageMain>
          <div className="tower-content">
            {error ? (
              <SourceError
                title="Control tower could not connect"
                error={error}
                refresh={() => {
                  setError(null)
                  void getIiiClient().then(setClient, setError)
                }}
              />
            ) : (
              <TowerLoading />
            )}
          </div>
        </PageMain>
      </PageBody>
    </PageShell>
  )
}

function ConnectedTower({
  iii,
  onRequestClose,
}: {
  iii: IiiClient
  onRequestClose?: () => void
}) {
  const [scope, setScope] = useState(ALL_PROJECTS)
  const data = useTowerData(iii, scope)
  const summary = useTowerSummary(iii, scope, data)
  const chat = useMemo(() => decisionChatCapabilities(iii, 'tower'), [iii])
  return (
    <TowerScreen
      data={data}
      summary={summary}
      iii={iii}
      chat={chat}
      scope={scope}
      onScopeChange={setScope}
      onRequestClose={onRequestClose}
    />
  )
}

export interface TowerScreenProps {
  data: TowerData
  scope: string
  onScopeChange: (scope: string) => void
  onRequestClose?: () => void
  initialView?: TowerView
  summaryCheckedAt?: number
  summary?: ReturnType<typeof useTowerSummary>
  iii?: TowerActions
  chat?: Pick<Host['chat'], 'openDecisionChat' | 'rebindDecisionChat'>
  user?: string
  initialSelectedId?: string
}

export function TowerScreen({
  data: sourceData,
  scope,
  onScopeChange,
  onRequestClose,
  initialView = 'overview',
  summaryCheckedAt: providedCheckedAt,
  summary,
  iii,
  chat,
  user = 'user',
  initialSelectedId,
}: TowerScreenProps) {
  const [view, setView] = useState<TowerView>(initialView)
  const [selectedId, setSelectedId] = useState<string | null>(
    initialSelectedId ?? null,
  )
  const [askAboutId, setAskAboutId] = useState<string | null>(null)
  const [leftRuns, setLeftRuns] = usePaneState<Record<string, string>>(
    `tower:left-runs:${user}`,
    {},
  )
  const stallVersion = (item: NeedsYouItem) => {
    const run = sourceData.runs.find(
      (row) => row.session.session_id === item.sessionId,
    )
    return JSON.stringify([
      item.createdAt,
      run?.session.updated_at,
      run?.status.turn_id,
      item.kind === 'stalled' ? item.armedWakes : null,
    ])
  }
  const items = sourceData.items.filter(
    (item) =>
      item.kind !== 'stalled' || leftRuns[item.id] !== stallVersion(item),
  )
  const data = {
    ...sourceData,
    items,
    runs: sourceData.runs.map((run) => ({
      ...run,
      needsYou: run.needsYou.filter((item) =>
        items.some((visible) => visible.id === item.id),
      ),
    })),
  }
  const nowItems = data.items.filter((item) => needUrgency(item) === 'now')
  const now = summary?.now ?? Date.now()
  const summaryCheckedAt = summary?.result
    ? Date.parse(summary.result.checked_at)
    : providedCheckedAt
  const freshSummary =
    summary?.result &&
    summaryIsFresh(Date.parse(summary.result.checked_at), now)
      ? resolveSummary(summary.result, data)
      : null
  const refresh = () => {
    data.refresh()
    summary?.refresh()
  }
  const openNeed = (item?: NeedsYouItem, askAbout = false) => {
    setSelectedId(item?.id ?? null)
    setAskAboutId(askAbout ? (item?.id ?? null) : null)
    setView('needs-you')
  }
  const scopeLabel =
    data.projects.find((project) => project.id === scope)?.name ??
    'All projects'
  const scopeControl = (
    <>
      <Select
        value={scope}
        onChange={onScopeChange}
        aria-label="Project scope"
        options={[
          { value: ALL_PROJECTS, label: 'All projects' },
          ...data.projects.map((project) => ({
            value: project.id,
            label: project.name,
          })),
        ]}
      />
      <span className="tower-freshness">
        <StatusDot
          tone={
            summaryCheckedAt !== undefined
              ? !summaryIsFresh(summaryCheckedAt, now)
                ? 'warn'
                : 'ok'
              : 'ghost'
          }
        />
        {summaryCheckedAt !== undefined
          ? `Summary ${Math.max(0, Math.floor((now - summaryCheckedAt) / 60_000))} min ago`
          : 'Summary absent'}
      </span>
    </>
  )
  return (
    <PageShell className="tower" aria-label="Control tower">
      <PageHeader
        icon={<RadioTower />}
        title="Control tower"
        onClose={onRequestClose}
        actions={
          <IconButton label="Refresh Control tower" onClick={refresh}>
            <RefreshCw className="size-4" />
          </IconButton>
        }
      >
        <span className="tower-header-description tower-muted">
          What the agents are building, and what waits on you
        </span>
        <div className="tower-scope tower-desktop-scope">{scopeControl}</div>
      </PageHeader>
      <div className="tower-scope tower-mobile-scope">{scopeControl}</div>
      <div className="tower-desktop-nav">
        <Tabs
          value={view}
          onValueChange={(value) => setView(value as TowerView)}
        >
          <TabsList aria-label="Control tower views">
            {VIEWS.map((tab) => (
              <TabsTrigger key={tab.value} value={tab.value} icon={false}>
                {tab.label}
                {tab.value === 'needs-you'
                  ? ` · ${data.items.length}`
                  : tab.value === 'runs'
                    ? ` · ${data.runs.length}`
                    : ''}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>
      <div className="tower-mobile-nav">
        <SegmentedControl
          aria-label="Control tower views"
          value={
            view === 'assumptions' || view === 'decided' ? 'needs-you' : view
          }
          onChange={setView}
          variant="radio"
          options={VIEWS.filter((tab) =>
            ['overview', 'needs-you', 'runs'].includes(tab.value),
          ).map((tab) => ({ ...tab, icon: false }))}
        />
      </div>
      <PageBody>
        <PageMain>
          <div
            className={`tower-content${view === 'needs-you' ? ' tower-inbox-content' : ''}`}
          >
            {view === 'overview' ? (
              <Overview
                data={data}
                scope={scope}
                scopeLabel={scopeLabel}
                nowItems={nowItems}
                openNeed={openNeed}
                openRuns={() => setView('runs')}
                summaryCheckedAt={summaryCheckedAt}
                summary={freshSummary}
                summaryLoading={summary?.loading}
                summaryError={summary?.error}
                refreshSummary={summary?.refresh ?? data.refresh}
                canAsk={!!chat?.openDecisionChat}
              />
            ) : view === 'runs' ? (
              <Runs
                data={data}
                scope={scope}
                openNeed={openNeed}
                summary={freshSummary}
              />
            ) : (
              <>
                <div className="tower-log-nav">
                  {VIEWS.filter((tab) =>
                    ['needs-you', 'assumptions', 'decided'].includes(tab.value),
                  ).map((tab) => (
                    <Button
                      key={tab.value}
                      size="sm"
                      variant="ghost"
                      onClick={() => setView(tab.value)}
                      aria-current={tab.value === view ? 'page' : undefined}
                    >
                      {tab.label}
                    </Button>
                  ))}
                </div>
                {view === 'needs-you' ? (
                  <NeedsYou
                    key={scope}
                    data={data}
                    iii={iii}
                    chat={chat}
                    user={user}
                    initialSelectedId={selectedId}
                    initialAskAboutId={askAboutId}
                    onLeave={(item) =>
                      setLeftRuns((current) => ({
                        ...current,
                        [item.id]: stallVersion(item),
                      }))
                    }
                    openRuns={() => setView('runs')}
                  />
                ) : (
                  <QuestionLog
                    key={`${scope}:${view}`}
                    data={data}
                    view={view}
                    iii={iii}
                    user={user}
                  />
                )}
              </>
            )}
          </div>
          <StatusBar
            end={
              <span className="inline-flex items-center gap-1.5">
                <StatusDot tone="ink" />
                live
              </span>
            }
          >
            <span className="truncate">
              {view === 'runs'
                ? `${data.runs.length} runs · ${data.runs.filter((run) => run.state === 'working').length} working · ${data.runs.filter((run) => run.state === 'stalled').length} stalled`
                : 'Counts and states from harness, kanban, approval-gate and GitHub'}
            </span>
          </StatusBar>
        </PageMain>
      </PageBody>
    </PageShell>
  )
}

function Section({
  title,
  aside,
  children,
}: {
  title: string
  aside?: ReactNode
  children: ReactNode
}) {
  return (
    <Card>
      <CardHeader className="tower-section-title">
        <h2>{title}</h2>
        {aside}
      </CardHeader>
      {children}
    </Card>
  )
}
function SourceError({
  title,
  error,
  refresh,
}: {
  title: string
  error: unknown
  refresh: () => void
}) {
  return (
    <StatusPanel
      variant="warn"
      role="status"
      headline={
        <span className="text-ink">
          <StatusDot tone="warn" /> {title}
        </span>
      }
      detail={errorMessage(error)}
      action={
        <Button variant="pill" size="sm" onClick={refresh}>
          Retry
        </Button>
      }
    />
  )
}
export function TowerLoading({ runs = false }: { runs?: boolean }) {
  return (
    <div
      role="status"
      aria-label="Loading Control tower"
      aria-busy="true"
      className="tower-stack"
    >
      <Skeleton className="h-5 w-64" />
      <Skeleton className="h-3 w-4/5" />
      <div className={runs ? 'tower-stack' : 'tower-tiles'}>
        {(runs ? [0, 1] : [0, 1, 2, 3, 4]).map((key) => (
          <Skeleton key={key} className="h-20 w-full" />
        ))}
      </div>
      <Skeleton className="h-40 w-full" />
      <Skeleton className="h-40 w-full" />
    </div>
  )
}
function SummarySlot({
  checkedAt,
  refresh,
  diary = false,
  loading,
  error,
}: {
  checkedAt?: number
  refresh: () => void
  diary?: boolean
  loading?: boolean
  error?: string | null
}) {
  return (
    <div className="flex items-center gap-3 flex-wrap">
      <span className="tower-muted" role="status" title={error ?? undefined}>
        {checkedAt !== undefined && !summaryIsFresh(checkedAt)
          ? 'The summary is out of date. Counts and states below stay live.'
          : loading
            ? 'Writing the summary. Live facts are shown below.'
            : error
              ? 'The summary is unavailable. Live facts are shown below.'
              : diary
                ? 'No diary for this project yet.'
                : 'No summary for this project. Live facts are shown below.'}
      </span>
      <Button variant="ghost" size="sm" onClick={refresh}>
        Refresh
      </Button>
    </div>
  )
}

function AgentSource({ checkedAt }: { checkedAt: string }) {
  return <span className="tower-meta">agent · {formatRelative(checkedAt)}</span>
}

function SummaryRefs({ refs, data }: { refs: string[]; data: TowerData }) {
  return (
    <span className="tower-summary-refs">
      {refs.map((ref) => {
        const card = data.tickets.find((ticket) => ticket.key === ref)
        if (card) return <CardLink key={ref} card={card} />
        if (ref.startsWith('session:'))
          return (
            <button
              key={ref}
              type="button"
              className="tower-link tower-meta"
              onClick={() => openSession(ref.slice(8))}
            >
              {ref}
            </button>
          )
        const pr = data.prs.find((pr) => `${pr.repo}#${pr.number}` === ref)
        return pr ? (
          <a
            key={ref}
            href={pr.url}
            target="_blank"
            rel="noreferrer"
            className="tower-link tower-meta"
          >
            {ref}
          </a>
        ) : null
      })}
    </span>
  )
}

function Doing({
  run,
  summary,
}: {
  run: TowerRun
  summary: TowerSummary | null
}) {
  const note = summary?.now[`session:${run.session.session_id}`]
  return (
    <span className="tower-prose-line">
      <span className="tower-one-line min-w-0">
        {note?.text || run.lastAssistant || 'No assistant message yet.'}
      </span>
      {note && summary ? (
        <AgentSource checkedAt={summary.checked_at} />
      ) : (
        <span className="tower-meta">from the transcript</span>
      )}
    </span>
  )
}
function NeedLabel({ item }: { item: NeedsYouItem }) {
  return <span className="tower-status">{needLabel(item)}</span>
}
const RUN_LABEL = {
  working: 'Working',
  waiting: 'Waiting on you',
  stalled: 'Stalled',
  done: 'Done',
  failed: 'Failed',
} as const
function runLabel(run: TowerRun) {
  return run.state === 'waiting' &&
    run.status.expects_wake &&
    !run.status.open_asks &&
    !run.needsYou.length
    ? 'Waiting for wake'
    : RUN_LABEL[run.state]
}
function RunState({ run }: { run: TowerRun }) {
  return (
    <span className="tower-status">
      <StatusDot
        tone={
          run.state === 'working'
            ? 'accent'
            : run.state === 'stalled' || run.state === 'failed'
              ? 'alert'
              : run.state === 'waiting'
                ? 'warn'
                : 'ok'
        }
      />
      {runLabel(run)}
    </span>
  )
}
function PrState({ pr }: { pr: TowerPullRequest }) {
  return (
    <span className="tower-status">
      <StatusDot
        tone={
          pr.checks === 'none'
            ? 'ghost'
            : pr.checks === 'passing'
              ? 'ok'
              : pr.checks === 'failing'
                ? 'alert'
                : 'warn'
        }
      />
      {pr.checks === 'none' ? 'No checks' : `CI ${pr.checks}`}
    </span>
  )
}
function CardLink({ card }: { card: TowerTicket }) {
  return (
    <button
      type="button"
      className="tower-link tower-meta"
      onClick={() => openCard(card)}
    >
      {card.key}
    </button>
  )
}
function ProgressBar({
  done,
  inProgress = 0,
  total,
  label,
}: {
  done: number
  inProgress?: number
  total: number
  label: string
}) {
  return (
    <div
      className="tower-bar"
      role="progressbar"
      aria-label={`${label}: ${done} of ${total} done, ${inProgress} in progress`}
      aria-valuemin={0}
      aria-valuemax={total || 1}
      aria-valuenow={done}
    >
      <span
        className="tower-bar-done"
        style={{ width: `${total ? (done / total) * 100 : 0}%` }}
      />
      <span
        className="tower-bar-progress"
        style={{ width: `${total ? (inProgress / total) * 100 : 0}%` }}
      />
    </div>
  )
}

function Overview({
  data,
  scope,
  scopeLabel,
  nowItems,
  openNeed,
  openRuns,
  summaryCheckedAt,
  summary,
  summaryLoading,
  summaryError,
  refreshSummary,
  canAsk,
}: {
  data: TowerData
  scope: string
  scopeLabel: string
  nowItems: NeedsYouItem[]
  openNeed: (item?: NeedsYouItem, askAbout?: boolean) => void
  openRuns: () => void
  summaryCheckedAt?: number
  summary: TowerSummary | null
  summaryLoading?: boolean
  summaryError?: string | null
  refreshSummary: () => void
  canAsk: boolean
}) {
  const waiting = data.runs.filter(
    (run) =>
      run.state === 'stalled' ||
      (run.state === 'waiting' && runLabel(run) !== 'Waiting for wake'),
  ).length
  const active = data.releases.filter((release) => release.status === 'active')
  const diary =
    summary?.diary
      .filter(
        (entry) =>
          new Date(entry.at).toDateString() === new Date().toDateString(),
      )
      .sort((a, b) => Date.parse(b.at) - Date.parse(a.at)) ?? []
  const tiles = [
    {
      label: 'Needs you',
      value:
        data.loading.needsYou || data.errors.needsYou ? '—' : data.items.length,
      detail: `${nowItems.length} now, ${data.items.length - nowItems.length} no rush`,
      tone: 'warn' as const,
    },
    {
      label: 'Working',
      value:
        data.loading.sessions || data.errors.sessions
          ? '—'
          : data.runs.filter((run) => run.state === 'working').length,
      detail: 'a turn running in the tree',
      tone: 'accent' as const,
    },
    {
      label: 'Waiting',
      value:
        data.loading.sessions ||
        data.errors.sessions ||
        data.loading.needsYou ||
        data.errors.needsYou
          ? '—'
          : waiting,
      detail: 'on an answer, held call or stall',
      tone: 'ghost' as const,
    },
    {
      label: 'Open PRs',
      value: data.loading.prs || data.errors.prs ? '—' : data.prs.length,
      detail: `${data.prs.filter((pr) => pr.checks === 'failing').length} with CI failing`,
      tone: 'ghost' as const,
    },
    {
      label: 'Active releases',
      value: data.loading.board || data.errors.board ? '—' : active.length,
      detail:
        active.map((release) => release.name).join(', ') ||
        'No active releases',
      tone: 'ghost' as const,
    },
  ]
  return (
    <>
      <Eyebrow as="h2">Summary</Eyebrow>
      {summary?.brief.headline ? (
        <div className="tower-summary">
          <h3>{summary.brief.headline}</h3>
          <p className="tower-muted">{summary.brief.text}</p>
          <div className="tower-row-head">
            <SummaryRefs refs={summary.brief.refs} data={data} />
            <AgentSource checkedAt={summary.checked_at} />
          </div>
        </div>
      ) : (
        <SummarySlot
          checkedAt={summaryCheckedAt}
          refresh={refreshSummary}
          loading={summaryLoading}
          error={summaryError}
        />
      )}
      <div className="tower-tiles">
        {tiles.map((tile) => (
          <div key={tile.label} className="tower-tile">
            <span className="inline-flex items-center gap-1.5 tower-muted">
              <StatusDot tone={tile.tone} />
              {tile.label}
            </span>
            <strong>{tile.value}</strong>
            <span className="tower-muted">{tile.detail}</span>
          </div>
        ))}
      </div>
      {!data.loading.board &&
      !data.loading.sessions &&
      !data.loading.prs &&
      !data.errors.board &&
      !data.errors.sessions &&
      !data.errors.prs &&
      data.tickets.length === 0 &&
      data.runs.length === 0 &&
      data.prs.length === 0 &&
      data.items.length === 0 ? (
        <div>
          <EmptyState
            title={`Nothing is happening in ${scopeLabel} yet`}
            description="Runs, pull requests and decisions show up here once a session works on one of its cards."
          />
          <div className="flex justify-center mb-4">
            <Button variant="ghost" size="sm" onClick={() => openBoard(scope)}>
              Open the board
            </Button>
          </div>
        </div>
      ) : null}
      <div className="tower-columns">
        <div className="tower-stack">
          <Section
            title="Needs you"
            aside={
              <Button variant="ghost" size="sm" onClick={() => openNeed()}>
                All {data.items.length}
              </Button>
            }
          >
            {data.errors.needsYou ? (
              <SourceError
                title="Some Needs you sources are unavailable"
                error={data.errors.needsYou}
                refresh={data.refresh}
              />
            ) : null}
            {data.loading.needsYou && !data.items.length ? (
              <CardBody>
                <Skeleton className="h-32 w-full" />
              </CardBody>
            ) : nowItems.length ? (
              nowItems.slice(0, 3).map((item) => (
                <div key={item.id} className="tower-row">
                  <div className="tower-row-head">
                    <NeedLabel item={item} />
                    {item.card ? <CardLink card={item.card} /> : null}
                    <time
                      className="tower-meta"
                      dateTime={new Date(item.createdAt).toISOString()}
                    >
                      {formatRelative(item.createdAt)}
                    </time>
                  </div>
                  <p className="my-2 font-medium">{item.title}</p>
                  {item.kind === 'question' && item.why ? (
                    <p className="tower-muted mb-2">{item.why}</p>
                  ) : null}
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      variant="pill"
                      onClick={() => openNeed(item)}
                    >
                      Open
                    </Button>
                    <AskAboutButton
                      item={item}
                      available={canAsk}
                      onClick={() => openNeed(item, true)}
                    />
                  </div>
                </div>
              ))
            ) : data.errors.needsYou ? null : (
              <CardBody>
                <EmptyState
                  compact
                  title="Nothing needs you now"
                  description="Work can keep going. No urgent questions, held calls or stalled runs."
                />
              </CardBody>
            )}
            <Button
              variant="ghost"
              size="sm"
              className="m-2"
              onClick={() =>
                openNeed(
                  data.items.find((item) => needUrgency(item) === 'no-rush'),
                )
              }
            >
              No rush · {data.items.length - nowItems.length}
            </Button>
          </Section>
          <Section title="Diary">
            <CardBody>
              {summary && diary.length ? (
                diary.map((entry) => (
                  <div
                    key={`${entry.at}:${entry.text}`}
                    className="tower-diary-line"
                  >
                    <time className="tower-meta" dateTime={entry.at}>
                      {new Date(entry.at).toLocaleTimeString([], {
                        hour: '2-digit',
                        minute: '2-digit',
                      })}
                    </time>
                    <p>{entry.text}</p>
                    <SummaryRefs refs={entry.refs} data={data} />
                    <AgentSource checkedAt={summary.checked_at} />
                  </div>
                ))
              ) : (
                <SummarySlot
                  checkedAt={summaryCheckedAt}
                  refresh={refreshSummary}
                  diary
                  loading={summaryLoading}
                  error={summaryError}
                />
              )}
            </CardBody>
          </Section>
        </div>
        <div className="tower-stack">
          <Section
            title="Releases"
            aside={
              <span className="tower-muted">
                done, now and next, from the board
              </span>
            }
          >
            <CardBody>
              {data.errors.board ? (
                <SourceError
                  title="Board could not be listed"
                  error={data.errors.board}
                  refresh={data.refresh}
                />
              ) : data.loading.board ? (
                <Skeleton className="h-28 w-full" />
              ) : data.releases.length ? (
                <div className="tower-releases">
                  {data.releases.map((release) => {
                    const blocker = data.items.find(
                      (item) =>
                        item.card?.release_id === release.id &&
                        needUrgency(item) === 'now',
                    )
                    return (
                      <div key={release.id} className="tower-release">
                        <div className="tower-row-head">
                          <span className="inline-flex items-center gap-1.5">
                            <StatusDot
                              tone={
                                release.status === 'released'
                                  ? 'ok'
                                  : release.status === 'active'
                                    ? 'ink'
                                    : 'ghost'
                              }
                            />
                            {release.status === 'released'
                              ? 'Released'
                              : release.status === 'active'
                                ? 'Now'
                                : 'Next'}
                          </span>
                          <span className="tower-meta ml-auto">
                            {release.done}/{release.total}
                          </span>
                        </div>
                        <button
                          type="button"
                          className="text-left font-semibold"
                          onClick={() =>
                            requestPanelOpen({
                              pageId: 'kanban-releases',
                              context: { id: release.id },
                            })
                          }
                        >
                          {release.name}
                        </button>
                        <ProgressBar
                          done={release.done}
                          total={release.total}
                          label={release.name}
                        />
                        <span className="tower-muted">
                          {release.open_questions
                            ? `${release.open_questions} need you`
                            : 'Nothing waits on you'}
                          {release.target_at
                            ? ` · target ${release.target_at.slice(0, 10)}`
                            : ''}
                        </span>
                        {blocker ? (
                          <button
                            type="button"
                            className="tower-link tower-muted text-left"
                            onClick={() => openNeed(blocker)}
                          >
                            Waits on {blocker.card?.key}
                          </button>
                        ) : null}
                      </div>
                    )
                  })}
                </div>
              ) : (
                <EmptyState
                  compact
                  title="No releases yet"
                  description="The project's releases will appear here."
                />
              )}
            </CardBody>
          </Section>
          <Section
            title="In progress"
            aside={
              <Button variant="ghost" size="sm" onClick={openRuns}>
                All runs
              </Button>
            }
          >
            {data.errors.sessions ? (
              <SourceError
                title="Sessions could not be listed"
                error={data.errors.sessions}
                refresh={data.refresh}
              />
            ) : data.loading.sessions ? (
              <CardBody>
                <Skeleton className="h-32 w-full" />
              </CardBody>
            ) : data.runs.length ? (
              data.runs.map((run) => (
                <div key={run.session.session_id} className="tower-row">
                  <div className="tower-row-head">
                    <button
                      type="button"
                      className="tower-subject"
                      onClick={openRuns}
                    >
                      {run.session.title || run.session.session_id}
                    </button>
                    {run.card ? <CardLink card={run.card} /> : null}
                    <RunState run={run} />
                    <time className="tower-meta">
                      {formatRelative(run.session.updated_at)}
                    </time>
                  </div>
                  <p className="tower-muted mt-1">
                    <Doing run={run} summary={summary} />
                  </p>
                  {run.needsYou.length ? (
                    <button
                      type="button"
                      className="tower-link tower-muted"
                      onClick={() => openNeed(run.needsYou[0])}
                    >
                      {
                        run.needsYou.filter((item) => item.kind === 'question')
                          .length
                      }{' '}
                      questions open · {run.needsYou.length} open items
                    </button>
                  ) : null}
                </div>
              ))
            ) : (
              <CardBody>
                <EmptyState
                  compact
                  title="No runs in progress"
                  description="Root sessions doing work will appear here."
                />
              </CardBody>
            )}
          </Section>
          <PullRequests data={data} scope={scope} summary={summary} />
          <Board data={data} scope={scope} scopeLabel={scopeLabel} />
        </div>
      </div>
    </>
  )
}

function PullRequests({
  data,
  scope,
  summary,
}: {
  data: TowerData
  scope: string
  summary: TowerSummary | null
}) {
  return (
    <Section
      title="Pull requests"
      aside={
        <span className="tower-meta">
          {data.errors.prs ? '—' : data.prs.length} open
        </span>
      }
    >
      {data.errors.prs ? (
        <CardBody>
          <StatusPanel
            variant="warn"
            headline={
              <span className="text-ink">
                <StatusDot tone="warn" /> Pull requests are not listed
              </span>
            }
            detail="GitHub is not answering. Everything else on the Overview stays live."
            action={
              <Button
                variant="pill"
                size="sm"
                onClick={() => requestScreenOpen({ screen: 'workers' })}
              >
                Open Workers
              </Button>
            }
          />
        </CardBody>
      ) : data.loading.prs ? (
        <CardBody>
          <Skeleton className="h-32 w-full" />
        </CardBody>
      ) : !data.repositories.length ? (
        <CardBody>
          <EmptyState
            compact
            title="Add the repositories of this project"
            description="No repositories are configured and no session root has a GitHub origin remote."
            action={{
              label: 'Add repositories',
              onClick: () =>
                requestPanelOpen({
                  pageId: 'kanban-projects',
                  context: scope === ALL_PROJECTS ? {} : { id: scope },
                }),
            }}
          />
        </CardBody>
      ) : data.prs.length ? (
        data.prs.map((pr) => (
          <div key={`${pr.repo}#${pr.number}`} className="tower-row">
            <div className="tower-row-head">
              <a
                href={pr.url}
                target="_blank"
                rel="noreferrer"
                className="tower-link tower-meta"
              >
                {pr.repo}#{pr.number}
              </a>
              <strong>{pr.title}</strong>
              {pr.card ? <CardLink card={pr.card} /> : null}
              <PrState pr={pr} />
              <time className="tower-meta">{formatRelative(pr.createdAt)}</time>
            </div>
            <p className="tower-muted">
              {summary?.pr_waits[`${pr.repo}#${pr.number}`]?.text ??
                pr.waitReason ??
                (pr.checks === 'running'
                  ? 'Checks are still running'
                  : 'No pending wait reported')}{' '}
              {summary?.pr_waits[`${pr.repo}#${pr.number}`] ? (
                <AgentSource checkedAt={summary.checked_at} />
              ) : (
                <span className="tower-meta">from GitHub</span>
              )}
            </p>
          </div>
        ))
      ) : (
        <CardBody>
          <EmptyState
            compact
            title="No open pull requests"
            description="Pull requests from this scope's repositories appear here."
          />
        </CardBody>
      )}
    </Section>
  )
}

function Board({
  data,
  scope,
  scopeLabel,
}: {
  data: TowerData
  scope: string
  scopeLabel: string
}) {
  const renderRows = (tickets: TowerTicket[], limit = 5) => (
    <>
      {tickets.slice(0, limit).map((ticket) => (
        <button
          type="button"
          key={ticket.id}
          className="tower-ticket"
          onClick={() => openCard(ticket)}
        >
          <span className="tower-meta">{ticket.key}</span>
          <span>{ticket.title}</span>
          <span className="tower-meta">
            {[
              ...ticket.pills.map((pill) => pill.text),
              ...(ticket.open_questions
                ? [`${ticket.open_questions} need you`]
                : []),
            ].join(' · ')}
          </span>
        </button>
      ))}
      {tickets.length > limit ? (
        <button
          type="button"
          className="tower-link tower-muted ml-3 mb-2"
          onClick={() => openBoard(scope)}
        >
          +{tickets.length - limit} more · Open the board
        </button>
      ) : null}
    </>
  )
  return (
    <Section
      title="Board"
      aside={<span className="tower-muted">{scopeLabel}</span>}
    >
      {data.errors.board ? (
        <SourceError
          title="Board could not be listed"
          error={data.errors.board}
          refresh={data.refresh}
        />
      ) : data.loading.board ? (
        <CardBody>
          <Skeleton className="h-40 w-full" />
        </CardBody>
      ) : (
        <div className="tower-board">
          <div className="tower-board-heading">
            <span>
              {
                data.tickets.filter(
                  (ticket) => ticket.status === data.board?.doneStatus,
                ).length
              }{' '}
              of {data.tickets.length} done
            </span>
            <Button size="sm" variant="ghost" onClick={() => openBoard(scope)}>
              Open board
            </Button>
          </div>
          {data.board?.columns.map((column, index) =>
            column.id === data.board?.doneStatus || index === 0 ? (
              <details key={column.id}>
                <summary>
                  {column.label}{' '}
                  <span className="tower-meta">{column.total}</span>
                </summary>
                {renderRows(column.tickets)}
              </details>
            ) : (
              <div key={column.id}>
                <div className="tower-board-heading">
                  <StatusDot tone="ink" />
                  {column.label}
                  <span className="tower-meta">{column.total}</span>
                </div>
                {renderRows(column.tickets, column.id === 'todo' ? 2 : 5)}
              </div>
            ),
          )}
        </div>
      )}
    </Section>
  )
}

function Runs({
  data,
  scope,
  openNeed,
  summary,
}: {
  data: TowerData
  scope: string
  openNeed: (item?: NeedsYouItem) => void
  summary: TowerSummary | null
}) {
  if (data.loading.sessions || (scope !== ALL_PROJECTS && data.loading.board))
    return <TowerLoading runs />
  if (scope !== ALL_PROJECTS && data.errors.board)
    return (
      <SourceError
        title="Run card links could not be listed"
        error={data.errors.board}
        refresh={data.refresh}
      />
    )
  if (data.errors.sessions)
    return (
      <SourceError
        title="Sessions could not be listed"
        error={data.errors.sessions}
        refresh={data.refresh}
      />
    )
  if (!data.runs.length)
    return (
      <EmptyState
        title="No runs in this project yet"
        description="Root sessions with work, questions, held calls or armed wakes appear here, along with linked runs that ended in the last day."
        action={{ label: 'Open the board', onClick: () => openBoard(scope) }}
      />
    )
  return (
    <>
      <div className="tower-legend">
        Progress bars:
        <span>
          <i className="tower-legend-done" />
          done
        </span>
        <span>
          <i className="tower-legend-progress" />
          in progress
        </span>
        <span>
          <i className="tower-legend-rest" />
          not started
        </span>
      </div>
      <div className="tower-stack">
        {data.runs.map((run) => (
          <Card key={run.session.session_id} className="tower-run">
            <CardHeader className="tower-section-title">
              <h2 className="mr-auto">
                {run.session.title || run.session.session_id}
              </h2>
              {run.card ? (
                <CardLink card={run.card} />
              ) : (
                <span className="tower-meta">Not on the board</span>
              )}
              <RunState run={run} />
              {run.card?.pills.map((pill) => (
                <span
                  key={`${pill.icon}:${pill.text}`}
                  className="tower-status tower-meta"
                >
                  {pill.text}
                </span>
              ))}
              {run.card ? (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => openCard(run.card as TowerTicket)}
                >
                  Open card
                </Button>
              ) : null}
              <Button
                size="sm"
                variant="ghost"
                onClick={() => openSession(run.session.session_id)}
              >
                Open session
              </Button>
            </CardHeader>
            <CardBody>
              <div className="tower-run-grid">
                <div>
                  <p className="tower-meta">
                    {run.profile || 'default profile'} · {run.startedBy} ·
                    started {formatRelative(run.session.created_at)} ago
                    {run.workingDir ? ` · ${run.workingDir}` : ''}
                  </p>
                  {run.progress.map((progress) => (
                    <div
                      key={progress.releaseId ?? 'none'}
                      className="tower-progress-row"
                    >
                      <span>{progress.releaseName}</span>
                      <ProgressBar
                        done={progress.done}
                        inProgress={progress.inProgress}
                        total={progress.total}
                        label={`${run.session.title} · ${progress.releaseName}`}
                      />
                      <span className="tower-meta text-right">
                        {progress.done}/{progress.total}
                      </span>
                    </div>
                  ))}
                  {run.subtasks === null ? (
                    <StatusPanel
                      headline="Card progress unavailable"
                      detail="The card's subtasks could not be read. Stall detection resumes when progress is available."
                      action={
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={data.refresh}
                        >
                          Retry
                        </Button>
                      }
                    />
                  ) : !run.progress.length ? (
                    <p className="tower-muted mt-2">
                      {run.card
                        ? 'No subtasks on this card.'
                        : 'Not on the board'}
                    </p>
                  ) : null}
                  {data.prs
                    .filter((pr) => pr.card && pr.card.id === run.card?.id)
                    .map((pr) => (
                      <div
                        key={`${pr.repo}#${pr.number}`}
                        className="tower-row-head mt-3"
                      >
                        <a
                          className="tower-link tower-meta"
                          href={pr.url}
                          target="_blank"
                          rel="noreferrer"
                        >
                          {pr.repo}#{pr.number}
                        </a>
                        <span className="tower-muted">
                          {summary?.pr_waits[`${pr.repo}#${pr.number}`]?.text ??
                            pr.waitReason}
                        </span>
                        {summary?.pr_waits[`${pr.repo}#${pr.number}`] ? (
                          <AgentSource checkedAt={summary.checked_at} />
                        ) : (
                          <span className="tower-meta">from GitHub</span>
                        )}
                        <PrState pr={pr} />
                      </div>
                    ))}
                </div>
                <dl className="tower-facts">
                  <div>
                    <Eyebrow as="dt">Session</Eyebrow>
                    <dd>
                      {runLabel(run)}
                      {run.status.expects_wake
                        ? ' · wake armed'
                        : ' · nothing armed'}
                      {run.status.open_asks
                        ? ` · ${run.status.open_asks} questions open`
                        : ''}
                    </dd>
                  </div>
                  <div>
                    <Eyebrow as="dt">Last progress</Eyebrow>
                    <dd>{formatRelative(run.lastProgress)} ago</dd>
                  </div>
                  <div>
                    <Eyebrow as="dt">Sessions</Eyebrow>
                    <dd>
                      {run.metrics
                        ? `${run.metrics.complete ? '' : 'At least '}${run.metrics.totals.sessions} in this run`
                        : `${run.status.children.length + 1} in the root's active tree`}
                    </dd>
                  </div>
                </dl>
                <dl className="tower-facts">
                  <div>
                    <Eyebrow as="dt">Doing</Eyebrow>
                    <dd>
                      <Doing run={run} summary={summary} />
                    </dd>
                  </div>
                  <div>
                    <Eyebrow as="dt">Needs you</Eyebrow>
                    <dd>
                      {run.needsYou.length
                        ? `${run.needsYou.length} open`
                        : 'Nothing open'}
                    </dd>
                  </div>
                  <div>
                    <Eyebrow as="dt">Usage</Eyebrow>
                    <dd className="tower-meta">
                      {run.metrics
                        ? `${run.metrics.complete ? '' : 'At least '}${run.metrics.totals.input_tokens == null ? '—' : formatTokenCount(run.metrics.totals.input_tokens)} in · ${run.metrics.totals.output_tokens == null ? '—' : formatTokenCount(run.metrics.totals.output_tokens)} out`
                        : 'No usage reported'}
                    </dd>
                  </div>
                </dl>
              </div>
              {run.needsYou.length ? (
                <div className="tower-attention">
                  <StatusDot
                    tone={run.state === 'stalled' ? 'alert' : 'warn'}
                  />
                  <span>
                    {run.needsYou[0].title}
                    {run.needsYou.length > 1
                      ? ` · ${run.needsYou.length - 1} more wait on you`
                      : ''}
                  </span>
                  <Button
                    size="sm"
                    variant="pill"
                    onClick={() => openNeed(run.needsYou[0])}
                  >
                    Open in Needs you
                  </Button>
                </div>
              ) : null}
              {data.errors.board ? (
                <p className="tower-muted mt-3">
                  Board progress is unavailable.{' '}
                  <button
                    className="tower-link"
                    type="button"
                    onClick={() => openBoard(scope)}
                  >
                    Open board
                  </button>
                </p>
              ) : null}
            </CardBody>
          </Card>
        ))}
      </div>
    </>
  )
}
