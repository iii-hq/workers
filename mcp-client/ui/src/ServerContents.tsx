import {
  Button,
  Chip,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  SearchField,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { coded } from './AgentAccessView'
import { currentRow, onColumnKeys } from './columns'
import {
  connectionRowText,
  countLabel,
  type FunctionRow,
  filterRows,
  functionRows,
  middleTruncate,
  presentServer,
  processRowText,
  rowGroups,
  STATE_WORD,
  skippedText,
  type Tone,
} from './presentation'
import type { EngineFunction, Item, Selection, Server } from './types'

/** Column 2: one server's pinned rows, then its functions in groups under the search field. */
export function ServerContents({
  server,
  functions,
  selection,
  onSelect,
  accessRow,
  accessAlert,
  onRetryLists,
}: {
  server: Server
  functions: EngineFunction[]
  selection: Selection
  onSelect: (s: Selection) => void
  accessRow: { tone: Tone; text: string } | null
  /** An agent can write without a person: said above Connection. */
  accessAlert: string | null
  onRetryLists: () => void
}) {
  const [query, setQuery] = useState('')
  const now = Date.now()
  const { dot, pulse, panel } = presentServer(server, now)
  const { rows, countText } = filterRows(functionRows(server, functions), query)
  const skipped = skippedText(server)
  const { item } = selection
  const pick = (next: Item) => onSelect({ server: server.name, item: next })
  // The name muted in a fixed cell, then the value with its dot (and a detail line under it), then a chevron.
  const pinned = (
    kind: 'connection' | 'process' | 'access',
    label: string,
    tone: Tone,
    value: string,
    detail?: string,
    pulse = false,
  ) => (
    <ListItem
      className="mcp-ui-pin"
      {...currentRow(item?.kind === kind)}
      leading={label}
      label={
        <>
          <StatusDot tone={tone} pulse={pulse} />
          <span>{value}</span>
        </>
      }
      description={detail}
      trailing={<ChevronRight size={16} aria-hidden />}
      onClick={() => pick({ kind })}
    />
  )
  const fnRow = (row: FunctionRow) => (
    <ListItem
      key={row.functionId}
      className="mcp-ui-fn"
      {...currentRow(item?.kind === 'function' && item.functionId === row.functionId)}
      title={row.name}
      label={middleTruncate(row.name, row.chip ? 28 : 36)}
      description={row.description ? coded(row.description) : undefined}
      trailing={row.chip ? <Chip tone={row.chip === 'Writes' ? 'neutral' : 'warning'}>{row.chip}</Chip> : null}
      onClick={() => pick({ kind: 'function', functionId: row.functionId })}
    />
  )

  return (
    <>
      <div className="mcp-ui-search">
        <SearchField
          className="mcp-ui-search-field"
          aria-label="Filter functions"
          placeholder={`Filter ${countLabel(server)}`}
          value={query}
          onChange={setQuery}
        />
        <span className="mcp-ui-count">{countText}</span>
      </div>
      <List
        role="group"
        aria-label={`${server.name} contents`}
        className="mcp-ui-list"
        data-column="2"
        onKeyDown={onColumnKeys}
      >
        {panel ? (
          <StatusPanel className="mcp-ui-panel" variant={panel.variant} headline={coded(panel.headline)} detail={panel.next} />
        ) : null}
        {accessAlert ? <StatusPanel className="mcp-ui-panel" variant="alert" headline={coded(accessAlert)} /> : null}
        <div className="mcp-ui-pins">
          {pinned('connection', 'Connection', dot, STATE_WORD[server.state], connectionRowText(server), pulse)}
          {server.transport === 'stdio'
            ? pinned(
                'process',
                'Process',
                server.process?.pid == null ? 'ghost' : 'ok',
                processRowText(server.process, now),
              )
            : null}
          {accessRow ? pinned('access', 'Agent access', accessRow.tone, accessRow.text) : null}
        </div>
        {rowGroups(server, rows, query).map((group) => (
          <ListGroup key={group.id} className="mcp-ui-group">
            <ListGroupLabel>
              {group.label} <span className="mcp-ui-group-count">{group.count}</span>
            </ListGroupLabel>
            {group.rows.map(fnRow)}
            {group.empty ? <p className="mcp-ui-quiet">{group.empty}</p> : null}
            {group.failed ? (
              <StatusPanel
                className="mcp-ui-panel"
                variant="alert"
                headline={group.failed.headline}
                detail={group.failed.detail}
                action={
                  <Button variant="secondary" size="sm" onClick={onRetryLists}>
                    Retry
                  </Button>
                }
              />
            ) : null}
          </ListGroup>
        ))}
        {skipped ? (
          <details className="mcp-ui-skipped">
            <summary>{skipped}</summary>
            <ul>
              {server.skipped.map((s) => (
                <li key={`${s.kind}:${s.name}`}>
                  <code>{s.name}</code>
                  <span>
                    {s.kind} · {s.reason}
                  </span>
                </li>
              ))}
            </ul>
          </details>
        ) : null}
      </List>
    </>
  )
}
