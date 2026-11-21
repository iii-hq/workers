import {
  Button,
  EmptyState,
  type Host,
  ListItem,
  type LiveAnnouncement,
  LiveRegion,
  MetaRow,
  PageBody,
  PageHeader,
  type PageHeaderMenuItem,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Skeleton,
  StatusBar,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useContainerNarrow, useCopyFlash, usePaneState } from '@iii-dev/console-ui/hooks'
import { ChevronLeft, Copy, FileJson, Plug, RefreshCw } from 'lucide-react'
import { type CSSProperties, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createApi } from './api'
import { focusColumn } from './columns'
import { FunctionDetail } from './FunctionDetail'
import {
  announcements,
  functionRows,
  type Level,
  narrowLevel,
  pageState,
  pathHeads,
  type RowActionId,
  statusBarText,
  type Tone,
} from './presentation'
import { ServerContents } from './ServerContents'
import { ServerList } from './ServerList'
import type { Selection, Server, ServersList } from './types'
import { useServers } from './use-servers'

const SKELETON_ROWS = [0, 1, 2, 3]
const COLUMN_OF: Record<Level, number> = { servers: 1, contents: 2, detail: 3 }

/** Column 3 for the Connection and Process rows, until their own views exist. */
function ServerFacts({ server }: { server: Server }) {
  return (
    <MetaRow
      items={[
        { label: 'Transport', value: server.transport },
        { label: 'Target', value: server.target },
        { label: 'Authentication', value: server.auth },
        { label: 'Protocol', value: server.protocol_version },
        { label: 'Server', value: server.server_info && `${server.server_info.name} ${server.server_info.version}` },
      ].filter((item) => item.value)}
    />
  )
}

/** The MCP servers page: the path row over servers, their contents and one thing in depth. */
export function McpServersPage({
  host,
  panelSide,
  onRequestClose,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const data = useServers(host)
  const api = useMemo(() => createApi(host.iii), [host])
  const [selection, setSelection] = useState<Selection>({ server: null, item: null })
  const [level, setLevel] = useState<Level>('servers')
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const [col1, setCol1] = usePaneState('mcp-client:col1', 288)
  const { ref: narrowRef, narrow } = useContainerNarrow()
  const root = useRef<HTMLDivElement | null>(null)
  const setRoot = useCallback(
    (el: HTMLDivElement | null) => {
      root.current = el
      narrowRef(el)
    },
    [narrowRef],
  )
  const state = pageState(data)
  const shown = narrowLevel(state, level)
  const server = data.list?.servers.find((s) => s.name === selection.server) ?? null
  const heads = pathHeads(selection, server, data.list?.servers.length ?? Object.keys(data.config ?? {}).length)
  const { item } = selection
  const fnRow = useMemo(
    () =>
      server && item?.kind === 'function'
        ? functionRows(server, data.functions[server.name] ?? []).find((row) => row.functionId === item.functionId)
        : undefined,
    [server, item, data.functions],
  )
  const fnGone = item?.kind === 'function' && server !== null && fnRow === undefined
  const copyFlash = useCopyFlash(item?.kind === 'function' ? item.functionId : '')
  // No add or import flow and no agent-access status yet: their buttons, commands and row stay hidden.
  const onAdd = undefined as (() => void) | undefined
  const onImport = undefined as (() => void) | undefined
  const accessRow = null as { tone: Tone; text: string } | null

  const reconnect = useCallback(
    (name: string) => void api.reconnect(name).then(data.refresh, data.refresh),
    [api, data.refresh],
  )
  const pickServer = (next: Selection) => {
    setSelection(next)
    setLevel('contents')
  }
  const pickItem = (next: Selection) => {
    setSelection(next)
    setLevel('detail')
  }
  const onAction = (name: string, action: RowActionId) => {
    pickServer({ server: name, item: { kind: 'connection' } })
    if (action === 'retry' || action === 'restart') reconnect(name)
  }
  const back = (to: Level, label: string) =>
    narrow ? (
      <Button
        variant="ghost"
        size="sm"
        className="mcp-ui-back"
        aria-label={`Back to ${label}`}
        onClick={() => setLevel(to)}
      >
        <ChevronLeft size={16} aria-hidden />
        {label}
      </Button>
    ) : null

  // A function the server no longer lists leaves its server on Connection.
  useEffect(() => {
    if (fnGone) setSelection((s) => ({ server: s.server, item: { kind: 'connection' } }))
  }, [fnGone])

  // Each server that became ready since the last read is said once.
  const previousList = useRef<ServersList | null>(null)
  useEffect(() => {
    if (!data.list) return
    const texts = announcements(previousList.current, data.list)
    previousList.current = data.list
    if (texts.length === 0) return
    setAnnouncement((last) => ({ seq: (last?.seq ?? 0) + 1, text: texts.join('. '), urgency: 'polite' }))
  }, [data.list])

  // A narrow drill-in hides the column that held focus: focus moves into the one shown.
  const lastShown = useRef(shown)
  useEffect(() => {
    if (lastShown.current === shown) return
    lastShown.current = shown
    if (narrow) focusColumn(root.current, COLUMN_OF[shown])
  }, [shown, narrow])

  useEffect(
    () =>
      commands?.register([
        { id: 'add-server', title: 'Add MCP server', enabled: () => onAdd !== undefined, run: () => onAdd?.() },
        {
          id: 'import-config',
          title: 'Import MCP config',
          enabled: () => onImport !== undefined,
          run: () => onImport?.(),
        },
        {
          id: 'reconnect',
          title: 'Reconnect server',
          enabled: () => selection.server !== null,
          run: () => {
            if (selection.server !== null) reconnect(selection.server)
          },
        },
        {
          id: 'copy-id',
          title: 'Copy function id',
          enabled: () => selection.item?.kind === 'function',
          run: copyFlash.copy,
        },
        {
          id: 'review-access',
          title: 'Review agent access',
          enabled: () => accessRow !== null,
          run: () => {
            setSelection({ server: selection.server, item: { kind: 'access' } })
            setLevel('detail')
          },
        },
      ]),
    [commands, selection, reconnect, copyFlash.copy, onAdd, onImport, accessRow],
  )
  const retry = (
    <Button variant="secondary" size="sm" onClick={data.refresh}>
      Retry
    </Button>
  )
  const menu: PageHeaderMenuItem[] = [
    { id: 'edit-json', label: 'Edit as JSON', icon: <FileJson size={16} />, disabled: true, onSelect: () => {} },
    { id: 'copy-json', label: 'Copy as JSON', icon: <Copy size={16} />, disabled: true, onSelect: () => {} },
    { id: 'refresh', label: 'Refresh', icon: <RefreshCw size={16} />, onSelect: data.refresh },
  ]

  return (
    <div
      className="mcp-ui-root"
      ref={setRoot}
      data-narrow={narrow}
      data-level={shown}
      data-side={panelSide}
      style={{ '--mcp-ui-col1': `${col1}px` } as CSSProperties}
    >
      <PageShell>
        <PageHeader
          icon={<Plug size={16} />}
          title="MCP servers"
          description="Tools from MCP servers, registered as iii functions"
          menu={menu}
          onClose={onRequestClose}
        />
        <div className="mcp-ui-path">
          <div className="mcp-ui-path-head">
            <code>{heads.first}</code>
            {state === 'loading' ? null : <span>{heads.firstMeta}</span>}
          </div>
          <div className="mcp-ui-path-head">
            {heads.second ? <code>{heads.second}</code> : null}
            {heads.secondMeta ? <span>{heads.secondMeta}</span> : null}
          </div>
          <div className="mcp-ui-path-head">
            {heads.thirdIsId ? <code>{heads.third}</code> : heads.third ? <b>{heads.third}</b> : null}
            {heads.thirdIsId ? (
              <Button variant="ghost" size="sm" onClick={copyFlash.copy}>
                <Copy size={16} aria-hidden />
                {copyFlash.state === 'copied' ? 'Copied' : 'Copy function id'}
              </Button>
            ) : null}
          </div>
        </div>
        <PageBody side={panelSide}>
          {state === 'config_down' ? (
            <PageMain className="mcp-ui-whole">
              <StatusPanel
                variant="alert"
                headline="The configuration worker cannot be reached"
                detail={data.configError}
                action={retry}
              />
            </PageMain>
          ) : (
            <>
              <PageSidebar
                className="mcp-ui-col1"
                label="Servers"
                side={panelSide}
                resizable
                minWidth={240}
                maxWidth={420}
                width={col1}
                onWidthChange={setCol1}
                narrow={narrow}
              >
                <ServerList
                  data={data}
                  selection={selection}
                  onSelect={pickServer}
                  onAction={onAction}
                  onAdd={onAdd}
                  onImport={onImport}
                />
              </PageSidebar>
              <PageMain className="mcp-ui-main">
                {state === 'worker_down' ? (
                  <div className="mcp-ui-whole">
                    <StatusPanel
                      variant="alert"
                      headline="The mcp-client worker is not running"
                      detail="Start it, then retry. Servers and their functions come back by themselves."
                      action={retry}
                    />
                  </div>
                ) : state === 'first_run' ? (
                  <div className="mcp-ui-whole">
                    <EmptyState
                      icon={Plug}
                      title="No MCP servers yet"
                      description="Add a server to register its tools as iii functions."
                    />
                  </div>
                ) : (
                  <div className="mcp-ui-cols">
                    <div className="mcp-ui-col2">
                      {state === 'loading' ? (
                        <>
                          <div className="mcp-ui-search" aria-hidden>
                            <Skeleton className="mcp-ui-skeleton-search" />
                          </div>
                          <div className="mcp-ui-list" aria-hidden>
                            {SKELETON_ROWS.map((n) => (
                              <ListItem
                                key={n}
                                as="div"
                                className="mcp-ui-fn"
                                label={<Skeleton className="mcp-ui-skeleton-name" />}
                                description={<Skeleton className="mcp-ui-skeleton-text" />}
                              />
                            ))}
                          </div>
                        </>
                      ) : (
                        <>
                          {back('servers', 'MCP servers')}
                          {server ? (
                            <ServerContents
                              key={server.name}
                              server={server}
                              functions={data.functions[server.name] ?? []}
                              selection={selection}
                              onSelect={pickItem}
                              accessRow={accessRow}
                              onRetryLists={() => reconnect(server.name)}
                            />
                          ) : null}
                        </>
                      )}
                    </div>
                    <div className="mcp-ui-col3" data-column="3">
                      {state === 'loading' ? (
                        SKELETON_ROWS.map((n) => <Skeleton key={n} className="mcp-ui-skeleton-line" />)
                      ) : (
                        <>
                          {selection.server ? back('contents', selection.server) : null}
                          {server && (item?.kind === 'connection' || item?.kind === 'process') ? (
                            <ServerFacts server={server} />
                          ) : null}
                          {server && fnRow ? (
                            <FunctionDetail api={api} server={server} row={fnRow} host={host} commands={commands} />
                          ) : null}
                        </>
                      )}
                    </div>
                  </div>
                )}
              </PageMain>
            </>
          )}
        </PageBody>
        {data.list ? (
          <StatusBar
            className="mcp-ui-status"
            end={narrow ? undefined : 'Arrow keys move between columns · Mod+Enter runs the test'}
          >
            {statusBarText(data.list)}
          </StatusBar>
        ) : null}
      </PageShell>
      <LiveRegion announcement={announcement} />
    </div>
  )
}
