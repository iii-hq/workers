import { z } from 'zod'
import { renderWithHighlight } from '@/components/chat/sandbox/highlight'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import { Badge } from '@/components/ui/Badge'
import {
  type BrowserConsoleEntry,
  type BrowserNetworkEntry,
  elementLabel,
  formatTime,
  levelBadgeVariant,
} from '@/lib/browser'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
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
    <pre className="m-0 max-h-96 overflow-auto px-3 py-2 font-mono text-[12px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
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
        <span className="break-all">{res.url}</span>
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
        <span className="break-all">{res.url}</span>
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
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · no live sessions
        </div>
      ) : (
        <table className="w-full font-mono text-[11.5px] text-ink">
          <tbody>
            {res.sessions.map((s) => (
              <tr
                key={s.session_id}
                className="border-b border-rule-2 last:border-b-0"
              >
                <td className="px-3 py-1 text-accent whitespace-nowrap">
                  {s.session_id}
                </td>
                <td className="px-3 py-1 text-ink break-all">{s.url}</td>
                <td className="px-3 py-1 text-ink-faint whitespace-nowrap">
                  {s.headless ? 'headless' : 'headful'}
                </td>
                <td className="px-3 py-1 text-ink-faint tabular-nums text-right whitespace-nowrap">
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
        <span className="break-all">{res.url}</span>
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
        {req?.ref ? <Chip className="text-accent">{req.ref}</Chip> : null}
        {req?.key ? <Chip>{req.key}</Chip> : null}
        {req?.x != null && req?.y != null ? (
          <Chip className="tabular-nums">
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
        <span className="break-all">{res.url}</span>
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
    <li className="flex items-start gap-2 border-b border-rule-2 px-3 py-1 font-mono text-[12px] leading-[1.55] last:border-b-0">
      <span className="shrink-0 tabular-nums text-ink-ghost">
        {formatTime(entry.timestamp)}
      </span>
      <Badge
        variant={levelBadgeVariant(entry.level)}
        className="w-[72px] shrink-0"
      >
        {entry.level}
      </Badge>
      <span className="min-w-0 flex-1 whitespace-pre-wrap break-words text-ink">
        {entry.text}
        {entry.source ? (
          <span className="text-ink-ghost"> · {entry.source}</span>
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
          <Chip className="text-warn border-warn/40">
            {res.dropped} dropped
          </Chip>
        ) : null}
      </MetaRow>
      {res.entries.length === 0 ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · no matching console entries
        </div>
      ) : (
        <ul className="max-h-80 overflow-auto">
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
    <li className="flex items-start gap-2 border-b border-rule-2 px-3 py-1 font-mono text-[12px] leading-[1.55] last:border-b-0">
      <span
        className={cn(
          'w-[42px] shrink-0 tabular-nums',
          entry.failed ? 'text-alert' : 'text-ink-faint',
        )}
      >
        {entry.status ?? (entry.failed ? 'err' : '...')}
      </span>
      <span className="w-[56px] shrink-0 text-ink-faint">{entry.method}</span>
      <span
        className={cn(
          'min-w-0 flex-1 break-all',
          entry.failed ? 'text-alert' : 'text-ink',
        )}
      >
        {entry.url}
        {entry.error ? (
          <span className="text-alert"> · {entry.error}</span>
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
          <Chip className="text-warn border-warn/40">failed only</Chip>
        ) : null}
        {req?.pattern ? <Chip>/{req.pattern}/</Chip> : null}
        {res.dropped > 0 ? (
          <Chip className="text-warn border-warn/40">
            {res.dropped} dropped
          </Chip>
        ) : null}
      </MetaRow>
      {res.entries.length === 0 ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · no matching requests
        </div>
      ) : (
        <ul className="max-h-80 overflow-auto">
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
        <Chip className="text-accent">{res.ref}</Chip>
      </MetaRow>
      <div className="max-h-80 overflow-auto">
        <table className="w-full font-mono text-[11.5px]">
          <tbody>
            {res.properties.map((prop) => (
              <tr
                key={prop.name}
                className="border-b border-rule-2 last:border-b-0"
              >
                <td className="w-[45%] px-3 py-1 text-ink-faint break-all">
                  {prop.name}
                </td>
                <td className="px-3 py-1 text-ink break-all">{prop.value}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {res.inline_style ? (
        <div className="border-t border-rule-2 px-3 py-1.5 font-mono text-[11.5px] text-ink-faint break-all">
          style="{res.inline_style}"
        </div>
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
        {req?.ref ? <Chip className="text-accent">{req.ref}</Chip> : null}
      </MetaRow>
      {req?.property ? (
        <ActionLine symbol="·" tone="ink">
          {req.property}: {req.value ?? ''}
        </ActionLine>
      ) : null}
      <div className="px-3 py-1.5 font-mono text-[11.5px] text-ink-faint break-all">
        style="{res.inline_style}"
      </div>
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
      <div className="max-h-96 overflow-auto px-3 py-2 font-mono text-[12px] leading-[1.55]">
        {rows.map(({ node, depth }) => (
          <div
            key={node.ref}
            className="whitespace-nowrap"
            style={{ paddingLeft: depth * 14 }}
          >
            {node.tag === '#text' ? (
              <span className="text-ink-faint">
                "{truncate(node.text ?? '', 80)}"
              </span>
            ) : (
              <span className="text-ink">
                {elementLabel(node.tag, node.id, node.classes)}
              </span>
            )}{' '}
            <span className="text-accent">[{node.ref}]</span>
            {node.child_count > node.children.length ? (
              <span className="text-ink-ghost">
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
          <span className="break-all">{truncate(req.expression, 200)}</span>
        </ActionLine>
      ) : null}
      {res.ok ? (
        res.value === undefined ? (
          <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
            · undefined
          </div>
        ) : (
          <div className="max-h-80 overflow-auto">
            <JsonHighlight
              code={JSON.stringify(res.value, null, 2) ?? 'null'}
            />
          </div>
        )
      ) : (
        <div className="px-3 py-2 font-mono text-[12px] text-alert whitespace-pre-wrap break-words">
          {res.error ?? 'evaluation failed'}
        </div>
      )}
    </div>
  )
}
