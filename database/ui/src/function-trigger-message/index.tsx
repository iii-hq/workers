/**
 * Injected function-trigger message renderer for every `database::*` call —
 * registered through `host.functionTriggers`, so it dispatches BEFORE the
 * console's built-in families and overrides how database calls render in
 * chat and in the traces span tab.
 *
 * Coverage is deliberately total (all 13 functions), grouped by response
 * shape: query-like (rows table), execute-like (affected rows), tx-like
 * (step list with failed-step marking), handles (prepare/begin/commit/
 * rollback), and listDatabases (config table). Error outputs return null
 * and fall through to the console's built-in error card — it already
 * renders the worker's `{code: "UNKNOWN_DB", available: [...]}` bodies
 * legibly. Unknown/odd success shapes degrade to a JSON dump inside the
 * card, never a throw. The card carries a small "database ui" tag so an
 * override is distinguishable from first-party rendering at a glance.
 */

import {
  Badge,
  CodeHighlight,
  Eyebrow,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
  JsonHighlight,
  MetaRow,
  type MetaRowItem,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  uiClasses,
} from '@iii-dev/console-ui'
import { unwrapEnvelope } from '@iii-dev/console-ui/format'
import {
  DB_PREFIX,
  type DbRequest,
  asRecord,
  asString,
  cellText,
  isErrorOutput,
  parseExecuteResp,
  parseListDatabases,
  parseQueryResp,
  parseRequest,
  parseTxResp,
  shortId,
} from './parsers'

const MAX_ROWS = 50

/** The request's identifiers, as the card's opening metadata strip. */
function requestItems(req: DbRequest): MetaRowItem[] {
  const items: MetaRowItem[] = []
  if (req.db) items.push({ label: 'db', value: req.db })
  if (req.transactionId) {
    items.push({ label: 'tx', value: shortId(req.transactionId) })
  }
  if (req.handleId) items.push({ label: 'handle', value: shortId(req.handleId) })
  if (req.isolation) items.push({ label: 'isolation', value: req.isolation })
  return items
}

function CardShell({
  op,
  running,
  items,
  children,
}: {
  op: string
  running?: boolean
  items?: MetaRowItem[]
  children?: React.ReactNode
}) {
  return (
    <div className="db-ui-msg">
      <div className="db-ui-msg-head">
        <Badge variant={running ? 'default' : 'accent'}>{op}</Badge>
        <Eyebrow className="db-ui-msg-tag">Database UI</Eyebrow>
      </div>
      {items && items.length > 0 ? <MetaRow items={items} /> : null}
      {children}
    </div>
  )
}

function SqlBlock({ req }: { req: DbRequest }) {
  if (!req.sql) return null
  return (
    <>
      <div className="db-ui-sql">
        <CodeHighlight code={req.sql} language="sql" wrap />
      </div>
      {req.params ? (
        <div className="db-ui-params">
          <span className="k">Params </span>
          {JSON.stringify(req.params)}
        </div>
      ) : null}
    </>
  )
}

function RowsTable({
  columns,
  rows,
}: {
  columns: string[]
  rows: Record<string, unknown>[]
}) {
  if (rows.length === 0 || columns.length === 0) return null
  const shown = rows.slice(0, MAX_ROWS)
  return (
    <div className="db-ui-table-wrap">
      <TableViewport>
        <TableFrame>
          <Table density="compact" className="db-ui-table">
            <TableHeader>
              <TableRow>
                {columns.map((c) => (
                  <TableHead key={c}>{c}</TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {shown.map((row, i) => (
                <TableRow key={i}>
                  {columns.map((c) => {
                    const { text, isNull } = cellText(row[c])
                    return (
                      <TableCell key={c} title={text}>
                        {isNull ? (
                          <span className="db-cell-null">{text}</span>
                        ) : (
                          text
                        )}
                      </TableCell>
                    )
                  })}
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </TableFrame>
      </TableViewport>
      {rows.length > MAX_ROWS ? (
        <div className="db-ui-msg-note">
          +{rows.length - MAX_ROWS} more rows
        </div>
      ) : null}
    </div>
  )
}

/** Last-resort body for a success shape we don't recognize. */
function RawDetails({ details }: { details: unknown }) {
  return (
    <div className="db-ui-sql">
      <JsonHighlight code={JSON.stringify(details ?? null, null, 2)} />
    </div>
  )
}

function QueryView({ req, details }: { req: DbRequest; details: unknown }) {
  const resp = parseQueryResp(details)
  return (
    <>
      <SqlBlock req={req} />
      {resp ? (
        resp.rows.length === 0 ? (
          <div className="db-ui-msg-note">· 0 rows</div>
        ) : (
          <RowsTable columns={resp.columns} rows={resp.rows} />
        )
      ) : (
        <RawDetails details={details} />
      )}
    </>
  )
}

function ExecuteView({ req, details }: { req: DbRequest; details: unknown }) {
  const resp = parseExecuteResp(details)
  return (
    <>
      <SqlBlock req={req} />
      <div className="db-ui-msg-note">
        {resp.affectedRows !== undefined
          ? `${resp.affectedRows} row${resp.affectedRows === 1 ? '' : 's'} affected`
          : 'Done'}
        {resp.lastInsertId ? ` · last insert id ${resp.lastInsertId}` : ''}
      </div>
      {resp.returnedRows.length > 0 ? (
        <RowsTable
          columns={Object.keys(resp.returnedRows[0])}
          rows={resp.returnedRows}
        />
      ) : null}
    </>
  )
}

function TxView({ req, details }: { req: DbRequest; details: unknown }) {
  const resp = parseTxResp(details)
  const statements = req.statements ?? []
  const count = Math.max(statements.length, resp.steps.length)
  return (
    <>
      {count > 0 ? (
        <div className="db-ui-steps">
          {Array.from({ length: count }, (_, i) => {
            const failed = resp.failedIndex === i
            const sql = statements[i]?.sql
            const affected = resp.steps[i]?.affectedRows
            return (
              <div key={i} className={`db-ui-step${failed ? ' failed' : ''}`}>
                <span className="idx">{i + 1}</span>
                <span className="sql">{sql ?? '—'}</span>
                <span className="meta">
                  {failed
                    ? 'Failed'
                    : affected !== undefined
                      ? `${affected} affected`
                      : ''}
                </span>
              </div>
            )
          })}
        </div>
      ) : (
        <RawDetails details={details} />
      )}
    </>
  )
}

function ListDatabasesView({ details }: { details: unknown }) {
  const dbs = parseListDatabases(details)
  if (dbs.length === 0)
    return <div className="db-ui-msg-note">No databases configured</div>
  const rows = dbs.map((d) => ({
    name: d.name,
    driver: d.driver,
    url: d.url,
    'pool max': d.poolMax,
  }))
  return (
    <RowsTable columns={['name', 'driver', 'url', 'pool max']} rows={rows} />
  )
}

function HandleView({ details, label }: { details: unknown; label: string }) {
  // PrepareResp{handle:{id,expires_at}} / BeginTxResp{transaction:{...}}.
  const obj = asRecord(details)
  const handle = asRecord(obj.handle ?? obj.transaction)
  const id = asString(handle.id) ?? asString(obj.transaction_id)
  const expires = asString(handle.expires_at)
  return (
    <div className="db-ui-msg-note">
      {label}
      {id ? ` · ${id}` : ''}
      {expires ? ` · expires ${expires}` : ''}
    </div>
  )
}

function SettledView({ message }: { message: FunctionTriggerMessage }) {
  const op = message.functionId.slice(DB_PREFIX.length)
  const req = parseRequest(message.input)
  const details = unwrapEnvelope(message.output)

  const body = (() => {
    switch (op) {
      case 'query':
      case 'transactionQuery':
      case 'runStatement':
        return <QueryView req={req} details={details} />
      case 'execute':
      case 'transactionExecute':
        return <ExecuteView req={req} details={details} />
      case 'executeBatch':
      case 'transaction':
        return <TxView req={req} details={details} />
      case 'listDatabases':
        return <ListDatabasesView details={details} />
      case 'prepareStatement':
        return (
          <>
            <SqlBlock req={req} />
            <HandleView details={details} label="Statement prepared" />
          </>
        )
      case 'beginTransaction':
        return <HandleView details={details} label="Transaction open" />
      case 'commitTransaction':
        return <div className="db-ui-msg-note">Committed</div>
      case 'rollbackTransaction':
        return <div className="db-ui-msg-note">Rolled back</div>
      default:
        // Anything else under database:: (e.g. on-config-change).
        return <RawDetails details={details} />
    }
  })()

  const tx =
    op === 'executeBatch' || op === 'transaction'
      ? parseTxResp(details)
      : undefined
  const items = requestItems(req)
  if (tx?.committed === true) {
    items.push({ label: 'outcome', value: 'committed', tone: 'ok' })
  } else if (tx?.committed === false) {
    items.push({ label: 'outcome', value: 'rolled back', tone: 'alert' })
  }
  return (
    <CardShell op={op} items={items}>
      {body}
    </CardShell>
  )
}

function RunningView({ message }: { message: FunctionTriggerMessage }) {
  const op = message.functionId.slice(DB_PREFIX.length)
  const req = parseRequest(message.input)
  return (
    <CardShell op={op} running items={requestItems(req)}>
      <SqlBlock req={req} />
      <div className={`db-ui-msg-note ${uiClasses.pulse}`}>Running…</div>
    </CardShell>
  )
}

/** Pending-approval preview: the SQL a call is about to run. */
function Preview({ message }: { message: FunctionTriggerMessage }) {
  const op = message.functionId.slice(DB_PREFIX.length)
  const req = parseRequest(message.input)
  return (
    <CardShell op={op} items={requestItems(req)}>
      <SqlBlock req={req} />
      {req.statements ? (
        <div className="db-ui-steps">
          {req.statements.map((s, i) => (
            <div key={i} className="db-ui-step">
              <span className="idx">{i + 1}</span>
              <span className="sql">{s.sql ?? '—'}</span>
            </div>
          ))}
        </div>
      ) : null}
    </CardShell>
  )
}

/** The worker's own branding of its ids: faint prefix, ink name. The console
    has no shared equivalent — its default is one plain span. */
function FunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith(DB_PREFIX)) {
    return <span style={{ color: 'var(--color-ink)' }}>{functionId}</span>
  }
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>{DB_PREFIX}</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>
        {functionId.slice(DB_PREFIX.length)}
      </span>
    </>
  )
}

export function createDatabaseTriggerRenderer(
  host: Host,
): FunctionTriggerRenderer {
  const render = (
    message: FunctionTriggerMessage,
    running: boolean,
  ): React.ReactNode | null => {
    if (!message.functionId.startsWith(DB_PREFIX)) return null
    if (message.pendingApproval) return null // host renders preview + approval bar
    if (running) return <RunningView message={message} />
    // Errors fall through to the console's built-in error card (it renders
    // the worker's {code, ...} bodies, e.g. UNKNOWN_DB + available list).
    if (isErrorOutput(message.output)) return null
    return <SettledView message={message} />
  }
  return {
    id: 'database/page.js#function-calls',
    isMatch: (functionId) => functionId.startsWith(DB_PREFIX),
    tryRender: (message) => render(message, !!message.running),
    tryRenderRunning: (message) => render(message, true),
    tryRenderPreview: (message) =>
      message.pendingApproval &&
      message.functionId.startsWith(DB_PREFIX) &&
      (parseRequest(message.input).sql ||
        parseRequest(message.input).statements) ? (
        <Preview message={message} />
      ) : null,
    FunctionIdLabel,
  }
}
