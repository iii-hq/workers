import {
  Button,
  EmptyState,
  type Host,
  List,
  ListItem,
  MetaRow,
  PageBody,
  PageHeader,
  type PageHeaderMenuItem,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useContainerNarrow, useCopyFlash, usePaneState } from '@iii-dev/console-ui/hooks'
import { Copy, FileJson, Plug, RefreshCw } from 'lucide-react'
import { type CSSProperties, useMemo, useState } from 'react'
import { createApi } from './api'
import { pageState, pathHeads, presentServer, type RowActionId } from './presentation'
import { ServerList } from './ServerList'
import type { Selection, Server } from './types'
import { useServers } from './use-servers'

const SKELETON_ROWS = [0, 1, 2, 3]

/** Column 3 for the Connection and Process rows, until their own views exist. */
function ServerFacts({ server }: { server: Server }) {
  const { panel } = presentServer(server, Date.now())
  return (
    <>
      {panel ? <StatusPanel variant={panel.variant} headline={panel.headline} detail={panel.next} /> : null}
      <MetaRow
        items={[
          { label: 'Transport', value: server.transport },
          { label: 'Target', value: server.target },
          { label: 'Authentication', value: server.auth },
          { label: 'Protocol', value: server.protocol_version },
          { label: 'Server', value: server.server_info && `${server.server_info.name} ${server.server_info.version}` },
        ].filter((item) => item.value)}
      />
    </>
  )
}

/** The MCP servers page: the path row over servers, their contents and one thing in depth. */
export function McpServersPage({ host, panelSide, onRequestClose }: { host: Host } & Partial<PageRenderProps>) {
  const data = useServers(host)
  const api = useMemo(() => createApi(host.iii), [host])
  const [selection, setSelection] = useState<Selection>({ server: null, item: null })
  const [col1, setCol1] = usePaneState('mcp-client:col1', 288)
  const { ref: narrowRef, narrow } = useContainerNarrow()
  const state = pageState(data)
  const server = data.list?.servers.find((s) => s.name === selection.server) ?? null
  const heads = pathHeads(selection, server, data.list?.servers.length ?? Object.keys(data.config ?? {}).length)
  const { item } = selection
  const copyFlash = useCopyFlash(item?.kind === 'function' ? item.functionId : '')
  const level = selection.server === null ? 'servers' : 'contents'

  const onAction = (name: string, action: RowActionId) => {
    setSelection({ server: name, item: { kind: 'connection' } })
    if (action === 'retry' || action === 'restart') void api.reconnect(name).then(data.refresh, data.refresh)
  }
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
      ref={narrowRef}
      data-narrow={narrow}
      data-level={level}
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
                label="Servers"
                side={panelSide}
                resizable
                minWidth={240}
                maxWidth={420}
                width={col1}
                onWidthChange={setCol1}
                narrow={narrow}
              >
                <ServerList data={data} selection={selection} onSelect={setSelection} onAction={onAction} />
              </PageSidebar>
              <PageMain>
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
                        SKELETON_ROWS.map((n) => (
                          <ListItem
                            key={n}
                            as="div"
                            aria-hidden
                            label={<Skeleton className="mcp-ui-skeleton-name" />}
                            description={<Skeleton className="mcp-ui-skeleton-text" />}
                          />
                        ))
                      ) : server ? (
                        <List aria-label={`${server.name} contents`}>
                          <ListItem
                            label="Connection"
                            selected={item?.kind === 'connection'}
                            onClick={() => setSelection({ server: server.name, item: { kind: 'connection' } })}
                          />
                        </List>
                      ) : null}
                    </div>
                    <div className="mcp-ui-col3">
                      {state === 'loading' ? (
                        SKELETON_ROWS.map((n) => <Skeleton key={n} className="mcp-ui-skeleton-line" />)
                      ) : server && (item?.kind === 'connection' || item?.kind === 'process') ? (
                        <ServerFacts server={server} />
                      ) : null}
                    </div>
                  </div>
                )}
              </PageMain>
            </>
          )}
        </PageBody>
      </PageShell>
    </div>
  )
}
