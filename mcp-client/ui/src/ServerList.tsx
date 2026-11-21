import { Button, List, ListItem, Skeleton, StatusDot } from '@iii-dev/console-ui'
import { Download, Plus } from 'lucide-react'
import type { KeyboardEvent } from 'react'
import { pageState, presentServer, type RowActionId } from './presentation'
import type { Selection } from './types'
import type { ServersData } from './use-servers'

const SKELETON_ROWS = [0, 1, 2, 3, 4]

/** Column 1: every configured server, its health and its one fix. */
export function ServerList({
  data,
  selection,
  onSelect,
  onAction,
  onAdd,
  onImport,
}: {
  data: ServersData
  selection: Selection
  onSelect: (s: Selection) => void
  onAction: (server: string, action: RowActionId) => void
  onAdd?: () => void
  onImport?: () => void
}) {
  const state = pageState(data)
  const now = Date.now()
  const select = (server: string) => onSelect({ server, item: { kind: 'connection' } })
  const row = (server: string) => ({
    as: 'div' as const,
    role: 'option',
    'data-list-roving': '',
    'data-level': 'server',
    className: 'mcp-ui-row',
    selected: server === selection.server,
    label: server,
    onClick: () => select(server),
    onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => {
      if (event.target !== event.currentTarget || (event.key !== 'Enter' && event.key !== ' ')) return
      event.preventDefault()
      select(server)
    },
  })

  return (
    <>
      <List role="listbox" aria-label="MCP servers" className="mcp-ui-list">
        {state === 'loading'
          ? SKELETON_ROWS.map((n) => (
              <ListItem
                key={n}
                as="div"
                aria-hidden
                className="mcp-ui-row"
                leading={<Skeleton className="mcp-ui-skeleton-dot" />}
                label={<Skeleton className="mcp-ui-skeleton-name" />}
                description={<Skeleton className="mcp-ui-skeleton-text" />}
              />
            ))
          : state === 'worker_down'
            ? Object.keys(data.config ?? {})
                .sort()
                .map((name) => <ListItem key={name} {...row(name)} leading={<StatusDot tone="ghost" />} />)
            : [...(data.list?.servers ?? [])]
                .sort((a, b) => a.name.localeCompare(b.name))
                .map((server) => {
                  const { dot, pulse, description, action } = presentServer(server, now)
                  return (
                    <ListItem
                      key={server.name}
                      {...row(server.name)}
                      data-tone={dot}
                      leading={<StatusDot tone={dot} pulse={pulse} />}
                      description={description}
                      trailing={
                        action ? (
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={(event) => {
                              event.stopPropagation()
                              onAction(server.name, action.id)
                            }}
                          >
                            {action.label}
                          </Button>
                        ) : server.tools_listed ? (
                          String(server.tools.length)
                        ) : null
                      }
                    />
                  )
                })}
      </List>
      {onAdd || onImport ? (
        <div className="mcp-ui-foot">
          {onAdd ? (
            <Button variant="ghost" size="sm" onClick={onAdd}>
              <Plus size={16} aria-hidden />
              Add server
            </Button>
          ) : null}
          {onImport ? (
            <Button variant="ghost" size="sm" onClick={onImport}>
              <Download size={16} aria-hidden />
              Import
            </Button>
          ) : null}
        </div>
      ) : null}
    </>
  )
}
