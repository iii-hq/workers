import {
  Button,
  EmptyState,
  Eyebrow,
  IconButton,
  SegmentedControl,
  PageBody,
  PageHeader,
  PageMain,
  PageShell,
  Select,
  Skeleton,
  StatusDot,
  StatusPanel,
  Card,
  CardBody,
  PanelHeader,
  Tabs,
  TabsList,
  TabsTrigger,
  StatusBar,
  type Host,
  type PageRenderProps,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import { RadioTower, RefreshCw } from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import { useControlPlaneOverview, type ControlPlaneOverviewData as ControlPlaneData } from './useControlPlaneOverview'
import { ControlPlaneHost, useControlPlaneHost } from './host'
import {
  ALL_PROJECTS,
  type NeedsYouItem,
  type ControlPlanePullRequest,
  type ControlPlaneRun,
  type ControlPlaneTicket,
} from '../../src/types'
import { AskAboutButton, NeedsYou, needLabel, QuestionLog, stallVersion, type ControlPlaneActions } from './NeedsYou'
import { resolveSummary, summaryIsFresh, type ControlPlaneSummary } from '../../src/summary'
import { formatAge } from './format'

type ControlPlaneView = 'overview' | 'needs-you' | 'assumptions' | 'decided' | 'runs'
const VIEWS: Array<{ value: ControlPlaneView; label: string }> = [
  { value: 'overview', label: 'Overview' },
  { value: 'needs-you', label: 'Needs you' },
  { value: 'assumptions', label: 'Assumptions' },
  { value: 'decided', label: 'Decided' },
  { value: 'runs', label: 'Runs' },
]

export function ControlPlane({ host, paneId, tabId, onRequestClose, configurationId }: { host: Host; configurationId?: string } & PageRenderProps) {
  const [scope, setScope] = usePaneState(`control-plane:${paneId}:scope`, ALL_PROJECTS)
  return (
    <ScopedControlPlane
      key={scope}
      host={host}
      configurationId={configurationId}
      paneId={paneId}
      tabId={tabId}
      onRequestClose={onRequestClose}
      scope={scope}
      onScopeChange={setScope}
    />
  )
}

function ScopedControlPlane({
  host,
  configurationId,
  paneId,
  tabId,
  onRequestClose,
  scope,
  onScopeChange,
}: { host: Host; configurationId?: string; scope: string; onScopeChange: (scope: string) => void } & Pick<
  PageRenderProps,
  'paneId' | 'tabId' | 'onRequestClose'
>) {
  const [lastVisit, setLastVisit] = usePaneState<string | null>(`control-plane:last-visit:${scope}`, null)
  const [since] = useState(lastVisit ?? undefined)
  useEffect(() => {
    setLastVisit(new Date().toISOString())
  }, [setLastVisit])
  const data = useControlPlaneOverview(host, scope, since)
  return (
    <ControlPlaneScreen
      host={host}
      configurationId={configurationId}
      paneId={paneId}
      data={data}
      summary={data.summary}
      iii={host.iii}
      chat={tabId ? host.chat : undefined}
      scope={scope}
      onScopeChange={onScopeChange}
      onRequestClose={onRequestClose}
    />
  )
}

export interface ControlPlaneScreenProps {
  host: Host
  configurationId?: string
  paneId?: string
  data: ControlPlaneData
  scope: string
  onScopeChange: (scope: string) => void
  onRequestClose?: () => void
  initialView?: ControlPlaneView
  summaryCheckedAt?: number
  summary?: ControlPlaneData['summary']
  iii?: ControlPlaneActions
  chat?: Pick<NonNullable<Host['chat']>, 'openDecisionChat' | 'rebindDecisionChat'>
  user?: string
  initialSelectedId?: string
}

export function ControlPlaneScreen(props: ControlPlaneScreenProps) {
  return (
    <ControlPlaneHost.Provider value={props.host}>
      <ControlPlaneScreenContent {...props} />
    </ControlPlaneHost.Provider>
  )
}

function ControlPlaneScreenContent({
  paneId = 'control-plane',
  configurationId,
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
}: ControlPlaneScreenProps) {
  const { ref, narrow } = useContainerNarrow({ below: 640 })
  const [view, setView] = useState<ControlPlaneView>(initialView)
  const [selectedId, setSelectedId] = useState<string | null>(initialSelectedId ?? null)
  const [askAboutId, setAskAboutId] = useState<string | null>(null)
  const [leftRuns, setLeftRuns] = usePaneState<Record<string, string>>(`control-plane:${paneId}:left-runs:${user}`, {})
  const items = sourceData.items.filter(
    (item) => item.kind !== 'stalled' || leftRuns[item.id] !== stallVersion(item, sourceData.runs),
  )
  const data = {
    ...sourceData,
    items,
    runs: sourceData.runs.map((run) => ({
      ...run,
      needsYou: run.needsYou.filter((item) => items.some((visible) => visible.id === item.id)),
    })),
  }
  const nowItems = data.items.filter((item) => 'urgency' in item && item.urgency === 'now')
  const now = summary?.now ?? Date.now()
  const summaryCheckedAt = summary?.checked_at ? Date.parse(summary.checked_at) : providedCheckedAt
  const freshSummary =
    summary?.result &&
    ((summary.fresh && summaryCheckedAt !== undefined && summaryIsFresh(summaryCheckedAt, now)) ||
      summary.error === 'update failed')
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
  const scopeLabel = data.projects.find((project) => project.id === scope)?.name ?? 'All projects'
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
      <span className="control-plane-freshness">
        <StatusDot
          tone={
            summaryCheckedAt !== undefined
              ? !!summary?.error || !summaryIsFresh(summaryCheckedAt, now)
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
    <PageShell className="control-plane" aria-label="Control plane">
      <PageHeader
        icon={<RadioTower />}
        title="Control plane"
        onClose={onRequestClose}
        actions={
          <IconButton label="Refresh Control plane" onClick={refresh}>
            <RefreshCw className="control-plane-size-4" />
          </IconButton>
        }
      >
        <span className="control-plane-header-description control-plane-muted">
          What the agents are building, and what waits on you
        </span>
        <div className="control-plane-scope control-plane-desktop-scope">{scopeControl}</div>
      </PageHeader>
      <div className="control-plane-scope control-plane-mobile-scope">{scopeControl}</div>
      <div className="control-plane-desktop-nav">
        <Tabs value={view} onValueChange={(value) => setView(value as ControlPlaneView)}>
          <TabsList aria-label="Control plane views">
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
      <div className="control-plane-mobile-nav">
        <SegmentedControl
          aria-label="Control plane views"
          value={view === 'assumptions' || view === 'decided' ? 'needs-you' : view}
          onChange={setView}
          variant="radio"
          options={VIEWS.filter((tab) => ['overview', 'needs-you', 'runs'].includes(tab.value)).map((tab) => ({
            ...tab,
            icon: false,
          }))}
        />
      </div>
      {summary?.error ? (
        <StatusPanel
          className="control-plane-summary-banner"
          variant="warn"
          headline={configurationId && summary.error === 'set the summary model in the Control plane settings' ? (
            <a className="control-plane-link" href={`#/configuration/workers/${encodeURIComponent(configurationId)}`}>
              {summary.error}
            </a>
          ) : summary.error}
          detail={summary.result ? `Last valid summary ${formatAge(summary.result.checked_at)}` : undefined}
        />
      ) : null}
      {summary?.throttled_until ? (
        <StatusPanel headline={`Refresh available ${new Date(summary.throttled_until).toLocaleTimeString()}`} />
      ) : null}
      <PageBody>
        <PageMain>
          <div ref={ref} className={`control-plane-content${view === 'needs-you' ? ' control-plane-inbox-content' : ''}`}>
            {view !== 'overview' && view !== 'runs' ? (
              <div className="control-plane-log-nav">
                {VIEWS.filter((tab) => ['needs-you', 'assumptions', 'decided'].includes(tab.value)).map((tab) => (
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
            ) : null}
            <NeedsYou
              key={scope}
              visible={view === 'needs-you'}
              narrow={narrow}
              onFocusDecision={() => setView('needs-you')}
              data={data}
              iii={iii}
              chat={chat}
              user={user}
              paneId={paneId}
              initialSelectedId={selectedId}
              initialAskAboutId={askAboutId}
              onLeave={(item) =>
                setLeftRuns((current) => ({
                  ...current,
                  [item.id]: stallVersion(item, sourceData.runs),
                }))
              }
              openRuns={() => setView('runs')}
            />
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
              <Runs data={data} scope={scope} openNeed={openNeed} summary={freshSummary} />
            ) : (
              <>
                {view === 'needs-you' ? null : (
                  <QuestionLog key={`${scope}:${view}`} data={data} view={view} iii={iii} user={user} paneId={paneId} />
                )}
              </>
            )}
          </div>
          <StatusBar
            end={
              <span className="control-plane-inline-status">
                <StatusDot tone="ink" />
                live
              </span>
            }
          >
            <span className="control-plane-truncate">
              {view === 'runs'
                ? `${data.runs.length} runs · ${data.runs.filter((run) => run.state === 'working').length} working · ${data.runs.filter((run) => run.state === 'stalled').length} stalled`
                : 'Counts and states from harness, kanban and GitHub'}
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
  folded = false,
}: {
  title: string
  aside?: ReactNode
  children: ReactNode
  folded?: boolean
}) {
  if (folded)
    return (
      <details className="control-plane-folded">
        <summary>{title}</summary>
        <div className="control-plane-folded-body">{children}</div>
      </details>
    )
  return (
    <Card>
      <PanelHeader className="control-plane-section-title">
        <h2>{title}</h2>
        {aside}
      </PanelHeader>
      {children}
    </Card>
  )
}
function SourceError({ title, error, refresh }: { title: string; error: unknown; refresh: () => void }) {
  return (
    <StatusPanel
      variant="warn"
      role="status"
      headline={
        <span className="control-plane-text-ink">
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
export function ControlPlaneLoading({ runs = false }: { runs?: boolean }) {
  return (
    <div role="status" aria-label="Loading Control plane" aria-busy="true" className="control-plane-stack">
      <Skeleton className="control-plane-h-5 control-plane-w-64" />
      <Skeleton className="control-plane-h-3 control-plane-w-4-5" />
      <div className={runs ? 'control-plane-stack' : 'control-plane-tiles'}>
        {(runs ? [0, 1] : [0, 1, 2, 3, 4]).map((key) => (
          <Skeleton key={key} className="control-plane-h-20 control-plane-w-full" />
        ))}
      </div>
      <Skeleton className="control-plane-h-40 control-plane-w-full" />
      <Skeleton className="control-plane-h-40 control-plane-w-full" />
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
    <div className="control-plane-summary-state">
      <span className="control-plane-muted" role="status" title={error ?? undefined}>
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
  return <span className="control-plane-meta">agent · {formatRelative(checkedAt)}</span>
}

function SummaryRefs({ refs, data }: { refs: string[]; data: ControlPlaneData }) {
  const host = useControlPlaneHost()
  return (
    <span className="control-plane-summary-refs">
      {refs.map((ref) => {
        const card = data.tickets.find((ticket) => ticket.key === ref)
        if (card) return <CardLink key={ref} card={card} />
        if (ref.startsWith('session:'))
          return (
            <button
              key={ref}
              type="button"
              className="control-plane-link control-plane-meta"
              onClick={() => host.chat?.openConversation?.(ref.slice(8))}
            >
              {ref}
            </button>
          )
        const pr = data.prs.find((pr) => `${pr.repo}#${pr.number}` === ref)
        return pr ? (
          <a key={ref} href={pr.url} target="_blank" rel="noreferrer" className="control-plane-link control-plane-meta">
            {ref}
          </a>
        ) : null
      })}
    </span>
  )
}

function Doing({ run, summary }: { run: ControlPlaneRun; summary: ControlPlaneSummary | null }) {
  const note = summary?.now[`session:${run.session.session_id}`]
  return (
    <span className="control-plane-prose-line">
      <span className="control-plane-one-line control-plane-min-w-0">
        {note?.text || run.lastAssistant || 'No assistant message yet.'}
      </span>
      {note && summary ? (
        <AgentSource checkedAt={summary.checked_at} />
      ) : (
        <span className="control-plane-meta">from the transcript</span>
      )}
    </span>
  )
}
function NeedLabel({ item }: { item: NeedsYouItem }) {
  return <span className="control-plane-status">{needLabel(item)}</span>
}
const RUN_LABEL = {
  working: 'Working',
  waiting: 'Waiting on you',
  stalled: 'Stalled',
  done: 'Done',
  failed: 'Failed',
} as const
function runLabel(run: ControlPlaneRun) {
  return run.state === 'waiting' && run.status.expects_wake && !run.status.open_asks && !run.needsYou.length
    ? 'Waiting for wake'
    : RUN_LABEL[run.state]
}
function RunState({ run }: { run: ControlPlaneRun }) {
  return (
    <span className="control-plane-status">
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
function PrState({ pr }: { pr: ControlPlanePullRequest }) {
  return (
    <span className="control-plane-status">
      <StatusDot
        tone={
          pr.checks === 'none' ? 'ghost' : pr.checks === 'passing' ? 'ok' : pr.checks === 'failing' ? 'alert' : 'warn'
        }
      />
      {pr.checks === 'none' ? 'No checks' : `CI ${pr.checks}`}
    </span>
  )
}
function CardLink({ card }: { card: ControlPlaneTicket }) {
  const host = useControlPlaneHost()
  return (
    <button
      type="button"
      className="control-plane-link control-plane-meta"
      onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: card.key } })}
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
      className="control-plane-bar"
      role="progressbar"
      aria-label={`${label}: ${done} of ${total} done, ${inProgress} in progress`}
      aria-valuemin={0}
      aria-valuemax={total || 1}
      aria-valuenow={done}
    >
      <span className="control-plane-bar-done" style={{ width: `${total ? (done / total) * 100 : 0}%` }} />
      <span className="control-plane-bar-progress" style={{ width: `${total ? (inProgress / total) * 100 : 0}%` }} />
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
  data: ControlPlaneData
  scope: string
  scopeLabel: string
  nowItems: NeedsYouItem[]
  openNeed: (item?: NeedsYouItem, askAbout?: boolean) => void
  openRuns: () => void
  summaryCheckedAt?: number
  summary: ControlPlaneSummary | null
  summaryLoading?: boolean
  summaryError?: string | null
  refreshSummary: () => void
  canAsk: boolean
}) {
  const host = useControlPlaneHost()
  const waiting = data.runs.filter(
    (run) => run.state === 'stalled' || (run.state === 'waiting' && runLabel(run) !== 'Waiting for wake'),
  ).length
  const active = data.releases.filter((release) => release.status === 'active')
  const diary =
    summary?.diary
      .filter((entry) => new Date(entry.at).toDateString() === new Date().toDateString())
      .sort((a, b) => Date.parse(b.at) - Date.parse(a.at)) ?? []
  const tiles = [
    {
      label: 'Needs you',
      value: data.loading.needsYou || data.errors.needsYou ? '—' : data.items.length,
      detail: `${nowItems.length} now, ${data.items.length - nowItems.length} no rush`,
      tone: 'warn' as const,
    },
    {
      label: 'Working',
      value:
        data.loading.sessions || data.errors.sessions ? '—' : data.runs.filter((run) => run.state === 'working').length,
      detail: 'a turn running in the tree',
      tone: 'accent' as const,
    },
    {
      label: 'Waiting',
      value:
        data.loading.sessions || data.errors.sessions || data.loading.needsYou || data.errors.needsYou ? '—' : waiting,
      detail: 'on an answer or stall',
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
      detail: active.map((release) => release.name).join(', ') || 'No active releases',
      tone: 'ghost' as const,
    },
  ]
  return (
    <div className="control-plane-stack">
      <Section title="Direction">
        <CardBody>
          {data.errors.board ? (
            <SourceError title="Direction is unavailable" error={data.errors.board} refresh={data.refresh} />
          ) : data.loading.board ? (
            <Skeleton className="control-plane-h-20 control-plane-w-full" />
          ) : data.direction.length ? (
            <div className="control-plane-releases">
              {data.direction.map(({ release, state, reasons }) => (
                <div key={release.id} className="control-plane-release">
                  <div className="control-plane-row-head">
                    <StatusDot tone={state === 'at-risk' ? 'warn' : 'ok'} />
                    <span>{state === 'at-risk' ? 'At risk' : 'On track'}</span>
                    <time className="control-plane-meta">{release.target_at?.slice(0, 10)}</time>
                  </div>
                  <button
                    type="button"
                    className="control-plane-subject"
                    onClick={() => host.panels?.open({ pageId: 'kanban-releases', context: { id: release.id } })}
                  >
                    {release.name}
                  </button>
                  <ProgressBar done={release.done} total={release.total} label={release.name} />
                  {reasons.map((reason) => (
                    <p key={reason} className="control-plane-muted">
                      {
                        {
                          'past-target': 'Target date has passed',
                          'open-cards': 'Open cards near the target date',
                          'escalated-question': 'An escalated question is open',
                        }[reason]
                      }
                    </p>
                  ))}
                  {summary?.direction
                    .filter((line) => line.release_id === release.id)
                    .map((line) => (
                      <div key={line.text} className="control-plane-prose-line">
                        <p>{line.text}</p>
                        <SummaryRefs refs={line.refs} data={data} />
                        <AgentSource checkedAt={summary.checked_at} />
                      </div>
                    ))}
                </div>
              ))}
            </div>
          ) : (
            <EmptyState compact title="No dated releases" description="Releases with a target date appear here." />
          )}
          {summary?.brief.headline ? (
            <div className="control-plane-summary">
              <h3>{summary.brief.headline}</h3>
              <p className="control-plane-muted">{summary.brief.text}</p>
              <div className="control-plane-row-head">
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
        </CardBody>
      </Section>
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
            <Skeleton className="control-plane-h-32 control-plane-w-full" />
          </CardBody>
        ) : (
          <div className="control-plane-needs-groups">
            {(['now', 'no-rush'] as const).map((group) => (
              <div key={group} data-urgency={group}>
                <PanelHeader>
                  <h3>{group === 'now' ? 'Now' : 'No rush'}</h3>
                  <span className="control-plane-meta">{data.items.filter((item) => item.urgency === group).length}</span>
                </PanelHeader>
                {data.items
                  .filter((item) => item.urgency === group)
                  .slice(0, 3)
                  .map((item) => (
                    <div key={item.id} className="control-plane-row">
                      <div className="control-plane-row-head">
                        <NeedLabel item={item} />
                        {item.card ? <CardLink card={item.card} /> : null}
                        <time className="control-plane-meta">{formatRelative(item.createdAt)}</time>
                      </div>
                      <p className="control-plane-need-title">{item.title}</p>
                      {item.kind === 'question' && item.why ? <p className="control-plane-muted">{item.why}</p> : null}
                      <div className="control-plane-row-head">
                        <Button size="sm" variant="pill" onClick={() => openNeed(item)}>
                          Open
                        </Button>
                        <AskAboutButton
                          item={item}
                          tokens={
                            data.runs.find((run) => run.session.session_id === item.sessionId)?.context_size ?? null
                          }
                          available={canAsk && item.kind !== 'stalled'}
                          onClick={() => openNeed(item, true)}
                        />
                      </div>
                    </div>
                  ))}
                {!data.items.some((item) => item.urgency === group) ? (
                  <CardBody>
                    <p className="control-plane-muted">
                      {group === 'now' ? 'Nothing needs you now.' : 'No questions without urgency.'}
                    </p>
                  </CardBody>
                ) : null}
              </div>
            ))}
          </div>
        )}
      </Section>
      <Section title="Exceptions">
        <CardBody>
          {data.exceptions.stalled.map((run) => (
            <button
              key={run.session.session_id}
              type="button"
              className="control-plane-exception"
              onClick={() =>
                openNeed(
                  data.items.find((item) => item.kind === 'stalled' && item.sessionId === run.session.session_id),
                )
              }
            >
              <StatusDot tone="warn" />
              Stalled run · {run.session.title}
            </button>
          ))}
          {data.exceptions.failing_prs.map((pr) => (
            <a key={pr.url} href={pr.url} target="_blank" rel="noreferrer" className="control-plane-exception">
              <StatusDot tone="alert" />
              CI failing · {pr.repo}#{pr.number} · {pr.title}
            </a>
          ))}
          {data.exceptions.blocked_cards.map((card) => (
            <button
              key={card.id}
              type="button"
              className="control-plane-exception"
              onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: card.key } })}
            >
              <StatusDot tone="warn" />
              Blocked · {card.key} · {card.title}
            </button>
          ))}
          {data.exceptions.old_questions.map((question) => (
            <button
              key={question.key}
              type="button"
              className="control-plane-exception"
              onClick={() =>
                openNeed(
                  data.items.find(
                    (item) =>
                      item.kind === 'question' &&
                      item.questionId === question.questionId &&
                      item.sessionId === question.sessionId,
                  ),
                )
              }
            >
              <StatusDot tone="warn" />
              Open more than a day · {question.questions[0]?.question}
            </button>
          ))}
          {!Object.values(data.exceptions).some((rows) => rows.length) ? (
            <p className="control-plane-muted">No exceptions in this project.</p>
          ) : null}
        </CardBody>
      </Section>
      <Section title="Decided for you" aside={<span className="control-plane-meta">Since your last visit</span>}>
        <CardBody>
          {data.assumptionsSinceVisit.length ? (
            data.assumptionsSinceVisit.map((row) => (
              <div key={row.key} className="control-plane-log-row">
                <h3>{row.questions.map((question) => question.question).join(' · ')}</h3>
                <p>{row.answerText}</p>
                <div className="control-plane-row-head">
                  <span className="control-plane-meta">{row.recordedBy}</span>
                  <time className="control-plane-meta">{formatRelative(row.askedAt)}</time>
                  {row.ticketId ? (
                    <button
                      type="button"
                      className="control-plane-link control-plane-meta"
                      onClick={() =>
                        host.panels?.open({ pageId: 'kanban-ticket', context: { id: row.ticketId ?? '' } })
                      }
                    >
                      {row.ticketId}
                    </button>
                  ) : null}
                </div>
              </div>
            ))
          ) : (
            <p className="control-plane-muted">No new assumptions since your last visit.</p>
          )}
        </CardBody>
      </Section>
      <div className="control-plane-tiles">
        {tiles.map((tile) => (
          <div key={tile.label} className="control-plane-tile">
            <span className="control-plane-muted">
              <StatusDot tone={tile.tone} /> {tile.label}
            </span>
            <strong>{tile.value}</strong>
            <span className="control-plane-muted">{tile.detail}</span>
          </div>
        ))}
      </div>
      {!data.loading.board &&
      !data.loading.sessions &&
      !data.errors.board &&
      !data.errors.sessions &&
      !data.tickets.length &&
      !data.runs.length &&
      !data.items.length ? (
        <EmptyState
          title={`Nothing is happening in ${scopeLabel} yet`}
          description="Runs, pull requests and decisions show up here once a session works on one of its cards."
        />
      ) : null}
      <Section title="Releases" folded aside={<span className="control-plane-muted">done, now and next, from the board</span>}>
        <CardBody>
          {data.errors.board ? (
            <SourceError title="Board could not be listed" error={data.errors.board} refresh={data.refresh} />
          ) : data.loading.board ? (
            <Skeleton className="control-plane-h-28 control-plane-w-full" />
          ) : data.releases.length ? (
            <div className="control-plane-releases">
              {data.releases.map((release) => {
                const blocker = data.items.find(
                  (item) => item.card?.release_id === release.id && 'urgency' in item && item.urgency === 'now',
                )
                return (
                  <div key={release.id} className="control-plane-release">
                    <div className="control-plane-row-head">
                      <span className="control-plane-inline-status">
                        <StatusDot
                          tone={release.status === 'released' ? 'ok' : release.status === 'active' ? 'ink' : 'ghost'}
                        />
                        {release.status === 'released' ? 'Released' : release.status === 'active' ? 'Now' : 'Next'}
                      </span>
                      <span className="control-plane-meta control-plane-ml-auto">
                        {release.done}/{release.total}
                      </span>
                    </div>
                    <button
                      type="button"
                      className="control-plane-text-left control-plane-font-semibold"
                      onClick={() =>
                        host.panels?.open({
                          pageId: 'kanban-releases',
                          context: { id: release.id },
                        })
                      }
                    >
                      {release.name}
                    </button>
                    <ProgressBar done={release.done} total={release.total} label={release.name} />
                    <span className="control-plane-muted">
                      {release.open_questions ? `${release.open_questions} need you` : 'Nothing waits on you'}
                      {release.target_at ? ` · target ${release.target_at.slice(0, 10)}` : ''}
                    </span>
                    {blocker ? (
                      <button
                        type="button"
                        className="control-plane-link control-plane-muted control-plane-text-left"
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
            <EmptyState compact title="No releases yet" description="The project's releases will appear here." />
          )}
        </CardBody>
      </Section>
      <Section
        title="Runs in full"
        folded
        aside={
          <Button variant="ghost" size="sm" onClick={openRuns}>
            All runs
          </Button>
        }
      >
        {data.errors.sessions ? (
          <SourceError title="Sessions could not be listed" error={data.errors.sessions} refresh={data.refresh} />
        ) : data.loading.sessions ? (
          <CardBody>
            <Skeleton className="control-plane-h-32 control-plane-w-full" />
          </CardBody>
        ) : data.runs.length ? (
          data.runs.map((run) => (
            <div key={run.session.session_id} className="control-plane-row">
              <div className="control-plane-row-head">
                <button type="button" className="control-plane-subject" onClick={openRuns}>
                  {run.session.title || run.session.session_id}
                </button>
                {run.card ? <CardLink card={run.card} /> : null}
                <RunState run={run} />
                <time className="control-plane-meta">{formatRelative(run.session.updated_at)}</time>
              </div>
              <p className="control-plane-muted control-plane-mt-1">
                <Doing run={run} summary={summary} />
              </p>
              {run.needsYou.length ? (
                <button type="button" className="control-plane-link control-plane-muted" onClick={() => openNeed(run.needsYou[0])}>
                  {run.needsYou.filter((item) => item.kind === 'question').length} questions open ·{' '}
                  {run.needsYou.length} open items
                </button>
              ) : null}
            </div>
          ))
        ) : (
          <CardBody>
            <EmptyState compact title="No runs in progress" description="Root sessions doing work will appear here." />
          </CardBody>
        )}
      </Section>

      <PullRequests data={data} scope={scope} summary={summary} />
      <Board data={data} scope={scope} scopeLabel={scopeLabel} />
      <Section title="Diary" folded>
        <CardBody>
          {summary && diary.length ? (
            diary.map((entry) => (
              <div key={`${entry.at}:${entry.text}`} className="control-plane-diary-line">
                <time className="control-plane-meta" dateTime={entry.at}>
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
  )
}

function PullRequests({ data, scope, summary }: { data: ControlPlaneData; scope: string; summary: ControlPlaneSummary | null }) {
  const host = useControlPlaneHost()
  return (
    <Section
      title="Pull requests"
      folded
      aside={<span className="control-plane-meta">{data.errors.prs ? '—' : data.prs.length} open</span>}
    >
      {data.errors.prs || (!data.loading.prs && !data.availability.github) ? (
        <CardBody>
          <StatusPanel
            variant="warn"
            headline={
              <span className="control-plane-text-ink">
                <StatusDot tone="warn" /> Pull requests are not listed
              </span>
            }
            detail={
              data.availability.github
                ? 'GitHub is not answering. Everything else on the Overview stays live.'
                : 'The GitHub worker is unavailable. Everything else on the Overview stays live.'
            }
            action={
              <Button variant="pill" size="sm" onClick={() => host.panels?.openScreen?.({ screen: 'workers' })}>
                Open Workers
              </Button>
            }
          />
        </CardBody>
      ) : data.loading.prs ? (
        <CardBody>
          <Skeleton className="control-plane-h-32 control-plane-w-full" />
        </CardBody>
      ) : !data.repositories.length ? (
        <CardBody>
          <EmptyState
            compact
            title="Add the repositories of this project"
            description="Configure repositories on the project to list its pull requests."
            action={{
              label: 'Add repositories',
              onClick: () =>
                host.panels?.open({
                  pageId: 'kanban-projects',
                  context: scope === ALL_PROJECTS ? {} : { id: scope },
                }),
            }}
          />
        </CardBody>
      ) : data.prs.length ? (
        data.prs.map((pr) => (
          <div key={`${pr.repo}#${pr.number}`} className="control-plane-row">
            <div className="control-plane-row-head">
              <a href={pr.url} target="_blank" rel="noreferrer" className="control-plane-link control-plane-meta">
                {pr.repo}#{pr.number}
              </a>
              <strong>{pr.title}</strong>
              {pr.card ? <CardLink card={pr.card} /> : null}
              <PrState pr={pr} />
              <time className="control-plane-meta">{formatRelative(pr.createdAt)}</time>
              {Date.now() - Date.parse(pr.as_of) > data.prs_interval_min * 60_000 ? (
                <time className="control-plane-meta" dateTime={pr.as_of}>
                  Checked {formatAge(pr.as_of)}
                </time>
              ) : null}
            </div>
            <p className="control-plane-muted">
              {summary?.pr_waits[`${pr.repo}#${pr.number}`]?.text ??
                pr.waitReason ??
                (pr.checks === 'running' ? 'Checks are still running' : 'No pending wait reported')}{' '}
              {summary?.pr_waits[`${pr.repo}#${pr.number}`] ? (
                <AgentSource checkedAt={summary.checked_at} />
              ) : (
                <span className="control-plane-meta">from GitHub</span>
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

function Board({ data, scope, scopeLabel }: { data: ControlPlaneData; scope: string; scopeLabel: string }) {
  const host = useControlPlaneHost()
  const renderRows = (tickets: ControlPlaneTicket[], limit = 5) => (
    <>
      {tickets.slice(0, limit).map((ticket) => (
        <button
          type="button"
          key={ticket.id}
          className="control-plane-ticket"
          onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: ticket.key } })}
        >
          <span className="control-plane-meta">{ticket.key}</span>
          <span>{ticket.title}</span>
          <span className="control-plane-meta">
            {[
              ...ticket.pills.map((pill) => pill.text),
              ...(ticket.open_questions ? [`${ticket.open_questions} need you`] : []),
            ].join(' · ')}
          </span>
        </button>
      ))}
      {tickets.length > limit ? (
        <button
          type="button"
          className="control-plane-link control-plane-muted control-plane-ml-3 control-plane-mb-2"
          onClick={() =>
            host.panels?.open({
              pageId: 'kanban-board',
              context: scope === ALL_PROJECTS ? { layout: 'list' } : { project: scope, layout: 'list' },
            })
          }
        >
          +{tickets.length - limit} more · Open the board
        </button>
      ) : null}
    </>
  )
  return (
    <Section title="Board" folded aside={<span className="control-plane-muted">{scopeLabel}</span>}>
      {data.errors.board ? (
        <SourceError title="Board could not be listed" error={data.errors.board} refresh={data.refresh} />
      ) : data.loading.board ? (
        <CardBody>
          <Skeleton className="control-plane-h-40 control-plane-w-full" />
        </CardBody>
      ) : (
        <div className="control-plane-board">
          <div className="control-plane-board-heading">
            <span>
              {data.tickets.filter((ticket) => ticket.status === data.board?.doneStatus).length} of{' '}
              {data.tickets.length} done
            </span>
            <Button
              size="sm"
              variant="ghost"
              onClick={() =>
                host.panels?.open({
                  pageId: 'kanban-board',
                  context: scope === ALL_PROJECTS ? { layout: 'list' } : { project: scope, layout: 'list' },
                })
              }
            >
              Open board
            </Button>
          </div>
          {data.board?.columns.map((column, index) =>
            column.id === data.board?.doneStatus || index === 0 ? (
              <details key={column.id}>
                <summary>
                  {column.label} <span className="control-plane-meta">{column.total}</span>
                </summary>
                {renderRows(column.tickets)}
              </details>
            ) : (
              <div key={column.id}>
                <div className="control-plane-board-heading">
                  <StatusDot tone="ink" />
                  {column.label}
                  <span className="control-plane-meta">{column.total}</span>
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
  data: ControlPlaneData
  scope: string
  openNeed: (item?: NeedsYouItem) => void
  summary: ControlPlaneSummary | null
}) {
  const host = useControlPlaneHost()
  if (data.loading.sessions || (scope !== ALL_PROJECTS && data.loading.board)) return <ControlPlaneLoading runs />
  if (scope !== ALL_PROJECTS && data.errors.board)
    return <SourceError title="Run card links could not be listed" error={data.errors.board} refresh={data.refresh} />
  if (data.errors.sessions)
    return <SourceError title="Sessions could not be listed" error={data.errors.sessions} refresh={data.refresh} />
  if (!data.runs.length)
    return (
      <EmptyState
        title="No runs in this project yet"
        description="Root sessions with work, questions or armed wakes appear here, along with linked runs that ended in the last day."
        action={{
          label: 'Open the board',
          onClick: () =>
            host.panels?.open({
              pageId: 'kanban-board',
              context: scope === ALL_PROJECTS ? { layout: 'list' } : { project: scope, layout: 'list' },
            }),
        }}
      />
    )
  return (
    <>
      <div className="control-plane-legend">
        Progress bars:
        <span>
          <i className="control-plane-legend-done" />
          done
        </span>
        <span>
          <i className="control-plane-legend-progress" />
          in progress
        </span>
        <span>
          <i className="control-plane-legend-rest" />
          not started
        </span>
      </div>
      <div className="control-plane-stack">
        {data.runs.map((run) => (
          <Card key={run.session.session_id} className="control-plane-run">
            <PanelHeader className="control-plane-section-title">
              <h2 className="control-plane-mr-auto">{run.session.title || run.session.session_id}</h2>
              {run.card ? <CardLink card={run.card} /> : <span className="control-plane-meta">Not on the board</span>}
              <RunState run={run} />
              {run.card?.pills.map((pill) => (
                <span key={`${pill.icon}:${pill.text}`} className="control-plane-status control-plane-meta">
                  {pill.text}
                </span>
              ))}
              {run.card ? (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: run.card?.key ?? '' } })}
                >
                  Open card
                </Button>
              ) : null}
              <Button size="sm" variant="ghost" onClick={() => host.chat?.openConversation?.(run.session.session_id)}>
                Open session
              </Button>
            </PanelHeader>
            <CardBody>
              <div className="control-plane-run-grid">
                <div>
                  <p className="control-plane-meta">
                    {run.profile || 'default profile'} · {run.startedBy} · started{' '}
                    {formatAge(run.session.created_at)}
                  </p>
                  {run.progress.map((progress) => (
                    <div key={progress.releaseId ?? 'none'} className="control-plane-progress-row">
                      <span>{progress.releaseName}</span>
                      <ProgressBar
                        done={progress.done}
                        inProgress={progress.inProgress}
                        total={progress.total}
                        label={`${run.session.title} · ${progress.releaseName}`}
                      />
                      <span className="control-plane-meta control-plane-text-right">
                        {progress.done}/{progress.total}
                      </span>
                    </div>
                  ))}
                  {run.subtasks === null ? (
                    <StatusPanel
                      headline="Card progress unavailable"
                      detail="The card's subtasks could not be read. Stall detection resumes when progress is available."
                      action={
                        <Button variant="ghost" size="sm" onClick={data.refresh}>
                          Retry
                        </Button>
                      }
                    />
                  ) : !run.progress.length ? (
                    <p className="control-plane-muted control-plane-mt-2">
                      {run.card ? 'No subtasks on this card.' : 'Not on the board'}
                    </p>
                  ) : null}
                  {data.prs
                    .filter((pr) => pr.card && pr.card.id === run.card?.id)
                    .map((pr) => (
                      <div key={`${pr.repo}#${pr.number}`} className="control-plane-row-head control-plane-mt-3">
                        <a className="control-plane-link control-plane-meta" href={pr.url} target="_blank" rel="noreferrer">
                          {pr.repo}#{pr.number}
                        </a>
                        <span className="control-plane-muted">
                          {summary?.pr_waits[`${pr.repo}#${pr.number}`]?.text ?? pr.waitReason}
                        </span>
                        {summary?.pr_waits[`${pr.repo}#${pr.number}`] ? (
                          <AgentSource checkedAt={summary.checked_at} />
                        ) : (
                          <span className="control-plane-meta">from GitHub</span>
                        )}
                        <PrState pr={pr} />
                      </div>
                    ))}
                </div>
                <dl className="control-plane-facts">
                  <div>
                    <Eyebrow as="dt">Session</Eyebrow>
                    <dd>
                      {runLabel(run)}
                      {run.status.expects_wake ? ' · wake armed' : ' · nothing armed'}
                      {run.status.open_asks ? ` · ${run.status.open_asks} questions open` : ''}
                    </dd>
                  </div>
                  <div>
                    <Eyebrow as="dt">Last progress</Eyebrow>
                    <dd>{formatAge(run.lastProgress)}</dd>
                  </div>
                  <div>
                    <Eyebrow as="dt">Context size</Eyebrow>
                    <dd>{run.context_size.toLocaleString()} tokens</dd>
                  </div>
                  <div>
                    <Eyebrow as="dt">Sessions</Eyebrow>
                    <dd>{`${run.status.children.length + 1} in the root's active tree`}</dd>
                  </div>
                </dl>
                <dl className="control-plane-facts">
                  <div>
                    <Eyebrow as="dt">Doing</Eyebrow>
                    <dd>
                      <Doing run={run} summary={summary} />
                    </dd>
                  </div>
                  <div>
                    <Eyebrow as="dt">Needs you</Eyebrow>
                    <dd>{run.needsYou.length ? `${run.needsYou.length} open` : 'Nothing open'}</dd>
                  </div>
                </dl>
              </div>
              {run.needsYou.length ? (
                <div className="control-plane-attention">
                  <StatusDot tone={run.state === 'stalled' ? 'alert' : 'warn'} />
                  <span>
                    {run.needsYou[0].title}
                    {run.needsYou.length > 1 ? ` · ${run.needsYou.length - 1} more wait on you` : ''}
                  </span>
                  <Button size="sm" variant="pill" onClick={() => openNeed(run.needsYou[0])}>
                    Open in Needs you
                  </Button>
                </div>
              ) : null}
              {data.errors.board ? (
                <p className="control-plane-muted control-plane-mt-3">
                  Board progress is unavailable.{' '}
                  <button
                    className="control-plane-link"
                    type="button"
                    onClick={() =>
                      host.panels?.open({
                        pageId: 'kanban-board',
                        context: scope === ALL_PROJECTS ? { layout: 'list' } : { project: scope, layout: 'list' },
                      })
                    }
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
