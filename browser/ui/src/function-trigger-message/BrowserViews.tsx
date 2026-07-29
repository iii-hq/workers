import { Badge, JsonHighlight } from '@iii-dev/console-ui'
import { z } from 'zod'
import {
  type BrowserConsoleEntry,
  type BrowserNetworkEntry,
  elementLabel,
  formatTime,
  levelBadgeVariant,
} from '../lib/browser'
import { cn } from '../lib/cn'
import { renderWithHighlight } from '../lib/highlight'
import { ActionLine, Chip, MetaRow, StatusPill } from '../lib/shared'
import {
  actResultSchema,
  consoleReadSchema,
  type DomNode,
  domReadResultSchema,
  evaluateResultSchema,
  historyResultSchema,
  navigateResultSchema,
  networkReadSchema,
  safeDecode,
  safeParseInput,
  sessionListResultSchema,
  sessionStartSchema,
  sessionStopResultSchema,
  snapshotResultSchema,
  stylesReadResultSchema,
  stylesWriteResultSchema,
} from './parsers'

/**
 * Per-function terminal views for `browser::*` chat cards. Every view
 * decodes the harness result envelope itself (see parsers) and returns
 * null when the payload doesn't parse, so the card falls back to the raw
 * JSON rendering instead of guessing.
 */

function truncate(s: string, max: number): string {
  const flat = s.replace(/\s+/g, ' ').trim()
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat
}

/* ---------------- snapshot ---------------- */

/** The a11y outline with the `[ref=eN]` handles subtly highlighted, reusing
 * the shared grep-style match highlighter. */
function SnapshotTree({ tree }: { tree: string }) {
  return (
    <pre className="br-ui-tree">
      <code>
        {renderWithHighlight(tree, '\\[ref=[^\\]]*\\]', {
          isRegex: true,
          ignoreCase: false,
        })}
      </code>
    </pre>
  )
}

export function SnapshotView({ output }: { output: unknown }) {
  const res = safeDecode(snapshotResultSchema, output)
  if (!res) return null
  return (
    <div>
      <MetaRow>
        <StatusPill label="snapshot" variant="accent" />
        {res.title ? <Chip>{truncate(res.title, 60)}</Chip> : null}
        {res.truncated ? <StatusPill label="truncated" variant="warn" /> : null}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="br-ui-break">{res.url}</span>
      </ActionLine>
      <SnapshotTree tree={res.tree} />
    </div>
  )
}

/* ---------------- sessions ---------------- */

export function SessionStartView({ output }: { output: unknown }) {
  const res = safeDecode(sessionStartSchema, output)
  if (!res) return null
  return (
    <div>
      <MetaRow>
        <StatusPill label="session started" variant="accent" />
        <Chip>{res.session_id}</Chip>
        <Chip>{res.headless ? 'headless' : 'headful'}</Chip>
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="br-ui-break">{res.url}</span>
      </ActionLine>
    </div>
  )
}

export function SessionStopView({ output }: { output: unknown }) {
  const res = safeDecode(sessionStopResultSchema, output)
  if (!res) return null
  return (
    <MetaRow>
      <StatusPill
        label={res.was_running ? 'stopped' : 'was not running'}
        variant={res.was_running ? 'accent' : 'default'}
      />
    </MetaRow>
  )
}

export function SessionListView({ output }: { output: unknown }) {
  const res = safeDecode(sessionListResultSchema, output)
  if (!res) return null
  return (
    <div>
      <MetaRow>
        <StatusPill
          label={`${res.sessions.length} sessions`}
          variant={res.sessions.length > 0 ? 'accent' : 'default'}
        />
      </MetaRow>
      {res.sessions.length === 0 ? (
        <div className="br-ui-empty-line">· no live sessions</div>
      ) : (
        <table className="br-ui-vtable">
          <tbody>
            {res.sessions.map((s) => (
              <tr key={s.session_id}>
                <td className="br-ui-td br-ui-td-accent br-ui-nowrap">
                  {s.session_id}
                </td>
                <td className="br-ui-td br-ui-break">{s.url}</td>
                <td className="br-ui-td br-ui-td-dim br-ui-nowrap">
                  {s.headless ? 'headless' : 'headful'}
                </td>
                <td className="br-ui-td br-ui-td-dim br-ui-num br-ui-right br-ui-nowrap">
                  {s.console_entries} logs
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}

/* ---------------- navigate / act / history ---------------- */

export function NavigateView({ output }: { output: unknown }) {
  const res = safeDecode(navigateResultSchema, output)
  if (!res) return null
  return (
    <div>
      <MetaRow>
        <StatusPill
          label={res.ok ? 'loaded' : 'failed'}
          variant={res.ok ? 'accent' : 'alert'}
        />
        {res.timed_out ? <StatusPill label="timed out" variant="warn" /> : null}
        {res.title ? <Chip>{truncate(res.title, 60)}</Chip> : null}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="br-ui-break">{res.url}</span>
      </ActionLine>
    </div>
  )
}

const actInputSchema = z.object({
  action: z.string().optional(),
  ref: z.string().optional(),
  x: z.number().optional(),
  y: z.number().optional(),
  key: z.string().optional(),
})

export function ActView({
  input,
  output,
}: {
  input: unknown
  output: unknown
}) {
  const res = safeDecode(actResultSchema, output)
  if (!res) return null
  const req = safeParseInput(actInputSchema, input)
  return (
    <div>
      <MetaRow>
        <StatusPill
          label={res.ok ? 'done' : 'failed'}
          variant={res.ok ? 'accent' : 'alert'}
        />
        {req?.action ? <Chip>{req.action}</Chip> : null}
        {req?.ref ? <Chip className="br-ui-chip-accent">{req.ref}</Chip> : null}
        {req?.key ? <Chip>{req.key}</Chip> : null}
        {req?.x != null && req?.y != null ? (
          <Chip className="br-ui-chip-num">
            {Math.round(req.x)},{Math.round(req.y)}
          </Chip>
        ) : null}
      </MetaRow>
      <ActionLine symbol="·" tone="ink">
        {res.detail}
      </ActionLine>
    </div>
  )
}

const historyInputSchema = z.object({ action: z.string().optional() })

export function HistoryView({
  input,
  output,
}: {
  input: unknown
  output: unknown
}) {
  const res = safeDecode(historyResultSchema, output)
  if (!res) return null
  const req = safeParseInput(historyInputSchema, input)
  return (
    <div>
      <MetaRow>
        <StatusPill
          label={req?.action ?? 'history'}
          variant={res.ok ? 'accent' : 'alert'}
        />
        {!res.moved ? (
          <StatusPill label="no history entry" variant="warn" />
        ) : null}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="br-ui-break">{res.url}</span>
      </ActionLine>
    </div>
  )
}

/* ---------------- console / network reads ---------------- */

const readInputSchema = z.object({
  pattern: z.string().optional(),
  level: z.string().optional(),
  failed_only: z.boolean().optional(),
  since_seq: z.number().optional(),
  limit: z.number().optional(),
})

export function ConsoleEntryRow({ entry }: { entry: BrowserConsoleEntry }) {
  return (
    <li className="br-ui-log-row">
      <span className="br-ui-log-time">{formatTime(entry.timestamp)}</span>
      <Badge variant={levelBadgeVariant(entry.level)} className="br-ui-log-level">
        {entry.level}
      </Badge>
      <span className="br-ui-log-text">
        {entry.text}
        {entry.source ? (
          <span className="br-ui-dim"> · {entry.source}</span>
        ) : null}
      </span>
    </li>
  )
}

export function ConsoleReadView({
  input,
  output,
}: {
  input: unknown
  output: unknown
}) {
  const res = safeDecode(consoleReadSchema, output)
  if (!res) return null
  const req = safeParseInput(readInputSchema, input)
  return (
    <div>
      <MetaRow>
        <StatusPill
          label={`${res.entries.length} entries`}
          variant={res.entries.length > 0 ? 'accent' : 'default'}
        />
        {req?.level ? <Chip>{req.level}</Chip> : null}
        {req?.pattern ? <Chip>/{req.pattern}/</Chip> : null}
        {res.dropped > 0 ? (
          <Chip className="br-ui-chip-warn">{res.dropped} dropped</Chip>
        ) : null}
      </MetaRow>
      {res.entries.length === 0 ? (
        <div className="br-ui-empty-line">· no matching console entries</div>
      ) : (
        <ul className="br-ui-scroll">
          {res.entries.map((entry) => (
            <ConsoleEntryRow key={entry.seq} entry={entry} />
          ))}
        </ul>
      )}
    </div>
  )
}

export function NetworkEntryRow({ entry }: { entry: BrowserNetworkEntry }) {
  return (
    <li className="br-ui-log-row">
      <span
        className={cn('br-ui-net-status', entry.failed && 'is-failed')}
      >
        {entry.status ?? (entry.failed ? 'err' : '...')}
      </span>
      <span className="br-ui-net-method">{entry.method}</span>
      <span className={cn('br-ui-net-url', entry.failed && 'is-failed')}>
        {entry.url}
        {entry.error ? (
          <span className="br-ui-alert"> · {entry.error}</span>
        ) : null}
      </span>
    </li>
  )
}

export function NetworkReadView({
  input,
  output,
}: {
  input: unknown
  output: unknown
}) {
  const res = safeDecode(networkReadSchema, output)
  if (!res) return null
  const req = safeParseInput(readInputSchema, input)
  return (
    <div>
      <MetaRow>
        <StatusPill
          label={`${res.entries.length} requests`}
          variant={res.entries.length > 0 ? 'accent' : 'default'}
        />
        {req?.failed_only ? (
          <Chip className="br-ui-chip-warn">failed only</Chip>
        ) : null}
        {req?.pattern ? <Chip>/{req.pattern}/</Chip> : null}
        {res.dropped > 0 ? (
          <Chip className="br-ui-chip-warn">{res.dropped} dropped</Chip>
        ) : null}
      </MetaRow>
      {res.entries.length === 0 ? (
        <div className="br-ui-empty-line">· no matching requests</div>
      ) : (
        <ul className="br-ui-scroll">
          {res.entries.map((entry) => (
            <NetworkEntryRow key={entry.seq} entry={entry} />
          ))}
        </ul>
      )}
    </div>
  )
}

/* ---------------- styles ---------------- */

export function StylesReadView({ output }: { output: unknown }) {
  const res = safeDecode(stylesReadResultSchema, output)
  if (!res) return null
  return (
    <div>
      <MetaRow>
        <StatusPill
          label={`${res.properties.length} properties`}
          variant="accent"
        />
        <Chip className="br-ui-chip-accent">{res.ref}</Chip>
      </MetaRow>
      <div className="br-ui-scroll">
        <table className="br-ui-vtable">
          <tbody>
            {res.properties.map((prop) => (
              <tr key={prop.name}>
                <td className="br-ui-td br-ui-td-dim br-ui-break br-ui-td-name">
                  {prop.name}
                </td>
                <td className="br-ui-td br-ui-break">{prop.value}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {res.inline_style ? (
        <div className="br-ui-inline-style">style="{res.inline_style}"</div>
      ) : null}
    </div>
  )
}

const stylesWriteInputSchema = z.object({
  ref: z.string().optional(),
  property: z.string().optional(),
  value: z.string().optional(),
})

export function StylesWriteView({
  input,
  output,
}: {
  input: unknown
  output: unknown
}) {
  const res = safeDecode(stylesWriteResultSchema, output)
  if (!res) return null
  const req = safeParseInput(stylesWriteInputSchema, input)
  return (
    <div>
      <MetaRow>
        <StatusPill
          label={res.ok ? 'applied' : 'failed'}
          variant={res.ok ? 'accent' : 'alert'}
        />
        {req?.ref ? <Chip className="br-ui-chip-accent">{req.ref}</Chip> : null}
      </MetaRow>
      {req?.property ? (
        <ActionLine symbol="·" tone="ink">
          {req.property}: {req.value ?? ''}
        </ActionLine>
      ) : null}
      <div className="br-ui-inline-style">style="{res.inline_style}"</div>
    </div>
  )
}

/* ---------------- dom ---------------- */

interface FlatDomRow {
  node: DomNode
  depth: number
}

function flattenDom(node: DomNode, depth: number, out: FlatDomRow[]): void {
  out.push({ node, depth })
  for (const child of node.children) flattenDom(child, depth + 1, out)
}

export function DomReadView({ output }: { output: unknown }) {
  const res = safeDecode(domReadResultSchema, output)
  if (!res) return null
  const rows: FlatDomRow[] = []
  flattenDom(res.root, 0, rows)
  return (
    <div>
      <MetaRow>
        <StatusPill label={`${rows.length} nodes`} variant="accent" />
        {res.truncated ? <StatusPill label="truncated" variant="warn" /> : null}
      </MetaRow>
      <div className="br-ui-dom">
        {rows.map(({ node, depth }) => (
          <div
            key={node.ref}
            className="br-ui-dom-row"
            style={{ paddingLeft: depth * 14 }}
          >
            {node.tag === '#text' ? (
              <span className="br-ui-dom-text">
                "{truncate(node.text ?? '', 80)}"
              </span>
            ) : (
              <span className="br-ui-dom-el">
                {elementLabel(node.tag, node.id, node.classes)}
              </span>
            )}{' '}
            <span className="br-ui-dom-ref">[{node.ref}]</span>
            {node.child_count > node.children.length ? (
              <span className="br-ui-dom-more">
                {' '}
                +{node.child_count - node.children.length} more
              </span>
            ) : null}
          </div>
        ))}
      </div>
    </div>
  )
}

/* ---------------- evaluate ---------------- */

const evaluateInputSchema = z.object({ expression: z.string().optional() })

export function EvaluateView({
  input,
  output,
}: {
  input: unknown
  output: unknown
}) {
  const res = safeDecode(evaluateResultSchema, output)
  if (!res) return null
  const req = safeParseInput(evaluateInputSchema, input)
  return (
    <div>
      <MetaRow>
        <StatusPill
          label={res.ok ? 'ok' : 'exception'}
          variant={res.ok ? 'accent' : 'alert'}
        />
      </MetaRow>
      {req?.expression ? (
        <ActionLine symbol="$" tone="ink">
          <span className="br-ui-break">{truncate(req.expression, 200)}</span>
        </ActionLine>
      ) : null}
      {res.ok ? (
        res.value === undefined ? (
          <div className="br-ui-empty-line">· undefined</div>
        ) : (
          <div className="br-ui-json-sm">
            <JsonHighlight code={JSON.stringify(res.value, null, 2) ?? 'null'} />
          </div>
        )
      ) : (
        <div className="br-ui-eval-err">{res.error ?? 'evaluation failed'}</div>
      )}
    </div>
  )
}
