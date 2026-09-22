import {
  Button,
  IconButton,
  LiveRegion,
  PageHeader,
  PageMain,
  PageShell,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import {
  useContainerNarrow,
  useDebounce,
  usePaneState,
  useWorkerLive,
} from '@iii-dev/console-ui/hooks'
import type { Host, PageRenderProps } from '@iii-dev/console-ui'
import { Activity, ArrowLeft, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { client } from '../api'
import type { GroupStatus, GroupSummary, StatusResponse } from '../api'
import { EVENT, PAGE_ID } from '../shared'
import { GroupDetailView } from './detail'
import { GroupsListView } from './list'
import { OPEN_STATES, ago, bulkOutcome, sinceMs } from './present.js'

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

export function SentinelPage({
  commands,
  conversationId,
  host,
  onRequestClose,
  paneId,
  panelContext,
  tabId,
}: Props) {
  const api = useMemo(() => client(host.iii), [host.iii])
  const { ref, narrow } = useContainerNarrow({ below: 760 })
  // The same page can be open in two columns of one tab, so everything that
  // survives a reload is keyed per pane rather than per page.
  const pane = paneId || tabId || 'alone'
  const [filters, setFilters] = usePaneState<Filters>(`sentinel:${pane}:filters`, INITIAL_FILTERS)
  const [selected, setSelected] = usePaneState<string | null>(`sentinel:${pane}:group`, null)
  // A visual panel says nothing to a screen reader. `seq` is monotonic so
  // the same sentence twice is announced twice.
  const [notice, setNotice] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState<{
    seq: number
    text: string
    urgency: 'polite' | 'assertive'
  } | null>(null)
  // Said to a screen reader only: the page's own state already shows it.
  const announce = useCallback((text: string) => {
    setAnnouncement((previous) => ({ seq: (previous?.seq ?? 0) + 1, text, urgency: 'polite' }))
  }, [])
  // Said and shown: a batch outcome has nothing else on screen to say it.
  const notify = useCallback(
    (text: string) => {
      setNotice(text)
      announce(text)
    },
    [announce],
  )

  // A sibling page can open this one on a group (`panels.open`).
  const contextGroup =
    panelContext?.context && typeof panelContext.context === 'object'
      ? (panelContext.context as { group_id?: string }).group_id
      : undefined
  useEffect(() => {
    if (contextGroup) setSelected(contextGroup)
  }, [contextGroup])

  // Settled, so a filter is a query rather than one query per keystroke.
  const search = useDebounce(filters.search, 250)
  const statuses = filters.statuses.join(',')

  const groups = useWorkerLive({
    iii: host.iii,
    handlerId: `iii::${PAGE_ID}-ui::events::${pane}`,
    triggers: [EVENT.groupChanged, EVENT.investigationChanged],
    fetch: useCallback(
      () =>
        api.groups({
          status: filters.statuses,
          service_name: filters.service || undefined,
          since_ms: sinceMs(filters.window, Date.now()) ?? undefined,
          search: search || undefined,
          limit: 50,
        }),
      [api, filters.statuses, filters.service, filters.window, search],
    ),
  })

  // `useWorkerLive` re-reads on its triggers and its poll, not when the query
  // itself changes. Without this the list answers the previous question until
  // the next error arrives, and the search box looks broken.
  const refresh = useRef(groups.refresh)
  refresh.current = groups.refresh
  useEffect(() => {
    refresh.current()
  }, [statuses, filters.service, filters.window, search])

  // Every worker the list has shown, so narrowing to one does not make the
  // others vanish from the menu that would widen it again.
  const [workers, setWorkers] = useState<string[]>([])
  useEffect(() => {
    const seen = groups.data?.groups.map((group) => group.service_name) ?? []
    setWorkers((previous) => {
      const next = [...new Set([...previous, ...seen])].sort()
      return next.length === previous.length ? previous : next
    })
  }, [groups.data])

  // "12 s ago" has to keep moving between refreshes.
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const tick = window.setInterval(() => setNow(Date.now()), 15_000)
    return () => window.clearInterval(tick)
  }, [])
  useEffect(() => setNow(Date.now()), [groups.data])

  const [opened, setOpened] = useState<GroupSummary | null>(null)
  useEffect(() => {
    if (!selected) setOpened(null)
  }, [selected])

  const bulk = useBulkActions(api, () => groups.refresh(), notify)
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
        title: 'Refresh error groups',
        run: () => groups.refresh(),
      },
      {
        id: 'back',
        title: 'Back to the groups',
        enabled: () => selected !== null,
        run: () => setSelected(null),
      },
    ])
  }, [commands, groups, selected])

  return (
    <PageShell ref={ref} data-narrow={narrow}>
      <PageHeader
        icon={<Activity size={16} />}
        title="Sentinel"
        description={
          selected
            ? opened
              ? [opened.service_name, opened.function_id].filter(Boolean).join(' · ')
              : undefined
            : describe(status)
        }
        onClose={onRequestClose}
        actions={
          <>
            {!selected && status?.groups.last_seen_ms ? (
              <span className="sentinel-ui-ingested">
                ingested {ago(status.groups.last_seen_ms, now)}
              </span>
            ) : null}
            <IconButton label="Refresh" onClick={() => groups.refresh()}>
              <RefreshCw size={16} />
            </IconButton>
          </>
        }
      >
        {selected ? (
          <Button variant="ghost" size="sm" onClick={() => setSelected(null)}>
            <ArrowLeft size={16} />
            Groups
          </Button>
        ) : null}
      </PageHeader>
      <PageMain>
        <LiveRegion announcement={announcement} />
        {/* Page-wide news sits in the same column as the content under it. */}
        <div className={selected ? 'sentinel-ui-banners sentinel-ui-banners--column' : 'sentinel-ui-banners'}>
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
              headline="Nothing is being recorded"
              detail="Either the configuration switches ingest off, or the worker's store or queue has not answered yet."
            />
          ) : null}
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
        </div>

        {selected ? (
          <GroupDetailView
            api={api}
            host={host}
            groupId={selected}
            conversationId={conversationId ?? null}
            narrow={narrow}
            repositories={status?.repositories ?? []}
            now={now}
            onBack={() => setSelected(null)}
            onChanged={() => groups.refresh()}
            onLoaded={setOpened}
            announce={announce}
          />
        ) : (
          <GroupsListView
            busy={bulk.busy}
            onBulk={bulk.run}
            status={status}
            filters={filters}
            groups={groups.data?.groups ?? []}
            loading={groups.loading || !groups.data}
            narrow={narrow}
            now={now}
            onFilters={setFilters}
            onOpen={setSelected}
            total={groups.data?.total ?? 0}
            workers={workers}
          />
        )}
      </PageMain>
    </PageShell>
  )
}

/** The one-line descriptor in the page header: what the worker is watching. */
function describe(status: StatusResponse | null): string {
  if (status && !status.enabled) return 'not ingesting'
  if (status && !status.sources.trace && !status.sources.log) return 'every source is switched off'
  return 'errors across every worker on this engine'
}

/**
 * Applying one decision across a selection.
 *
 * Each group is its own compare-and-set, so a batch is a loop rather than a
 * transaction — and that is the honest shape: a group somebody resolved a
 * second ago should refuse, and the rest should still go through. What the
 * person gets back is a count and the first reason, not a silent partial
 * success.
 */
function useBulkActions(
  api: ReturnType<typeof client>,
  refresh: () => void,
  notify: (message: string) => void,
) {
  const [busy, setBusy] = useState(false)

  const run = async (action: string, groupIds: string[]) => {
    setBusy(true)
    const failures: string[] = []
    let done = 0
    for (const groupId of groupIds) {
      try {
        if (action === 'resolve') await api.resolve(groupId, false)
        else if (action === 'ignore') await api.ignore(groupId, { kind: 'forever' })
        else if (action === 'ignore-version') await api.ignore(groupId, { kind: 'version_change' })
        else if (action === 'unignore') await api.unignore(groupId)
        else if (action === 'reopen') await api.reopen(groupId)
        done += 1
      } catch (cause) {
        failures.push(errorMessage(cause))
      }
    }
    setBusy(false)
    notify(bulkOutcome(action.replace('-version', ''), done, failures))
    refresh()
  }

  return { busy, run }
}
