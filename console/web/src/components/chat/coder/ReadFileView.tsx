/**
 * `coder::read-file` — single-path scalars vs batch `paths[]`, full reads
 * vs `line_from`/`line_to` windows, stat probes, and pre-numbered content.
 *
 * Wire: workers/coder/src/functions/read_file.rs (v0.4.1). Single-path
 * responses populate the top-level scalars and omit `results`; batch
 * responses populate only `results` (request order). `total_lines`
 * ABSENT means "not fully traversed" — distinct from 0.
 */
import { TriangleAlert } from 'lucide-react'
import {
  formatBytes,
  formatMode,
  formatMtime,
  inferLangFromPath,
} from '@/components/chat/sandbox/format'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import { CodeHighlight } from '@/lib/syntax'
import {
  type ReadEntryResult,
  type ReadFileRequest,
  type ReadFileResponse,
  type ReadTarget,
  readFileRequestSchema,
  readFileResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface ReadFileViewProps {
  input: unknown
  output?: unknown
  running?: boolean
}

/* ---------------- pure derivation helpers (unit-tested) ---------------- */

/** Request-side options for one read target — collapses the batch
    string/object shorthand and nullish wire fields into one shape. */
export interface NormalizedReadTarget {
  path: string
  lineFrom: number | null
  lineTo: number | null
  numbered: boolean
  stat: boolean
}

export function normalizeReadTarget(target: ReadTarget): NormalizedReadTarget {
  if (typeof target === 'string') {
    // Bare string = whole-file read with handler defaults.
    return {
      path: target,
      lineFrom: null,
      lineTo: null,
      numbered: false,
      stat: false,
    }
  }
  return {
    path: target.path,
    lineFrom: target.line_from ?? null,
    lineTo: target.line_to ?? null,
    numbered: target.numbered ?? false,
    stat: target.stat ?? false,
  }
}

/**
 * One renderable unit: requested target + its wire result. Single-path
 * responses are folded into a synthetic `ReadEntryResult` so both modes
 * share a renderer. `result` stays null while running or when the
 * response is missing/unparseable.
 */
export interface ReadEntryModel {
  requested: NormalizedReadTarget
  result: ReadEntryResult | null
}

export function isBatchRequest(req: ReadFileRequest): boolean {
  return Array.isArray(req.paths)
}

/** Stable list key — the same path may appear twice with different
    windows, so the key carries the whole target descriptor. */
export function readTargetKey(t: NormalizedReadTarget): string {
  return [
    t.path,
    t.lineFrom ?? '',
    t.lineTo ?? '',
    t.numbered ? 'n' : '',
    t.stat ? 's' : '',
  ].join(':')
}

export function deriveReadEntries(
  req: ReadFileRequest,
  resp: ReadFileResponse | null,
): ReadEntryModel[] {
  if (isBatchRequest(req)) {
    // Results arrive in request order — align by index. Per-entry window
    // flags live on the targets; top-level window fields are ignored.
    return (req.paths ?? []).map((target, i) => ({
      requested: normalizeReadTarget(target),
      result: resp?.results?.[i] ?? null,
    }))
  }
  const path = req.path ?? resp?.path
  // Neither path nor paths — runtime rejects with C210; nothing to render.
  if (path == null) return []
  const result: ReadEntryResult | null = resp
    ? {
        path: resp.path ?? path,
        // Single-path failures arrive as top-level handler errors (the
        // family error view renders those) — a parsed response = success.
        success: true,
        content: resp.content,
        is_utf8: resp.is_utf8,
        lines_returned: resp.lines_returned,
        total_lines: resp.total_lines,
        more_lines: resp.more_lines,
        size: resp.size,
        mode: resp.mode,
        mtime: resp.mtime,
      }
    : null
  return [
    {
      requested: {
        path,
        lineFrom: req.line_from ?? null,
        lineTo: req.line_to ?? null,
        numbered: req.numbered ?? false,
        stat: req.stat ?? false,
      },
      result,
    },
  ]
}

/** `"L10–50"` / `"L40–EOF"`. The wire defaults `line_from` to 1 when only
    `line_to` is set. Null when the read isn't windowed. */
export function windowLabel(
  lineFrom: number | null,
  lineTo: number | null,
): string | null {
  if (lineFrom == null && lineTo == null) return null
  const from = lineFrom ?? 1
  return lineTo == null ? `L${from}–EOF` : `L${from}–${lineTo}`
}

/** First line of the follow-up window when `more_lines` is true. */
export function nextWindowStart(
  lineFrom: number | null,
  linesReturned: number | null | undefined,
): number {
  return (lineFrom ?? 1) + (linesReturned ?? 0)
}

/** Numeric st_mode (lower 9 bits, e.g. 420 = 0o644) → `"rw-r--r--"`.
    The sandbox `formatMode` takes octal STRINGS; coder sends integers. */
export function formatNumericMode(mode: number): string {
  if (!Number.isInteger(mode) || mode < 0) return '—'
  return formatMode((mode & 0o777).toString(8).padStart(3, '0'))
}

/* ---------------- chip rows ---------------- */

function RequestChips({ t }: { t: NormalizedReadTarget }) {
  const window = windowLabel(t.lineFrom, t.lineTo)
  return (
    <>
      {window ? <Chip label="window">{window}</Chip> : null}
      {t.numbered ? <Chip label="numbered">true</Chip> : null}
      {t.stat ? <Chip label="stat">true</Chip> : null}
    </>
  )
}

/** size / lines / lossy badge for a successful read. `size` is the FILE
    size from metadata — labelled apart from content size when windowed. */
function MetaChips({ entry }: { entry: ReadEntryModel }) {
  const { requested, result } = entry
  if (!result?.success || requested.stat) return null
  const windowed = requested.lineFrom != null || requested.lineTo != null
  return (
    <>
      {result.size != null ? (
        <Chip label={windowed ? 'file size' : 'size'}>
          {formatBytes(result.size)}
        </Chip>
      ) : null}
      {result.total_lines != null ? (
        <Chip label="lines">{result.total_lines}</Chip>
      ) : null}
      {result.is_utf8 === false ? (
        // Binary bytes were replaced with U+FFFD in the returned content.
        <Chip label="utf8" className="border-warn text-warn">
          lossy
        </Chip>
      ) : null}
    </>
  )
}

/** `more_lines: true` — content beyond the returned window exists. Offer
    the next `line_from` so the follow-up window is one copy away. */
function MoreLinesPill({ entry }: { entry: ReadEntryModel }) {
  const r = entry.result
  if (r?.more_lines !== true) return null
  return (
    <span className="inline-flex items-center gap-1.5">
      <FooterPill tone="warn">file continues</FooterPill>
      <span className="font-mono text-[11px] text-ink-faint tabular-nums">
        next window from L
        {nextWindowStart(entry.requested.lineFrom, r.lines_returned)}
      </span>
    </span>
  )
}

/* ---------------- bodies ---------------- */

/** Verbatim wire message — C213 budget errors name the budget, the bytes
    consumed, and the recovery call; never paraphrase. */
function EntryErrorBlock({ error }: { error: ReadEntryResult['error'] }) {
  return (
    <div className="px-3 py-2 font-mono text-[12px] text-warn break-words">
      {error ? error.message : 'read failed'}
    </div>
  )
}

/** Metadata-only probe (`stat: true`) — compact chip grid like FsStatView.
    `total_lines` / `is_utf8` are absent for files over max_read_bytes;
    the stat itself still SUCCEEDED. */
function StatBody({ result }: { result: ReadEntryResult }) {
  return (
    <div className="px-3 py-2 flex flex-wrap items-center gap-1.5">
      <Chip label="size">
        {result.size != null ? formatBytes(result.size) : '—'}
      </Chip>
      <Chip label="mode">
        {result.mode != null ? formatNumericMode(result.mode) : '—'}
      </Chip>
      <Chip label="mtime">
        {result.mtime != null ? formatMtime(result.mtime) : '—'}
      </Chip>
      {result.total_lines != null ? (
        <Chip label="lines">{result.total_lines}</Chip>
      ) : null}
      {result.is_utf8 != null ? (
        <Chip
          label="utf8"
          className={result.is_utf8 ? undefined : 'border-warn text-warn'}
        >
          {result.is_utf8 ? 'lossless' : 'lossy'}
        </Chip>
      ) : null}
      {result.total_lines == null ? (
        <span className="font-mono text-[11px] text-ink-ghost">
          · too large to line-count
        </span>
      ) : null}
    </div>
  )
}

/** File content. Numbered reads arrive with literal `N→` prefixes baked
    into each line (absolute numbers that feed update-file line ops), so
    they pass through a plain `<pre>` instead of the highlighter — never
    double-number. */
function ContentBody({ entry }: { entry: ReadEntryModel }) {
  const r = entry.result
  if (!r?.success) return null
  if (entry.requested.stat) return <StatBody result={r} />
  const content = typeof r.content === 'string' ? r.content : null
  if (content === null) {
    return (
      <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
        · no content
      </div>
    )
  }
  if (content === '') {
    // Window past EOF / empty file — both succeed with empty content.
    return (
      <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
        · empty
      </div>
    )
  }
  const lang = entry.requested.numbered
    ? null
    : inferLangFromPath(entry.requested.path)
  if (lang) return <CodeHighlight code={content} language={lang} wrap />
  return (
    <pre className="bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
      <code>{content}</code>
    </pre>
  )
}

/* ---------------- modes ---------------- */

function SingleRead({
  entry,
  budget,
  running,
}: {
  entry: ReadEntryModel
  budget: number | null
  running?: boolean
}) {
  const r = entry.result
  const hasFooter =
    r?.success === true &&
    !entry.requested.stat &&
    (r.size != null ||
      r.mode != null ||
      r.mtime != null ||
      r.total_lines != null ||
      r.is_utf8 === false ||
      r.more_lines === true)

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          file
        </span>
        <code className="font-mono text-[12px] text-ink">
          {entry.requested.path}
        </code>
        <RequestChips t={entry.requested} />
        {budget != null ? (
          // Per-call max_output_bytes override on full reads.
          <Chip label="budget">{formatBytes(budget)}</Chip>
        ) : null}
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · reading…
          </span>
        ) : null}
      </div>

      <ContentBody entry={entry} />

      {hasFooter && r ? (
        <div className="bg-paper-2 border-t border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-1.5">
          <MetaChips entry={entry} />
          {r.mode != null ? (
            <Chip label="mode">{formatNumericMode(r.mode)}</Chip>
          ) : null}
          {r.mtime != null ? (
            <Chip label="mtime">{formatMtime(r.mtime)}</Chip>
          ) : null}
          <MoreLinesPill entry={entry} />
        </div>
      ) : null}
    </div>
  )
}

/** Batch budget exhaustion is a per-entry C213 — earlier entries may have
    succeeded while later ones failed individually. Render each honestly. */
function BatchEntry({
  entry,
  running,
}: {
  entry: ReadEntryModel
  running?: boolean
}) {
  const { requested, result } = entry
  const failed = result != null && !result.success
  return (
    <div className="border-b border-rule-2 last:border-b-0">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-[12px] text-ink">{requested.path}</span>
        <RequestChips t={requested} />
        <MetaChips entry={entry} />
        {failed ? (
          <span className="inline-flex items-center gap-1 font-mono text-[11px] text-warn">
            <TriangleAlert aria-hidden className="w-3.5 h-3.5" />
            {result.error?.code ?? 'err'}
          </span>
        ) : null}
        {result == null && !running ? (
          <span className="font-mono text-[11px] text-ink-faint">—</span>
        ) : null}
      </div>
      {failed ? (
        <EntryErrorBlock error={result.error} />
      ) : (
        <ContentBody entry={entry} />
      )}
      {entry.result?.more_lines === true ? (
        <div className="bg-paper-2 border-t border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-1.5">
          <MoreLinesPill entry={entry} />
        </div>
      ) : null}
    </div>
  )
}

function BatchRead({
  entries,
  running,
}: {
  entries: ReadEntryModel[]
  running?: boolean
}) {
  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="files">{entries.length}</Chip>
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · reading…
          </span>
        ) : null}
      </div>
      {entries.map((entry) => (
        <BatchEntry
          key={readTargetKey(entry.requested)}
          entry={entry}
          running={running}
        />
      ))}
    </div>
  )
}

/* ---------------- view ---------------- */

export function ReadFileView({ input, output, running }: ReadFileViewProps) {
  const req = safeParseRequest(readFileRequestSchema, input)
  if (!req) return null
  const resp =
    output != null ? safeParseResponse(readFileResponseSchema, output) : null
  const entries = deriveReadEntries(req, resp)
  if (entries.length === 0) return null

  if (isBatchRequest(req)) {
    return <BatchRead entries={entries} running={running} />
  }
  return (
    <SingleRead
      entry={entries[0]}
      budget={req.max_output_bytes ?? null}
      running={running}
    />
  )
}
