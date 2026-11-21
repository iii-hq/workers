import { type Host, List, ListItem, PageHeader, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { Plug } from 'lucide-react'
import { useServers } from './use-servers'

/** The MCP servers page: the header over the configured servers. */
export function McpServersPage({ host, onRequestClose }: { host: Host } & Partial<PageRenderProps>) {
  const { list } = useServers(host)
  return (
    <PageShell className="mcp-ui-root">
      <PageHeader
        icon={<Plug size={16} />}
        title="MCP servers"
        description="Tools from MCP servers, registered as iii functions"
        onClose={onRequestClose}
      />
      <List>
        {list?.servers.map((server) => (
          <ListItem key={server.name} label={server.name} />
        ))}
      </List>
    </PageShell>
  )
}
