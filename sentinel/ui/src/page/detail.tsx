import {
  Button,
  Chip,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  EmptyState,
  ErrorBoundary,
  Skeleton,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import type { Host } from '@iii-dev/console-ui'
import {
  Ban,
  ChevronDown,
  CircleCheck,
  Folder,
  MessageSquare,
  RotateCcw,
  ScanSearch,
  Square,
  TriangleAlert,
} from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import type {
  Client,
  DiagnosisRecord,
  GroupDetail,
  GroupSummary,
  OccurrenceSummary,
  StatusResponse,
} from '../api'
import { DiagnosisTab } from './diagnosis'
import { EvidenceView } from './evidence'
import { InvestigateWith } from './InvestigateWith'
import { Dot, StatusBadge } from './marks'
import { OccurrencesTable } from './occurrences'
import {
  ago,
  availableActions,
  ignoreNote,
  regressedNote,
  spaced,
  stamp,
  statusLook,
  versionRange,
} from './present.js'
import { GroupTimeline } from './timeline'

interface Props {
  api: Client
  host: Host
  groupId: string
  conversationId: string | null
  narrow: boolean
  now: number
  repositories: StatusResponse['repositories']
  onBack: () => void
  onChanged: () => void
  onLoaded: (group: GroupSummary) => void
  announce: (text: string) => void
}

type Tab = 'latest' | 'occurrences' | 'diagnosis' | 'history'

export function GroupDetailView({
  api,
  host,
  groupId,
  conversationId,
  narrow,
  now,
  repositories,
  onBack,
  onChanged,
  onLoaded,
  announce,
}: Props) {
  const [detail, setDetail] = useState<GroupDetail | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)
  const [picking, setPicking] = useState(false)
  const [tab, setTab] = useState<Tab>('latest')
  // The occurrence the first tab shows; the latest one unless a row of the
  // occurrences table was opened.
  const [viewing, setViewing] = useState<OccurrenceSummary | null>(null)
  const diagnoses = useDiagnoses(api, groupId, detail?.diagnosis?.id)

  const load = useCallback(() => {
    api
      .group(groupId)
      .then((value) => {
        setDetail(value)
        setError(null)
        onLoaded(value.group)
      })
      .catch((cause) => setError(errorMessage(cause)))
  }, [api, groupId])

  useEffect(load, [load])
  useEffect(() => {
    setTab('latest')
    setViewing(null)
  }, [groupId])

  // Every outcome is announced; only the ones no state note will explain
  // (a stopped pass) stay on screen as well.
  const act = async (run: () => Promise<unknown>, message?: string, visible = false) => {
    setBusy(true)
    setNotice(null)
    try {
      await run()
      if (message) {
        announce(message)
        if (visible) setNotice(message)
      }
      load()
      onChanged()
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setBusy(false)
    }
  }

  if (error && !detail) {
    return (
      <div className="sentinel-ui-detail">
        <StatusPanel
          variant="alert"
          headline="Could not read this group"
          detail={error}
          action={
            <Button size="sm" onClick={onBack}>
              Back
            </Button>
          }
        />
      </div>
    )
  }
  if (!detail) {
    return (
      <div className="sentinel-ui-detail">
        <Skeleton />
      </div>
    )
  }

  const { group } = detail
  const actions = availableActions(group.status)
  const running = detail.active_investigation
  const session = running?.session_id ?? detail.latest_investigation?.session_id ?? null
  const sessionInView = session !== null && session === conversationId
  const repository = repositories.find((entry) => entry.workers.includes(group.service_name))
  const repositoryPath = repository?.exists ? repository.path : null
  const repositoryName = repository ? repository.path.split('/').filter(Boolean).pop() : null

  const openSession = () => {
    if (session) host.chat?.selectConversation?.(session)
  }
  const investigate = (mode: 'assisted' | 'chat', model?: string, provider?: string) =>
    act(async () => {
      setPicking(false)
      try {
        const outcome = await api.investigate(group.id, mode, model, provider)
        if (outcome.existing) setNotice('An investigation was already running; opening that one.')
        setTab('diagnosis')
        host.chat?.selectConversation?.(outcome.session_id)
      } catch (cause) {
        // No default model is the shipped configuration, not a failure: the
        // person picks one for this run instead of reading an error.
        if (!errorMessage(cause).includes('sentinel/no_model')) throw cause
        announce('No default model is configured. Pick one for this investigation.')
        setPicking(true)
      }
    })
  const askForDiagnosis = () => {
    if (!session) return
    host.chat?.selectConversation?.(session)
    host.chat?.compose?.({
      text: 'Record what you have so far with sentinel::diagnosis::record, even if the confidence is low.',
      submit: true,
    })
  }

  const investigateMenu = (primary: boolean) => (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant={primary ? 'primary' : 'pill'} disabled={busy}>
          {primary ? <ScanSearch size={16} /> : null}
          Investigate
          <ChevronDown size={16} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>Investigate</DropdownMenuLabel>
        <MenuItem
          label="Investigate"
          hint={repositoryName ? `${repositoryName}/` : 'trace only'}
          onSelect={() => investigate('assisted')}
        />
        <MenuItem label="Investigate with…" hint="pick a model" onSelect={() => setPicking(true)} />
        <MenuItem label="Open in chat" hint="no first pass" onSelect={() => investigate('chat')} />
      </DropdownMenuContent>
    </DropdownMenu>
  )
  const resolveMenu = (primary: boolean) => (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant={primary ? 'primary' : 'pill'} disabled={busy}>
          {primary ? <CircleCheck size={16} /> : null}
          Resolve
          <ChevronDown size={16} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>Resolve</DropdownMenuLabel>
        <MenuItem
          label="Resolve now"
          hint="reopens on any new occurrence"
          onSelect={() => act(() => api.resolve(group.id, false), 'Resolved. A new occurrence reopens it as a regression.')}
        />
        <MenuItem
          label="Resolve until version change"
          hint={[group.service_name, group.last_version].filter(Boolean).join(' ')}
          onSelect={() =>
            act(
              () => api.resolve(group.id, true),
              group.last_version
                ? `Resolved on ${group.last_version}. Occurrences on this version keep counting without reopening.`
                : 'Resolved until the version changes.',
            )
          }
        />
      </DropdownMenuContent>
    </DropdownMenu>
  )
  const ignoreMenu = (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="pill" disabled={busy}>
          Ignore
          <ChevronDown size={16} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>Ignore until…</DropdownMenuLabel>
        <MenuItem label="Forever" onSelect={() => act(() => api.ignore(group.id, { kind: 'forever' }), 'Ignored forever.')} />
        <MenuItem
          label="50 more occurrences"
          onSelect={() =>
            act(() => api.ignore(group.id, { kind: 'occurrences', count: 50 }), 'Ignored until 50 more occurrences.')
          }
        />
        <MenuItem
          label={`${group.service_name} version changes`}
          hint={group.last_version}
          onSelect={() =>
            act(() => api.ignore(group.id, { kind: 'version_change' }), `Ignored until ${group.service_name} changes version.`)
          }
        />
      </DropdownMenuContent>
    </DropdownMenu>
  )
  const sessionButton =
    session && !sessionInView ? (
      <Button size="sm" variant="pill" onClick={openSession}>
        <MessageSquare size={16} />
        {running ? <Dot tone="accent" pulse /> : null}
        {group.status === 'diagnosed' ? 'Continue in chat' : 'Open session'}
      </Button>
    ) : null

  return (
    <div className="sentinel-ui-detail" data-narrow={narrow}>
      <div className="sentinel-ui-dhead">
        <div className="sentinel-ui-dhead-title">
          <Dot tone={statusLook(group.status).dot} pulse={group.status === 'investigating'} />
          <div className="sentinel-ui-dhead-copy">
            <h2>{group.title}</h2>
            <div className="sentinel-ui-kicker">
              <span>{[group.service_name, group.function_id].filter(Boolean).join(' · ')}</span>
              <code>·</code>
              <code title={group.fingerprint}>fp {group.fingerprint.slice(0, 5)}…{group.fingerprint.slice(-4)}</code>
              <code>·</code>
              <code>source {group.source}</code>
            </div>
          </div>
        </div>
        <div className="sentinel-ui-dhead-tools">
          <div className="sentinel-ui-dhead-badges">
            <StatusBadge status={group.status} />
            {group.status === 'regressed' && group.regressed_at_ms ? (
              <span className="sentinel-ui-quiet-mono">{ago(group.regressed_at_ms, now)}</span>
            ) : null}
            <Chip>{spaced(group.occurrence_count)} occurrences</Chip>
            <Chip>
              {spaced(group.sessions_affected)} {group.sessions_affected === 1 ? 'session' : 'sessions'}
            </Chip>
            <span className="sentinel-ui-quiet-mono">{versionRange(group.first_version, group.last_version)}</span>
            {repository ? (
              <Chip tone={repository.exists ? 'neutral' : 'warning'} title={repository.path}>
                <Folder size={16} aria-hidden="true" />
                {repositoryName}/ · {repository.exists ? 'read-only' : 'missing on disk'}
              </Chip>
            ) : (
              <Chip tone="warning">
                <Folder size={16} aria-hidden="true" />
                no repository mapped
              </Chip>
            )}
          </div>
          <div className="sentinel-ui-dhead-actions" role="toolbar" aria-label="Group actions">
            {sessionInView ? (
              <span className="sentinel-ui-quiet-mono">
                <Dot tone="accent" pulse={Boolean(running)} /> session open beside
              </span>
            ) : null}
            {group.status === 'diagnosed' ? (
              <>
                {resolveMenu(true)}
                {sessionButton}
                {investigateMenu(false)}
                {ignoreMenu}
              </>
            ) : (
              <>
                {sessionButton}
                {actions.includes('stop') && running ? (
                  <Button
                    size="sm"
                    variant="pill"
                    disabled={busy}
                    onClick={() =>
                      act(
                        () => api.cancel(running.id),
                        'First pass stopped. The session stays — open it to continue by hand.',
                        true,
                      )
                    }
                  >
                    <Square size={16} />
                    Stop
                  </Button>
                ) : null}
                {actions.includes('reopen') ? (
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={busy}
                    onClick={() => act(() => api.reopen(group.id), 'Reopened.')}
                  >
                    <RotateCcw size={16} />
                    Reopen
                  </Button>
                ) : null}
                {actions.includes('unignore') ? (
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={busy}
                    onClick={() => act(() => api.unignore(group.id), 'Back in the open list.')}
                  >
                    <RotateCcw size={16} />
                    Unignore
                  </Button>
                ) : null}
                {actions.includes('investigate') ? investigateMenu(true) : null}
                {actions.includes('resolve') ? resolveMenu(false) : null}
                {actions.includes('ignore') ? ignoreMenu : null}
              </>
            )}
          </div>
        </div>
      </div>

      {notice ? (
        <StatusPanel
          variant="info"
          headline={notice}
          action={
            <Button size="sm" variant="ghost" onClick={() => setNotice(null)}>
              Dismiss
            </Button>
          }
        />
      ) : null}
      {error ? <StatusPanel variant="alert" headline="That did not go through" detail={error} /> : null}

      {group.status === 'regressed' ? (
        <StatusPanel
          variant="alert"
          icon={<TriangleAlert size={16} />}
          headline="Regressed."
          detail={regressedNote(group, now)}
        />
      ) : null}
      {group.status === 'ignored' ? (
        <StatusPanel variant="info" icon={<Ban size={16} />} headline={ignoreNote(group)} />
      ) : null}
      {group.status === 'resolved' ? (
        <StatusPanel
          variant="success"
          icon={<CircleCheck size={16} />}
          headline={`Resolved${group.resolved_at_ms ? ` ${ago(group.resolved_at_ms, now)}` : ''}.`}
          detail={
            group.resolve_until_version_change && group.resolved_version
              ? `Occurrences on ${group.resolved_version} keep counting; the first one on another version reopens it as a regression.`
              : 'A new occurrence reopens it as a regression.'
          }
        />
      ) : null}
      {!repository ? (
        <StatusPanel
          variant="info"
          icon={<Folder size={16} />}
          headline={`No repository is mapped for ${group.service_name}.`}
          detail={`An investigation reads the ${group.source === 'log' ? 'log window' : 'trace'} and nothing else. Grouping, evidence, regression and the whole list work the same — only the agent's code access is missing. Map it in the configuration to give the agent the source.`}
        />
      ) : null}

      <dl className={`${uiClasses.panel} sentinel-ui-facts`}>
        <Fact label="First seen" lead={ago(group.first_seen_ms, now)} rest={stamp(group.first_seen_ms)} />
        <Fact label="Last seen" lead={ago(group.last_seen_ms, now)} rest={stamp(group.last_seen_ms)} />
        <Fact
          label="Occurrences · sessions"
          lead={spaced(group.occurrence_count)}
          rest={
            group.sessions_affected > 0
              ? `${spaced(group.sessions_affected)} distinct ${group.sessions_affected === 1 ? 'session' : 'sessions'}`
              : 'no session context'
          }
        />
        <Fact
          label="Worker version"
          lead={versionRange(group.first_version, group.last_version)}
        />
      </dl>

      <InvestigateWith
        host={host}
        open={picking}
        onCancel={() => setPicking(false)}
        onInvestigate={(model, provider) => investigate('assisted', model, provider)}
      />

      <Tabs value={tab} onValueChange={(next) => setTab(next as Tab)}>
        <TabsList variant="line">
          <TabsTrigger value="latest" icon={false}>
            {viewing ? 'Occurrence' : 'Latest occurrence'}
          </TabsTrigger>
          <TabsTrigger value="occurrences" icon={false}>
            Occurrences<span className="sentinel-ui-tab-count">{spaced(group.occurrence_count)}</span>
          </TabsTrigger>
          <TabsTrigger value="diagnosis" icon={false}>
            Diagnosis
            {diagnoses.total > 0 ? <span className="sentinel-ui-tab-count">{diagnoses.total}</span> : null}
          </TabsTrigger>
          <TabsTrigger value="history" icon={false}>
            History
          </TabsTrigger>
        </TabsList>
        <TabsContent value="latest" className="sentinel-ui-tab">
          <ErrorBoundary>
            {viewing ? (
              <div className="sentinel-ui-viewing">
                <span>Showing the occurrence from {ago(viewing.at_ms, now)}.</span>
                <Button size="sm" variant="ghost" onClick={() => setViewing(null)}>
                  Back to the latest
                </Button>
              </div>
            ) : null}
            {(viewing ?? detail.latest_occurrence) ? (
              <EvidenceView
                api={api}
                host={host}
                now={now}
                occurrence={(viewing ?? detail.latest_occurrence) as OccurrenceSummary}
              />
            ) : (
              <EmptyState compact title="No occurrence yet" description="The group exists but has no recorded occurrence." />
            )}
          </ErrorBoundary>
        </TabsContent>
        <TabsContent value="occurrences" className="sentinel-ui-tab">
          <ErrorBoundary>
            <OccurrencesTable
              api={api}
              groupId={group.id}
              host={host}
              now={now}
              total={group.occurrence_count}
              onOpen={(occurrence) => {
                setViewing(occurrence.id === detail.latest_occurrence?.id ? null : occurrence)
                setTab('latest')
              }}
            />
          </ErrorBoundary>
        </TabsContent>
        <TabsContent value="diagnosis" className="sentinel-ui-tab">
          <ErrorBoundary>
            <DiagnosisTab
              group={group}
              host={host}
              now={now}
              records={diagnoses.records}
              investigation={running ?? detail.latest_investigation}
              running={Boolean(running)}
              sessionInView={sessionInView}
              onAsk={session ? askForDiagnosis : undefined}
              onOpenSession={session ? openSession : undefined}
              repositoryPath={repositoryPath}
            />
          </ErrorBoundary>
        </TabsContent>
        <TabsContent value="history" className="sentinel-ui-tab">
          <ErrorBoundary>
            <GroupTimeline api={api} group={group} now={now} />
          </ErrorBoundary>
        </TabsContent>
      </Tabs>
    </div>
  )
}

function Fact({ label, lead, rest }: { label: string; lead: string; rest?: string }) {
  return (
    <div>
      <dt className={uiClasses.eyebrow}>{label}</dt>
      <dd title={rest ? `${lead} · ${rest}` : lead}>
        <b>{lead}</b>
        {rest ? ` · ${rest}` : ''}
      </dd>
    </div>
  )
}

function MenuItem({ label, hint, onSelect }: { label: string; hint?: string; onSelect: () => void }) {
  return (
    <DropdownMenuItem onSelect={onSelect}>
      <span>{label}</span>
      {hint ? <span className="sentinel-ui-menu-hint">{hint}</span> : null}
    </DropdownMenuItem>
  )
}

/**
 * Every recording for this group, newest first. Re-read whenever the current
 * one changes, which is how a new recording appears without a reload.
 */
function useDiagnoses(api: Client, groupId: string, currentId: string | undefined) {
  const [state, setState] = useState<{ records: DiagnosisRecord[]; total: number }>({
    records: [],
    total: 0,
  })
  useEffect(() => {
    let live = true
    if (!currentId) {
      setState({ records: [], total: 0 })
      return
    }
    api
      .diagnoses(groupId)
      .then((response) => live && setState({ records: response.diagnoses, total: response.total }))
      .catch((cause) => {
        // The versions are context, not the answer: losing them must not
        // take the diagnosis down with them.
        if (live) {
          setState({ records: [], total: 0 })
          console.warn('sentinel: could not read the diagnoses', errorMessage(cause))
        }
      })
    return () => {
      live = false
    }
  }, [api, groupId, currentId])
  return state
}
