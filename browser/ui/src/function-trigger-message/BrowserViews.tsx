import {
  ActionLine,
  Badge,
  Chip,
  EmptyState,
  JsonHighlight,
  MetaRow,
  Table,
  TableBody,
  TableCell,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { ArrowRight, DollarSign, Dot, TriangleAlert } from 'lucide-react'
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
    <pre className="br-ui-text br-ui-scroll">
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
        <Badge variant="accent">snapshot</Badge>
        {res.title ? <Chip>{truncate(res.title, 60)}</Chip> : null}
        {res.truncated ? <Badge variant="warn">truncated</Badge> : null}
      </MetaRow>
      <ActionLine icon={<ArrowRight size={16} aria-hidden />} tone="ink">
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
        <Badge variant="accent">session started</Badge>
        <Chip>{res.session_id}</Chip>
        {res.incognito ? <Chip tone="warning">incognito</Chip> : null}
        <Chip>{res.headless ? 'headless' : 'headful'}</Chip>
      </MetaRow>
      <ActionLine icon={<ArrowRight size={16} aria-hidden />} tone="ink">
        <span className="br-ui-break">{res.url}</span>
      </ActionLine>
      {res.error ? (
        <ActionLine icon={<TriangleAlert size={16} aria-hidden />} tone="warn">
          <span className="br-ui-break">page failed to load: {res.error}</span>
        </ActionLine>
      ) : null}
    </div>
  )
}

export function SessionStopView({ output }: { output: unknown }) {
  const res = safeDecode(sessionStopResultSchema, output)
  if (!res) return null
  return (
    <MetaRow>
      <Badge variant={res.was_running ? 'accent' : 'default'}>{res.was_running ? 'stopped' : 'was not running'}</Badge>
    </MetaRow>
  )
}

export function SessionListView({ output }: { output: unknown }) {
  const res = safeDecode(sessionListResultSchema, output)
  if (!res) return null
  return (
    <div>
      <MetaRow>
        <Badge variant={res.sessions.length > 0 ? 'accent' : 'default'}>{`${res.sessions.length} ${res.sessions.length === 1 ? 'tab' : 'tabs'}`}</Badge>
      </MetaRow>
      {res.sessions.length === 0 ? (
        <EmptyState title="No tabs" description="Nothing is open in this browser." />
      ) : (
        <TableViewport className="br-ui-table">
          <Table density="compact">
            <TableBody>
              {res.sessions.map((s) => (
                <TableRow key={s.session_id}>
                  <TableCell className="br-ui-accent br-ui-nowrap">{s.session_id}</TableCell>
                  <TableCell className="br-ui-break">{s.url}</TableCell>
                  <TableCell className="br-ui-faint br-ui-nowrap">
                    {s.incognito ? 'incognito · ' : ''}
                    {s.active === false ? 'asleep' : s.headless ? 'headless' : 'headful'}
                  </TableCell>
                  <TableCell className="br-ui-faint br-ui-num br-ui-right br-ui-nowrap">
                    {s.console_entries} logs
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </TableViewport>
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
        <Badge variant={res.ok ? 'accent' : 'alert'}>{res.ok ? 'loaded' : 'failed'}</Badge>
        {res.timed_out ? <Badge variant="warn">timed out</Badge> : null}
        {res.title ? <Chip>{truncate(res.title, 60)}</Chip> : null}
      </MetaRow>
      <ActionLine icon={<ArrowRight size={16} aria-hidden />} tone="ink">
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
        <Badge variant={res.ok ? 'accent' : 'alert'}>{res.ok ? 'done' : 'failed'}</Badge>
        {req?.action ? <Chip>{req.action}</Chip> : null}
        {req?.ref ? <Chip tone="accent">{req.ref}</Chip> : null}
        {req?.key ? <Chip>{req.key}</Chip> : null}
        {req?.x != null && req?.y != null ? (
          <Chip className="br-ui-num">
            {Math.round(req.x)},{Math.round(req.y)}
          </Chip>
        ) : null}
      </MetaRow>
      <ActionLine icon={<Dot size={16} aria-hidden />} tone="ink">
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
        <Badge variant={res.ok ? 'accent' : 'alert'}>{req?.action ?? 'history'}</Badge>
        {!res.moved ? (
          <Badge variant="warn">no history entry</Badge>
        ) : null}
      </MetaRow>
      <ActionLine icon={<ArrowRight size={16} aria-hidden />} tone="ink">
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
    <li className="br-ui-row">
      <span className="br-ui-num br-ui-dim">{formatTime(entry.timestamp)}</span>
      <Badge
        variant={levelBadgeVariant(entry.level)}
        className="br-ui-log-level"
      >
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
        <Badge variant={res.entries.length > 0 ? 'accent' : 'default'}>{`${res.entries.length} entries`}</Badge>
        {req?.level ? <Chip>{req.level}</Chip> : null}
        {req?.pattern ? <Chip>/{req.pattern}/</Chip> : null}
        {res.dropped > 0 ? (
          <Chip tone="warning">{res.dropped} dropped</Chip>
        ) : null}
      </MetaRow>
      {res.entries.length === 0 ? (
        <EmptyState
          title="No console entries"
          description="Nothing the page logged matches this read."
        />
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
    <li className="br-ui-row">
      <span className={cn('br-ui-net-status br-ui-num', entry.failed ? 'br-ui-alert' : 'br-ui-faint')}>
        {entry.status ?? (entry.failed ? 'err' : '...')}
      </span>
      <span className="br-ui-net-method br-ui-faint">{entry.method}</span>
      <span className={cn('br-ui-break', entry.failed && 'br-ui-alert')}>
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
        <Badge variant={res.entries.length > 0 ? 'accent' : 'default'}>{`${res.entries.length} requests`}</Badge>
        {req?.failed_only ? (
          <Chip tone="warning">Failed only</Chip>
        ) : null}
        {req?.pattern ? <Chip>/{req.pattern}/</Chip> : null}
        {res.dropped > 0 ? (
          <Chip tone="warning">{res.dropped} dropped</Chip>
        ) : null}
      </MetaRow>
      {res.entries.length === 0 ? (
        <EmptyState
          title="No requests"
          description="Nothing the page requested matches this read."
        />
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
        <Badge variant="accent">{`${res.properties.length} properties`}</Badge>
        <Chip tone="accent">{res.ref}</Chip>
      </MetaRow>
      <TableViewport className="br-ui-table br-ui-scroll">
        <Table density="compact">
          <TableBody>
            {res.properties.map((prop) => (
              <TableRow key={prop.name}>
                <TableCell className="br-ui-faint br-ui-break br-ui-td-name">{prop.name}</TableCell>
                <TableCell className="br-ui-break">{prop.value}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableViewport>
      {res.inline_style ? (
        <div className="br-ui-text br-ui-faint">style="{res.inline_style}"</div>
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
        <Badge variant={res.ok ? 'accent' : 'alert'}>{res.ok ? 'applied' : 'failed'}</Badge>
        {req?.ref ? <Chip tone="accent">{req.ref}</Chip> : null}
      </MetaRow>
      {req?.property ? (
        <ActionLine icon={<Dot size={16} aria-hidden />} tone="ink">
          {req.property}: {req.value ?? ''}
        </ActionLine>
      ) : null}
      <div className="br-ui-text br-ui-faint">style="{res.inline_style}"</div>
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
        <Badge variant="accent">{`${rows.length} nodes`}</Badge>
        {res.truncated ? <Badge variant="warn">truncated</Badge> : null}
      </MetaRow>
      <div className="br-ui-text br-ui-scroll">
        {rows.map(({ node, depth }) => (
          <div
            key={node.ref}
            className="br-ui-dom-row"
            style={{ paddingLeft: depth * 14 }}
          >
            {node.tag === '#text' ? (
              <span className="br-ui-faint">
                "{truncate(node.text ?? '', 80)}"
              </span>
            ) : (
              <span className="br-ui-ink">
                {elementLabel(node.tag, node.id, node.classes)}
              </span>
            )}{' '}
            <span className="br-ui-accent">[{node.ref}]</span>
            {node.child_count > node.children.length ? (
              <span className="br-ui-dim">
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
        <Badge variant={res.ok ? 'accent' : 'alert'}>{res.ok ? 'ok' : 'exception'}</Badge>
      </MetaRow>
      {req?.expression ? (
        <ActionLine icon={<DollarSign size={16} aria-hidden />} tone="ink">
          <span className="br-ui-break">{truncate(req.expression, 200)}</span>
        </ActionLine>
      ) : null}
      {res.ok ? (
        res.value === undefined ? (
          <div className="br-ui-json-sm">
            <JsonHighlight code="undefined" />
          </div>
        ) : (
          <div className="br-ui-json-sm">
            <JsonHighlight
              code={JSON.stringify(res.value, null, 2) ?? 'null'}
            />
          </div>
        )
      ) : (
        <div className="br-ui-text br-ui-alert">{res.error ?? 'evaluation failed'}</div>
      )}
    </div>
  )
}
