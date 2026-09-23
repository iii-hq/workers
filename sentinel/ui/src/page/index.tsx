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
import { OPEN_STATES, PAGE_SIZE, ago, bulkOutcome, sinceMs } from './present.js'

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
  // A page of groups, and more on request; a new filter starts from one page.
  const [limit, setLimit] = useState(PAGE_SIZE)
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
          limit,
        }),
      [api, filters.statuses, filters.service, filters.window, search, limit],
    ),
  })

  // `useWorkerLive` re-reads on its triggers and its poll, not when the query
  // itself changes. Without this the list answers the previous question until
  // the next error arrives, and the search box looks broken.
  const refresh = useRef(groups.refresh)
  refresh.current = groups.refresh
  useEffect(() => {
    refresh.current()
  }, [statuses, filters.service, filters.window, search, limit])
  useEffect(() => setLimit(PAGE_SIZE), [statuses, filters.service, filters.window, search])

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

  // The list refetches on every group event and on the fallback poll; each
  // new answer is the detail's cue to re-read its own group.
  const [revision, setRevision] = useState(0)
  useEffect(() => {
    if (groups.data) setRevision((previous) => previous + 1)
  }, [groups.data])

  // "12s ago" has to keep moving between refreshes.
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const tick = window.setInterval(() => setNow(Date.now()), 15_000)
    return () => window.clearInterval(tick)
  }, [])
  useEffect(() => setNow(Date.now()), [groups.data])

  // Opening a group, or going back, starts at the top rather than wherever
  // the other view was scrolled to.
  const scroller = useRef<HTMLDivElement | null>(null)
  // …except the list, which comes back to where it was left: the row a
  // person opened is the row they return to.
  const listTop = useRef(0)
  const openGroup = useCallback((groupId: string) => {
    listTop.current = scroller.current?.scrollTop ?? 0
    setSelected(groupId)
  }, [])
  useEffect(() => {
    scroller.current?.scrollTo({ top: selected ? 0 : listTop.current })
  }, [selected])

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
        // On a group the header reads title, back, then where it failed —
        // the design's order — so the kicker rides after the back action
        // instead of in `description`, which renders before it.
        description={selected ? undefined : describe(status)}
        onClose={onRequestClose}
        actions={
          <>
            {!selected && !narrow && status?.groups.last_seen_ms ? (
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
          narrow ? (
            // A worded button does not fit beside the header's own actions
            // on a phone; the arrow keeps its name for everyone else.
            <IconButton label="Back to the groups" onClick={() => setSelected(null)}>
              <ArrowLeft size={16} />
            </IconButton>
          ) : (
            <Button variant="ghost" size="sm" onClick={() => setSelected(null)}>
              <ArrowLeft size={16} />
              Groups
            </Button>
          )
        ) : null}
        {selected && opened ? (
          <span className="sentinel-ui-header-kicker">
            {[opened.service_name, opened.function_id].filter(Boolean).join(' · ')}
          </span>
        ) : null}
      </PageHeader>
      <PageMain>
        <LiveRegion announcement={announcement} />
        {/* `PageMain` clips by contract; the page owns its scroll. */}
        <div className="sentinel-ui-scroll" ref={scroller}>
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
            revision={revision}
              announce={announce}
            />
          ) : (
            <GroupsListView
              busy={bulk.busy}
              onBulk={bulk.run}
              status={status}
              filters={filters}
              groups={groups.data?.groups ?? []}
              // No answer yet is loading; a failed first read is the error
            // panel above, not skeleton rows that never resolve.
            loading={groups.loading || (!groups.data && !groups.error)}
              narrow={narrow}
              now={now}
              onFilters={setFilters}
              onOpen={openGroup}
              onMore={() => setLimit((previous) => previous + PAGE_SIZE)}
              total={groups.data?.total ?? 0}
              workers={workers}
            />
          )}
        </div>
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
