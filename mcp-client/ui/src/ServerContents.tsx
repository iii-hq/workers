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
import { type ReactNode, useState } from 'react'
import { currentRow, onColumnKeys } from './columns'
import {
  connectionRowText,
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
  const pinned = (
    kind: 'connection' | 'process' | 'access',
    label: string,
    description?: string,
    trailing?: ReactNode,
  ) => (
    <ListItem
      className="mcp-ui-pin"
      {...currentRow(item?.kind === kind)}
      label={label}
      description={description}
      trailing={trailing}
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
      description={row.description || undefined}
      trailing={row.chip ? <Chip tone={row.chip === 'Writes' ? 'neutral' : 'warning'}>{row.chip}</Chip> : null}
      onClick={() => pick({ kind: 'function', functionId: row.functionId })}
    />
  )

  return (
    <>
      <div className="mcp-ui-search">
        <SearchField className="mcp-ui-search-field" aria-label="Filter functions" value={query} onChange={setQuery} />
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
          <StatusPanel className="mcp-ui-panel" variant={panel.variant} headline={panel.headline} detail={panel.next} />
        ) : null}
        {accessAlert ? <StatusPanel className="mcp-ui-panel" variant="alert" headline={accessAlert} /> : null}
        {pinned(
          'connection',
          'Connection',
          connectionRowText(server),
          <>
            <StatusDot tone={dot} pulse={pulse} />
            {STATE_WORD[server.state]}
          </>,
        )}
        {server.transport === 'stdio' ? pinned('process', 'Process', processRowText(server.process, now)) : null}
        {accessRow
          ? pinned(
              'access',
              'Agent access',
              undefined,
              <>
                <StatusDot tone={accessRow.tone} />
                {accessRow.text}
              </>,
            )
          : null}
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
