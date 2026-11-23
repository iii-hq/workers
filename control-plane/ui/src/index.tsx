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
  uiClasses,
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
  const data = useControlPlaneOverview(host, scope)
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
  // Spins while the click's reads are in flight and while a writer turn runs.
  const refreshing = Boolean(summary?.refreshing || summary?.writing)
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
          <IconButton label="Refresh Control plane" onClick={refresh} aria-busy={refreshing}>
            <RefreshCw className={`control-plane-size-4${refreshing ? ` ${uiClasses.spin}` : ''}`} />
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
                {tab.value === 'overview' ? null : (
                  <span className="control-plane-tab-count">
                    {
                      {
                        'needs-you': data.items.length,
                        assumptions: data.questions.filter((row) => row.recordedBy).length,
                        decided: data.questions.filter(
                          (row) => !row.recordedBy && !['open', 'expired', 'answering'].includes(row.state),
                        ).length,
                        runs: data.runs.length,
                      }[tab.value]
                    }
                  </span>
                )}
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
                configurationId={configurationId}
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
                : view === 'needs-you'
                  ? `${data.items.length} open${data.items.length ? ` · oldest ${formatRelative(Math.min(...data.items.map((item) => item.createdAt)))}` : ''} · ${data.runs.length} runs`
                  : 'Counts and states read live from harness, kanban and GitHub · lines marked agent are written by the control-plane agent'}
            </span>
          </StatusBar>
        </PageMain>
      </PageBody>
    </PageShell>
  )
}

function Section({
  title,
  subtitle,
  aside,
  children,
}: {
  title: string
  subtitle?: ReactNode
  aside?: ReactNode
  children: ReactNode
}) {
  return (
    <Card className="control-plane-section">
      <PanelHeader className="control-plane-section-title">
        <h2>{title}</h2>
        {subtitle ? <span className="control-plane-section-subtitle">{subtitle}</span> : null}
        <span className="control-plane-spacer" />
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
const MODEL_MISSING = 'set the summary model in the Control plane settings'
const PROFILE_MISSING = 'writer profile missing: apply the control plane kit'

function SummarySlot({
  checkedAt,
  refresh,
  diary = false,
  loading,
  error,
  configurationId,
}: {
  checkedAt?: number
  refresh: () => void
  diary?: boolean
  loading?: boolean
  error?: string | null
  configurationId?: string
}) {
  if (!diary && (error === MODEL_MISSING || error === PROFILE_MISSING))
    return (
      <StatusPanel
        className="control-plane-summary-panel"
        headline="No summary for this project"
        detail={
          error === MODEL_MISSING
            ? 'Choose the model that writes it in the Control plane settings. The Overview shows only live facts: sessions show their last message, pull requests a reason from GitHub.'
            : 'The control-plane writer profile is not installed: apply the control-plane kit. The Overview shows only live facts meanwhile.'
        }
        action={
          error === MODEL_MISSING && configurationId ? (
            <Button
              variant="pill"
              size="sm"
              onClick={() => {
                window.location.hash = `#/configuration/workers/${encodeURIComponent(configurationId)}`
              }}
            >
              Open settings
            </Button>
          ) : undefined
        }
      />
    )
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

/** The writer's prose with each reference it cites rendered as a link in place. */
function LinkedText({ text, refs, data }: { text: string; refs: string[]; data: ControlPlaneData }) {
  const cited = refs.filter((ref) => text.includes(ref)).sort((a, b) => b.length - a.length)
  if (!cited.length) return <>{text}</>
  const parts = text.split(new RegExp(`(${cited.map((ref) => ref.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})`))
  return (
    <>
      {parts.map((part, index) =>
        cited.includes(part) ? <SummaryRefs key={`${part}:${index}`} refs={[part]} data={data} /> : part,
      )}
    </>
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
function Recommended({ item }: { item: NeedsYouItem }) {
  if (item.kind !== 'question') return null
  const option = item.entry.questions[0]?.options.find((candidate) => candidate.recommended)
  const text = option?.label ?? item.why
  if (!text) return null
  return (
    <p className="control-plane-muted">
      {option ? <span className="control-plane-text-ink">Recommended:</span> : null} {text}
    </p>
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
  configurationId,
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
  configurationId?: string
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
      detail: 'a turn running',
      tone: 'accent' as const,
    },
    {
      label: 'Waiting',
      value:
        data.loading.sessions || data.errors.sessions || data.loading.needsYou || data.errors.needsYou ? '—' : waiting,
      detail: `${data.runs.filter((run) => run.state === 'waiting').length} on you, ${data.runs.filter((run) => run.state === 'stalled').length} stalled`,
      tone: 'ghost' as const,
    },
    {
      label: 'Open PRs',
      value: data.loading.prs || data.errors.prs ? '—' : data.prs.length,
      detail: `CI failing in ${data.prs.filter((pr) => pr.checks === 'failing').length}`,
      tone: 'ghost' as const,
    },
    {
      label: 'Active releases',
      value: data.loading.board || data.errors.board ? '—' : active.length,
      detail: active.map((release) => release.name.split(' — ')[0]).join(', ') || 'No active releases',
      tone: 'ghost' as const,
    },
  ]
  const noRush = data.items.length - nowItems.length
  // Done, now and next: the latest released, every active one, then the planned ones, at most eight.
  const releasedLast = data.releases.filter((release) => release.status === 'released').slice(-1)
  const orderedReleases = [
    ...releasedLast,
    ...data.releases.filter((release) => release.status === 'active'),
    ...data.releases.filter((release) => release.status === 'planned'),
  ]
  const shownReleases = orderedReleases.slice(0, 8)
  const hiddenReleases = data.releases.length - shownReleases.length
  return (
    <>
      {summaryError !== MODEL_MISSING && summaryError !== PROFILE_MISSING && summary?.brief.headline ? (
        <div className="control-plane-summary">
          <div className="control-plane-row-head">
            <Eyebrow as="h2">Summary</Eyebrow>
            <AgentSource checkedAt={summary.checked_at} />
          </div>
          <h3>{summary.brief.headline}</h3>
          <p className="control-plane-summary-text">
            <LinkedText text={summary.brief.text} refs={summary.brief.refs} data={data} />
          </p>
          <div className="control-plane-row-head">
            <SummaryRefs refs={summary.brief.refs.filter((ref) => !summary.brief.text.includes(ref))} data={data} />
          </div>
          {summaryError === 'update failed' ? (
            <p className="control-plane-meta">
              The last update failed; this is the last valid summary, {formatAge(summary.checked_at)}.
            </p>
          ) : null}
        </div>
      ) : (
        <>
          <Eyebrow as="h2">Summary</Eyebrow>
          <SummarySlot
            checkedAt={summaryCheckedAt}
            refresh={refreshSummary}
            loading={summaryLoading}
            error={summaryError}
            configurationId={configurationId}
          />
        </>
      )}
      <div className="control-plane-tiles">
        {tiles.map((tile) => (
          <div key={tile.label} className="control-plane-tile">
            <span className="control-plane-muted control-plane-inline-status">
              <StatusDot tone={tile.tone} /> {tile.label}
            </span>
            <strong>{tile.value}</strong>
            <span className="control-plane-muted">{tile.detail}</span>
          </div>
        ))}
      </div>
      {!data.loading.board &&
      !data.loading.sessions &&
      !data.loading.prs &&
      !data.errors.board &&
      !data.errors.sessions &&
      !data.errors.prs &&
      !data.tickets.length &&
      !data.runs.length &&
      !data.prs.length &&
      !data.items.length ? (
        <EmptyState
          title={`Nothing is happening in ${scopeLabel} yet`}
          description="Runs, pull requests and decisions show up here once a session works on one of its cards."
        />
      ) : null}
      <div className="control-plane-columns">
        <div className="control-plane-stack">
          <Section
            title="Needs you"
            aside={
              <>
                {nowItems.length ? (
                  <span className="control-plane-now-chip">
                    <StatusDot tone="warn" /> {nowItems.length} now
                  </span>
                ) : null}
                <button type="button" className="control-plane-link control-plane-header-link" onClick={() => openNeed()}>
                  All {data.items.length}
                </button>
              </>
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
            ) : nowItems.length ? (
              <div data-urgency="now">
              {nowItems.slice(0, 3).map((item) => (
                <div key={item.id} className="control-plane-row">
                  <div className="control-plane-row-head">
                    <NeedLabel item={item} />
                    {item.card ? <CardLink card={item.card} /> : null}
                    <time className="control-plane-meta" dateTime={new Date(item.createdAt).toISOString()}>
                      {formatRelative(item.createdAt)}
                    </time>
                  </div>
                  <p className="control-plane-need-title">{item.title}</p>
                  <Recommended item={item} />
                  <div className="control-plane-row-head">
                    <Button size="sm" variant="pill" onClick={() => openNeed(item)}>
                      Open
                    </Button>
                    <AskAboutButton
                      item={item}
                      compact
                      available={canAsk && item.kind !== 'stalled'}
                      onClick={() => openNeed(item, true)}
                    />
                  </div>
                </div>
              ))}
              </div>
            ) : data.errors.needsYou ? null : (
              <p className="control-plane-empty-line">Nothing needs you now. Work keeps going.</p>
            )}
            <button
              type="button"
              className="control-plane-fold-row"
              onClick={() => openNeed(data.items.find((item) => item.urgency === 'no-rush'))}
            >
              <span>No rush</span>
              <span className="control-plane-meta">{noRush}</span>
            </button>
          </Section>
          <Section
            title="Diary"
            subtitle="today's milestones"
            aside={summary ? <AgentSource checkedAt={summary.checked_at} /> : null}
          >
            <CardBody>
              {summary && diary.length ? (
                diary.map((entry) => (
                  <div key={`${entry.at}:${entry.text}`} className="control-plane-diary-line">
                    <time className="control-plane-meta" dateTime={entry.at}>
                      {new Date(entry.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
                    </time>
                    <p>
                      <LinkedText text={entry.text} refs={entry.refs} data={data} />
                    </p>
                    <SummaryRefs refs={entry.refs.filter((ref) => !entry.text.includes(ref))} data={data} />
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
        <div className="control-plane-stack">
          <Section title="Releases" subtitle="done, now and next, from the board">
            <CardBody>
              {data.errors.board ? (
                <SourceError title="Board could not be listed" error={data.errors.board} refresh={data.refresh} />
              ) : data.loading.board ? (
                <Skeleton className="control-plane-h-28 control-plane-w-full" />
              ) : data.releases.length ? (
                <div className="control-plane-releases">
                  {shownReleases.map((release) => {
                    const blocker = data.items.find(
                      (item) => item.card?.release_id === release.id && item.urgency === 'now',
                    )
                    const [name, ...about] = release.name.split(' — ')
                    return (
                      <div key={release.id} className="control-plane-release" data-status={release.status}>
                        <span className="control-plane-inline-status">
                          <StatusDot
                            tone={release.status === 'released' ? 'ok' : release.status === 'active' ? 'warn' : 'ghost'}
                          />
                          {release.status === 'released' ? 'Released' : release.status === 'active' ? 'Now' : 'Next'}
                        </span>
                        <button
                          type="button"
                          className="control-plane-release-name"
                          title={release.name}
                          onClick={() => host.panels?.open({ pageId: 'kanban-releases', context: { id: release.id } })}
                        >
                          {name}
                        </button>
                        <span className="control-plane-release-about" title={about.join(' — ')}>
                          {about.join(' — ') || '\u00a0'}
                        </span>
                        <div className="control-plane-release-progress">
                          <ProgressBar done={release.done} total={release.total} label={release.name} />
                          <span className="control-plane-meta">
                            {release.done}/{release.total}
                          </span>
                        </div>
                        {blocker ? (
                          <button
                            type="button"
                            className="control-plane-link control-plane-muted control-plane-text-left"
                            onClick={() => openNeed(blocker)}
                          >
                            Waits on {blocker.card?.key} · {release.open_questions || 1} needs you
                          </button>
                        ) : (
                          <span className="control-plane-muted">
                            {release.open_questions ? `${release.open_questions} need you` : 'Nothing waits on you'}
                          </span>
                        )}
                      </div>
                    )
                  })}
                  {hiddenReleases ? (
                    <button
                      type="button"
                      className="control-plane-link control-plane-muted control-plane-text-left"
                      onClick={() => host.panels?.open({ pageId: 'kanban-releases', context: {} })}
                    >
                      +{hiddenReleases} more · Open releases
                    </button>
                  ) : null}
                </div>
              ) : (
                <EmptyState compact title="No releases yet" description="The project's releases will appear here." />
              )}
            </CardBody>
          </Section>
          <Section
            title="In progress"
            subtitle="root sessions, on the board or not"
            aside={
              <button type="button" className="control-plane-link control-plane-header-link" onClick={openRuns}>
                All runs
              </button>
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
                </div>
              ))
            ) : (
              <p className="control-plane-empty-line">No runs in progress. Root sessions doing work appear here.</p>
            )}
          </Section>
          <PullRequests data={data} scope={scope} summary={summary} />
          <Board data={data} scope={scope} scopeLabel={scopeLabel} />
        </div>
      </div>
    </>
  )
}

function PullRequests({ data, scope, summary }: { data: ControlPlaneData; scope: string; summary: ControlPlaneSummary | null }) {
  const host = useControlPlaneHost()
  return (
    <Section
      title="Pull requests"
      subtitle="open, with why each one waits"
      aside={<span className="control-plane-count-chip">{data.errors.prs ? '—' : data.prs.length} open</span>}
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
  const done = data.tickets.filter((ticket) => ticket.status === data.board?.doneStatus).length
  const columns = data.board?.columns ?? []
  const isFolded = (id: string, index: number) => id === data.board?.doneStatus || index === 0
  const open = columns.filter((column, index) => !isFolded(column.id, index))
  const folded = columns.filter((column, index) => isFolded(column.id, index))
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
    <Section
      title="Board"
      subtitle={`the whole board · ${scopeLabel.toLowerCase()}`}
      aside={
        data.loading.board || data.errors.board ? null : (
          <span className="control-plane-board-progress">
            <span className="control-plane-meta">
              {done} of {data.tickets.length} done
            </span>
            <ProgressBar done={done} total={data.tickets.length} label="Board" />
          </span>
        )
      }
    >
      {data.errors.board ? (
        <SourceError title="Board could not be listed" error={data.errors.board} refresh={data.refresh} />
      ) : data.loading.board ? (
        <CardBody>
          <Skeleton className="control-plane-h-40 control-plane-w-full" />
        </CardBody>
      ) : (
        <div className="control-plane-board">
          {open.map((column) => (
            <div key={column.id} className="control-plane-board-column">
              <div className="control-plane-board-heading">
                <StatusDot tone={column.id === 'in_review' ? 'warn' : 'ink'} />
                {column.label}
                <span className="control-plane-meta control-plane-ml-auto">{column.total}</span>
              </div>
              {renderRows(column.tickets, column.id === 'todo' ? 2 : 5)}
            </div>
          ))}
          {folded.map((column) => (
            <details key={column.id} className="control-plane-board-column">
              <summary className="control-plane-board-heading">
                {column.label}
                <span className="control-plane-meta control-plane-ml-auto">{column.total}</span>
              </summary>
              {renderRows(column.tickets)}
            </details>
          ))}
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
          <Card key={run.session.session_id} className="control-plane-section control-plane-run">
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
                <button
                  type="button"
                  className="control-plane-link control-plane-header-link"
                  onClick={() => host.panels?.open({ pageId: 'kanban-ticket', context: { id: run.card?.key ?? '' } })}
                >
                  Open card
                </button>
              ) : null}
              <button
                type="button"
                className="control-plane-link control-plane-header-link"
                onClick={() => host.chat?.openConversation?.(run.session.session_id)}
              >
                Open session
              </button>
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
                <div
                  className="control-plane-attention"
                  data-tone={run.needsYou[0].kind === 'stalled' ? 'alert' : undefined}
                >
                  <StatusDot tone={run.needsYou[0].kind === 'stalled' ? 'alert' : 'warn'} />
                  <span className="control-plane-attention-text">
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
