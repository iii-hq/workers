import {
  Button,
  EmptyState,
  PageHeader,
  PageMain,
  PageShell,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useContainerNarrow, useWorkerLive } from '@iii-dev/console-ui/hooks'
import type { Host, PageRenderProps } from '@iii-dev/console-ui'
import { ShieldAlert } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { client } from '../api'
import type { GroupStatus, StatusResponse } from '../api'
import { EVENT, PAGE_ID } from '../shared'
import { GroupDetailView } from './detail'
import { GroupsListView } from './list'
import { OPEN_STATES, sinceMs } from './present.js'

type Props = { host: Host } & PageRenderProps

/** What the list is narrowed to. Lives here so the detail view can go back
    to exactly the list the user was looking at. */
export interface Filters {
  statuses: GroupStatus[]
  window: string
  service: string
  search: string
}

const INITIAL_FILTERS: Filters = {
  statuses: OPEN_STATES as GroupStatus[],
  window: '24h',
  service: '',
  search: '',
}

export function SentinelPage({ host, panelSide, conversationId, panelContext, commands }: Props) {
  const api = useMemo(() => client(host.iii), [host.iii])
  const { ref, narrow } = useContainerNarrow({ below: 760 })
  const [filters, setFilters] = useState<Filters>(INITIAL_FILTERS)
  const [selected, setSelected] = useState<string | null>(null)

  // A sibling page can open this one on a group (`panels.open`).
  const contextGroup =
    panelContext?.context && typeof panelContext.context === 'object'
      ? (panelContext.context as { group_id?: string }).group_id
      : undefined
  useEffect(() => {
    if (contextGroup) setSelected(contextGroup)
  }, [contextGroup])

  const groups = useWorkerLive({
    iii: host.iii,
    handlerId: `iii::${PAGE_ID}-ui::events`,
    triggers: [EVENT.groupChanged, EVENT.investigationChanged],
    fetch: useCallback(
      () =>
        api.groups({
          status: filters.statuses,
          service_name: filters.service || undefined,
          since_ms: sinceMs(filters.window, Date.now()) ?? undefined,
          search: filters.search || undefined,
          limit: 50,
        }),
      [api, filters.statuses, filters.service, filters.window, filters.search],
    ),
  })

  const [status, setStatus] = useState<StatusResponse | null>(null)
  useEffect(() => {
    let live = true
    api
      .status()
      .then((value) => live && setStatus(value))
      .catch(() => live && setStatus(null))
    return () => {
      live = false
    }
  }, [api, groups.data])

  useEffect(() => {
    if (!commands) return
    return commands.register([
      {
        id: 'refresh',
        title: 'Refresh errors',
        shortcut: 'R',
        run: () => groups.refresh(),
      },
      {
        id: 'back',
        title: 'Back to the error list',
        shortcut: 'Escape',
        enabled: () => selected !== null,
        run: () => setSelected(null),
      },
    ])
  }, [commands, groups, selected])

  return (
    <PageShell ref={ref} data-narrow={narrow}>
      <PageHeader
        icon={<ShieldAlert size={16} />}
        title="errors"
        description={describe(status)}
        actions={
          <Button variant="ghost" size="sm" onClick={() => groups.refresh()}>
            Refresh
          </Button>
        }
      />
      <PageMain>
        {status?.config_error ? (
          <StatusPanel
            variant="alert"
            headline="The stored configuration was refused"
            detail={`${status.config_error} — the worker is running on defaults and is not ingesting.`}
          />
        ) : null}
        {status && !status.enabled && !status.config_error ? (
          <StatusPanel
            variant="warn"
            headline="Ingest is not open yet"
            detail="The worker is up but its store or queue has not answered. Nothing is being recorded."
          />
        ) : null}
        {groups.error ? (
          <StatusPanel
            variant="alert"
            headline="Could not read the error groups"
            detail={groups.error}
            action={
              <Button size="sm" onClick={() => groups.refresh()}>
                Retry
              </Button>
            }
          />
        ) : null}

        {selected ? (
          <GroupDetailView
            api={api}
            host={host}
            groupId={selected}
            conversationId={conversationId ?? null}
            narrow={narrow}
            repositories={status?.repositories ?? []}
            onBack={() => setSelected(null)}
            onChanged={() => groups.refresh()}
          />
        ) : groups.data && groups.data.groups.length === 0 && !groups.loading ? (
          <EmptyState
            icon={ShieldAlert}
            title="Nothing has failed here"
            description={
              filters.search || filters.service
                ? 'No group matches this filter. Widen the window or clear the search.'
                : 'When a worker fails, the group appears here with the evidence already frozen.'
            }
          />
        ) : (
          <GroupsListView
            counts={status?.groups}
            filters={filters}
            groups={groups.data?.groups ?? []}
            live={groups.live}
            loading={groups.loading}
            narrow={narrow}
            onFilters={setFilters}
            onOpen={setSelected}
            panelSide={panelSide}
            total={groups.data?.total ?? 0}
          />
        )}
      </PageMain>
    </PageShell>
  )
}

/** The one-line descriptor in the page header: what the worker is watching. */
function describe(status: StatusResponse | null): string {
  if (!status) return 'sentinel'
  const sources = [
    status.sources.trace ? 'traces' : null,
    status.sources.log ? 'logs' : null,
  ].filter(Boolean)
  if (!status.enabled) return 'not ingesting'
  const where = sources.length ? sources.join(' and ') : 'nothing'
  const blind = status.engine.trace_store === 'disabled' ? ' · span store off' : ''
  return `watching ${where}${blind}`
}
