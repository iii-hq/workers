import { Button, List, ListItem, Skeleton, StatusDot } from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { Download, Plus } from 'lucide-react'
import type { ReactNode } from 'react'
import { currentRow, onColumnKeys } from './columns'
import { byName, pageState, presentServer, type RowActionId, type Tone } from './presentation'
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
  // The row is the shared list-item look; its pick button and its fix action are siblings in it,
  // since a button inside a button (or inside an option) is lost to assistive tech.
  const row = (name: string, tone: Tone, pulse = false, description?: string, count?: string, action?: ReactNode) => {
    const selected = name === selection.server
    return (
      <div
        key={name}
        className={`${uiClasses.listItem} mcp-ui-row mcp-ui-server`}
        data-selected={selected || undefined}
        data-tone={tone}
      >
        <button
          type="button"
          className="mcp-ui-pick"
          data-list-item=""
          data-list-roving=""
          data-level="server"
          {...currentRow(selected)}
          onClick={() => onSelect({ server: name, item: { kind: 'connection' } })}
        >
          <span className={uiClasses.listItemIcon}>
            <StatusDot tone={tone} pulse={pulse} />
          </span>
          <span className={uiClasses.listItemContent}>
            <span className={uiClasses.listItemTitle}>{name}</span>
            {description ? <span className={uiClasses.listItemDescription}>{description}</span> : null}
          </span>
          {count ? <span className={uiClasses.listItemMeta}>{count}</span> : null}
        </button>
        {action}
      </div>
    )
  }

  return (
    <>
      <List role="group" aria-label="MCP servers" className="mcp-ui-list" data-column="1" onKeyDown={onColumnKeys}>
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
                .sort(byName)
                .map((name) => row(name, 'ghost'))
            : [...(data.list?.servers ?? [])]
                .sort((a, b) => byName(a.name, b.name))
                .map((server) => {
                  const { dot, pulse, description, action } = presentServer(server, now)
                  // One trailing thing: the fix, else the tool count once the tools are listed.
                  const count = !action && server.tools_listed ? String(server.tools.length) : undefined
                  return row(
                    server.name,
                    dot,
                    pulse,
                    description,
                    count,
                    action ? (
                      <Button
                        variant="secondary"
                        size="sm"
                        className="mcp-ui-fix"
                        onClick={() => onAction(server.name, action.id)}
                      >
                        {action.label}
                      </Button>
                    ) : null,
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
