import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  EmptyState,
  Eyebrow,
  MetaRow,
  Skeleton,
  StatusDot,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import type { Host } from '@iii-dev/console-ui'
import { ArrowLeft, MessageSquare, Search, Square } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import type { Client, GroupDetail, Investigation, StatusResponse } from '../api'
import { DiagnosisCard } from './diagnosis'
import { EvidenceView } from './evidence'
import { OccurrencesTable } from './occurrences'
import {
  STATUS_PRESENTATION,
  availableActions,
  ignoreSummary,
  sessionAffordance,
} from './present.js'

interface Props {
  api: Client
  host: Host
  groupId: string
  conversationId: string | null
  narrow: boolean
  repositories: StatusResponse['repositories']
  onBack: () => void
  onChanged: () => void
}

export function GroupDetailView({
  api,
  host,
  groupId,
  conversationId,
  narrow,
  repositories,
  onBack,
  onChanged,
}: Props) {
  const [detail, setDetail] = useState<GroupDetail | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)

  const load = useCallback(() => {
    api
      .group(groupId)
      .then((value) => {
        setDetail(value)
        setError(null)
      })
      .catch((cause) => setError(errorMessage(cause)))
  }, [api, groupId])

  useEffect(load, [load])

  const act = async (run: () => Promise<unknown>, message?: string) => {
    setBusy(true)
    setNotice(null)
    try {
      await run()
      if (message) setNotice(message)
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
    )
  }
  if (!detail) return <Skeleton />

  const { group } = detail
  const presentation = STATUS_PRESENTATION[group.status]
  const actions = availableActions(group.status)
  const session =
    detail.active_investigation?.session_id ?? detail.latest_investigation?.session_id ?? null
  const affordance = sessionAffordance(session, conversationId)
  const repository = repositories.find((entry) => entry.workers.includes(group.service_name))

  const openSession = () => {
    if (session) host.chat?.selectConversation?.(session)
  }

  const investigate = (mode: 'assisted' | 'chat') =>
    act(async () => {
      const outcome = await api.investigate(group.id, mode)
      if (outcome.existing) setNotice('An investigation was already running; opening that one.')
      host.chat?.selectConversation?.(outcome.session_id)
    })

  return (
    <div className="sentinel-ui-detail" data-narrow={narrow}>
      <div className="sentinel-ui-detail-head">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft size={16} />
          <span>All errors</span>
        </Button>
        <div className="sentinel-ui-detail-actions">
          {affordance ? (
            affordance.variant === 'live' ? (
              <span className="sentinel-ui-liveness">
                <StatusDot tone="ok" />
                <span>{affordance.label}</span>
              </span>
            ) : (
              <Button size="sm" variant="ghost" onClick={openSession}>
                <MessageSquare size={16} />
                <span>{affordance.label}</span>
              </Button>
            )
          ) : null}

          {actions.includes('investigate') ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="sm" disabled={busy}>
                  <Search size={16} />
                  <span>Investigate</span>
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent>
                <DropdownMenuItem onSelect={() => investigate('assisted')}>
                  Investigate now
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => investigate('chat')}>
                  Open in chat with the evidence
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}

          {actions.includes('stop') && detail.active_investigation ? (
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() =>
                act(
                  () => api.cancel(detail.active_investigation?.id ?? ''),
                  'The first pass was stopped.',
                )
              }
            >
              <Square size={16} />
              <span>Stop</span>
            </Button>
          ) : null}

          {actions.includes('resolve') ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="sm" variant="ghost" disabled={busy}>
                  Resolve
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent>
                <DropdownMenuItem onSelect={() => act(() => api.resolve(group.id, false))}>
                  Resolved
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => act(() => api.resolve(group.id, true))}>
                  Resolved — keep counting until the version changes
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
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
                  onSelect={() => act(() => api.ignore(group.id, { kind: 'forever' }))}
                >
                  Forever
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() =>
                    act(() => api.ignore(group.id, { kind: 'occurrences', count: 50 }))
                  }
                >
                  Until 50 more occurrences
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => act(() => api.ignore(group.id, { kind: 'version_change' }))}
                >
                  Until the worker version changes
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}

          {actions.includes('reopen') ? (
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => act(() => api.reopen(group.id))}>
              Reopen
            </Button>
          ) : null}
          {actions.includes('unignore') ? (
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => act(() => api.unignore(group.id))}>
              Stop ignoring
            </Button>
          ) : null}
        </div>
      </div>

      <Card>
        <CardHeader>
          <div className="sentinel-ui-detail-title">
            <Badge variant={presentation.tone === 'danger' ? 'alert' : 'default'}>
              {presentation.label}
            </Badge>
            <h2>{group.title}</h2>
          </div>
        </CardHeader>
        <CardBody>
          <MetaRow
            items={[
              { label: 'worker', value: group.service_name },
              { label: 'function', value: group.function_id ?? '—' },
              { label: 'namespace', value: group.namespace },
              { label: 'occurrences', value: String(group.occurrence_count) },
              { label: 'sessions', value: String(group.sessions_affected) },
              { label: 'first seen', value: formatRelative(group.first_seen_ms) },
              { label: 'last seen', value: formatRelative(group.last_seen_ms) },
              { label: 'versions', value: versionRange(group.first_version, group.last_version) },
              { label: 'fingerprint', value: group.fingerprint.slice(0, 12) },
            ]}
          >
            {group.status === 'ignored' ? <Chip tone="neutral">{ignoreSummary(group.ignore_rule)}</Chip> : null}
            {detail.trace_available ? null : <Chip tone="neutral">snapshot only</Chip>}
          </MetaRow>
          <pre className="sentinel-ui-message">{detail.message_sample}</pre>
        </CardBody>
      </Card>

      {notice ? <StatusPanel variant="info" headline={notice} /> : null}
      {error ? <StatusPanel variant="alert" headline="That did not go through" detail={error} /> : null}
      {detail.active_investigation ? (
        <InvestigationLine investigation={detail.active_investigation} onOpen={openSession} />
      ) : null}

      <Tabs defaultValue="evidence">
        <TabsList variant="line">
          <TabsTrigger value="evidence" icon={false}>
            Latest occurrence
          </TabsTrigger>
          <TabsTrigger value="occurrences" icon={false}>
            Occurrences
          </TabsTrigger>
          <TabsTrigger value="diagnosis" icon={false}>
            Diagnosis
          </TabsTrigger>
        </TabsList>
        <TabsContent value="evidence">
          {detail.latest_occurrence ? (
            <EvidenceView
              api={api}
              host={host}
              occurrence={detail.latest_occurrence}
              repositoryPath={repository?.exists ? repository.path : null}
            />
          ) : (
            <EmptyState
              compact
              title="No occurrence yet"
              description="The group exists but has no recorded occurrence."
            />
          )}
        </TabsContent>
        <TabsContent value="occurrences">
          <OccurrencesTable api={api} groupId={group.id} />
        </TabsContent>
        <TabsContent value="diagnosis">
          <DiagnosisCard
            api={api}
            diagnosis={detail.diagnosis}
            groupId={group.id}
            host={host}
            investigation={detail.active_investigation ?? detail.latest_investigation}
            onAsk={() => {
              if (!session) return
              host.chat?.selectConversation?.(session)
              host.chat?.compose?.({
                text: 'Record what you have so far with sentinel::diagnosis::record, even if the confidence is low.',
                submit: true,
              })
            }}
            repositoryPath={repository?.exists ? repository.path : null}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function InvestigationLine({
  investigation,
  onOpen,
}: {
  investigation: Investigation
  onOpen: () => void
}) {
  return (
    <div className="sentinel-ui-investigation">
      <StatusDot tone="accent" />
      <Eyebrow>first pass</Eyebrow>
      <span>
        {investigation.model} · started {formatRelative(investigation.created_ms)}
      </span>
      <Button size="sm" variant="ghost" onClick={onOpen}>
        Watch it
      </Button>
    </div>
  )
}

function versionRange(first?: string, last?: string): string {
  if (!first && !last) return 'unknown'
  if (!first || first === last) return last ?? first ?? 'unknown'
  return `${first} → ${last}`
}
