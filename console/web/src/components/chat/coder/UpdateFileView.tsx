/**
 * `coder::update-file` — per-op echo rendering (v0.4.1 wire).
 *
 * The before/after full-file diff is GONE from the wire: each applied op
 * returns a bounded post-apply snapshot (`OpEcho`) instead. Done state
 * renders one section per file (path + ops/applied/lines chips) followed
 * by echo blocks keyed by op_index; replace ops emit up to 5 site echoes
 * sharing one op_index (`total_replacements` is the only record of the
 * extras). The op-summary table remains for pending/running and as the
 * fallback when the response is missing or unparseable. Per-file
 * atomicity: a failed file carries empty echoes and only `error` —
 * sibling files in the batch still apply, so batches render per file.
 *
 * Echoes render "half-diff" styled: lines the op INSERTED are tagged added
 * (green `+` gutter); old text is not on the wire, so update_lines/remove
 * ops get a single amber (`text-warn`) `−` stub at the seam instead of
 * real deletion rows. Replace-site echoes are entirely post-replace text —
 * every line is added. Addition ranges anchor on POST-APPLY coordinates
 * reconstructed from the echo itself, falling back to mapping the
 * request's own line-op deltas when the echo head-clamps (see
 * `postRegionFirst`), so multi-op batches that shift later regions still
 * tag the right lines.
 */
import { TriangleAlert } from 'lucide-react'
import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { cn } from '@/lib/utils'
import { ChecksList } from './ChecksView'
import {
  formatUpdateOp,
  type OpEcho,
  safeParseRequest,
  safeParseResponse,
  truncateInline,
  type UpdateFileRequest,
  type UpdateFileResult,
  type UpdateFileSpec,
  type UpdateOp,
  updateFileRequestSchema,
  updateFileResponseSchema,
} from './parsers'

interface UpdateFileViewProps {
  input: unknown
  output?: unknown
  running?: boolean
  preview?: boolean
}

/* ---------------- pure echo derivation (unit-tested) ---------------- */

export interface EchoGroup {
  opIndex: number
  echoes: OpEcho[]
}

/**
 * Group echoes by `op_index`, preserving wire order (`build_echoes` emits
 * them sorted by op_index; replace sites keep match order). Replace ops
 * are the only multi-echo producers — every echo in a >1 group is a
 * separate match site of the same op.
 */
export function groupEchoesByOp(echoes: readonly OpEcho[]): EchoGroup[] {
  const groups: EchoGroup[] = []
  const byOpIndex = new Map<number, OpEcho[]>()
  for (const echo of echoes) {
    const bucket = byOpIndex.get(echo.op_index)
    if (bucket) {
      bucket.push(echo)
    } else {
      const fresh = [echo]
      byOpIndex.set(echo.op_index, fresh)
      groups.push({ opIndex: echo.op_index, echoes: fresh })
    }
  }
  return groups
}

export type EchoRow =
  | { kind: 'line'; lineNo: number; text: string; added: boolean }
  | { kind: 'elision'; count: number }
  | { kind: 'stub'; verb: 'replaced' | 'removed'; label: string }

/**
 * Mirror of `update_file.rs::split_content` (Rust `str::lines()`): a
 * trailing `\n` does NOT produce a final empty line, `""` yields ZERO
 * lines, and CRLF `\r` is trimmed without affecting the count. This is
 * the K in the addition range [anchor, anchor+K-1] — drift here mistags
 * the boundary context lines.
 */
export function contentLineCount(content: string): number {
  if (content === '') return 0
  const parts = content.split('\n')
  return parts[parts.length - 1] === '' ? parts.length - 1 : parts.length
}

/** Mirror of `update_file.rs::ECHO_CONTEXT` — context lines above/below a
    line op's region; load-bearing for `postRegionFirst` reconstruction. */
const ECHO_CONTEXT = 2

/** Mirror of `update_file.rs::anchor` — a line op's first affected
    ORIGINAL line (the bottom-up application sort key). Replace ops have
    no line anchor. */
function lineOpAnchor(op: UpdateOp): number | null {
  switch (op.op) {
    case 'insert':
      return op.at_line
    case 'remove':
    case 'update_lines':
      return op.from_line
    case 'replace':
      return null
  }
}

/** Mirror of the `MutationEvent` delta a line op records
    (`record_line_op_events`): the line-count change it causes. Replace
    deltas depend on server-side match positions — not computable here. */
function lineOpDelta(op: UpdateOp): number | null {
  switch (op.op) {
    case 'insert':
      return contentLineCount(op.content)
    case 'remove':
      return -(op.to_line - op.from_line + 1)
    case 'update_lines':
      return contentLineCount(op.content) - (op.to_line - op.from_line + 1)
    case 'replace':
      return null
  }
}

/**
 * POST-APPLY first line of the op's echo region, reconstructed from the
 * wire. `build_line_echo` sets `from_line = post_first − ECHO_CONTEXT`
 * whenever the subtraction doesn't clamp at the file head, so for
 * `from_line > 1` the anchor is exact even when other ops in a multi-op
 * file shifted this region (Rust maps every anchor through later mutation
 * events — `map_through_events`). At `from_line === 1` the head clamp
 * erases the offset (post_first ∈ {1..ECHO_CONTEXT+1}); the wire alone
 * cannot disambiguate, but the REQUEST can when the file's batch is
 * line-op-only: line ops apply bottom-up and overlap validation keeps
 * their covers disjoint, so `map_through_events` collapses to a plain
 * delta sum — only siblings with a strictly SMALLER anchor apply after
 * this op and shift its region (strictness also excludes the op itself;
 * anchors are distinct because each lies in its op's disjoint cover).
 * Batches containing a `replace` keep the ORIGINAL-anchor fallback:
 * regex ops run after all line ops but their match positions (and thus
 * deltas) exist only server-side — accepted residual: a newline-count-
 * changing replacement above a head-of-file region can mistag at most
 * the clamp window (pinned in tests). Callers without the request's op
 * list also degrade to the original anchor.
 */
function postRegionFirst(
  echo: OpEcho,
  originalFirst: number,
  op: UpdateOp,
  fileOps?: readonly UpdateOp[],
): number {
  if (echo.from_line > 1) return echo.from_line + ECHO_CONTEXT
  const ownAnchor = lineOpAnchor(op)
  if (
    ownAnchor === null ||
    !fileOps ||
    fileOps.some((o) => o.op === 'replace')
  ) {
    return originalFirst
  }
  return fileOps.reduce((mapped, sibling) => {
    const anchor = lineOpAnchor(sibling)
    const delta = lineOpDelta(sibling)
    return anchor !== null && delta !== null && anchor < ownAnchor
      ? mapped + delta
      : mapped
  }, originalFirst)
}

/**
 * Which absolute (post-apply) line numbers did this op INSERT? The range
 * start comes from `postRegionFirst` (post-apply coordinates), the length
 * K from the op's content. Replace-site echoes carry zero context
 * (`build_site_echo`), so every echoed line is post-replace text.
 */
function addedLinePredicate(
  echo: OpEcho,
  op?: UpdateOp,
  fileOps?: readonly UpdateOp[],
): (lineNo: number) => boolean {
  if (!op) return () => false
  switch (op.op) {
    case 'replace':
      return () => true
    case 'insert': {
      const first = postRegionFirst(echo, op.at_line, op, fileOps)
      const last = first + contentLineCount(op.content) - 1
      return (lineNo) => lineNo >= first && lineNo <= last
    }
    case 'update_lines': {
      const first = postRegionFirst(echo, op.from_line, op, fileOps)
      const last = first + contentLineCount(op.content) - 1
      return (lineNo) => lineNo >= first && lineNo <= last
    }
    case 'remove':
      return () => false
  }
}

/**
 * Deletion stub for ops that destroyed original text: the old lines are
 * not on the wire, so a single `−` row stands in for them. insert is a
 * pure addition and replace's old text is carried by the op tooltip
 * (`formatUpdateOp`) — neither gets a stub. The LABEL keeps original
 * coordinates (it names destroyed original lines); the SEAM is post-apply:
 * update_lines anchors where its additions begin, remove one past its
 * recorded anchor (`record_line_op_events` anchors remove at
 * `from_line − 1`, the surviving line above the cut).
 */
function deletionStub(
  echo: OpEcho,
  op?: UpdateOp,
  fileOps?: readonly UpdateOp[],
): { row: EchoRow; anchorLine: number } | null {
  if (!op || (op.op !== 'update_lines' && op.op !== 'remove')) return null
  const verb = op.op === 'update_lines' ? 'replaced' : 'removed'
  const range =
    op.from_line === op.to_line
      ? `L${op.from_line}`
      : `L${op.from_line}–L${op.to_line}`
  const anchorLine =
    op.op === 'update_lines'
      ? postRegionFirst(echo, op.from_line, op, fileOps)
      : postRegionFirst(echo, op.from_line - 1, op, fileOps) + 1
  return {
    row: { kind: 'stub', verb, label: `${verb} original ${range}` },
    anchorLine,
  }
}

/** Place the stub at the diff seam: before the first row at/after the
    post-apply seam line (i.e. between leading context and the additions). */
function insertStub(
  rows: readonly EchoRow[],
  stub: EchoRow,
  anchorLine: number,
): EchoRow[] {
  const idx = rows.findIndex((r) => r.kind === 'line' && r.lineNo >= anchorLine)
  if (idx < 0) return [...rows, stub]
  return [...rows.slice(0, idx), stub, ...rows.slice(idx)]
}

/**
 * Numbered rows for one echo, with the elision divider placed where the
 * wire cut the middle. `update_file.rs` always elides symmetrically: line
 * ops keep the first/last ECHO_HEAD_TAIL (8+8) lines (`build_line_echo`),
 * replace sites keep exactly the region's first and last line
 * (`build_site_echo`) — so the divider sits after ceil(lines/2) and tail
 * numbering resumes at `from_line + head + elided`. When `op` is given,
 * rows inside the op's POST-APPLY addition range (`postRegionFirst`) are
 * tagged `added` (the range may span the elided middle — head and tail
 * rows tag independently) and update_lines/remove contribute a deletion
 * stub at the seam. `fileOps` (the file's FULL op batch from the request)
 * lets head-clamped echoes map the anchor through sibling line-op deltas
 * instead of falling back to original coordinates.
 */
export function echoRows(
  echo: OpEcho,
  op?: UpdateOp,
  fileOps?: readonly UpdateOp[],
): EchoRow[] {
  const isAdded = addedLinePredicate(echo, op, fileOps)
  const lineRow = (text: string, lineNo: number): EchoRow => ({
    kind: 'line' as const,
    lineNo,
    text,
    added: isAdded(lineNo),
  })

  const elided = echo.elided ?? 0
  let rows: EchoRow[]
  if (elided <= 0) {
    rows = echo.lines.map((text, i) => lineRow(text, echo.from_line + i))
  } else {
    const headLen = Math.ceil(echo.lines.length / 2)
    const head = echo.lines
      .slice(0, headLen)
      .map((text, i) => lineRow(text, echo.from_line + i))
    const tail = echo.lines
      .slice(headLen)
      .map((text, i) => lineRow(text, echo.from_line + headLen + elided + i))
    rows = [...head, { kind: 'elision' as const, count: elided }, ...tail]
  }

  const stub = deletionStub(echo, op, fileOps)
  return stub ? insertStub(rows, stub.row, stub.anchorLine) : rows
}

/* ---------------- rendering ---------------- */

function EchoLines({
  echo,
  op,
  fileOps,
}: {
  echo: OpEcho
  op?: UpdateOp
  fileOps?: readonly UpdateOp[]
}) {
  const rows = echoRows(echo, op, fileOps)
  if (rows.length === 0) {
    return (
      <div className="px-3 font-mono text-[12px] text-ink-ghost">
        · empty echo
      </div>
    )
  }
  return (
    <div className="font-mono text-[12px] leading-[1.55]">
      {rows.map((row) => {
        if (row.kind === 'elision') {
          return (
            /* At most one elision divider per echo — the key is unique. */
            <div key="elision" className="px-3 flex text-ink-ghost select-none">
              <span className="w-4 shrink-0" aria-hidden />
              <span className="w-12 shrink-0 pr-2 text-right">⋯</span>
              <span className="italic">
                {row.count} {row.count === 1 ? 'line' : 'lines'} elided
              </span>
            </div>
          )
        }
        if (row.kind === 'stub') {
          return (
            /* At most one deletion stub per echo — the key is unique. */
            <div key="stub" className="px-3 flex text-warn">
              <span className="w-4 shrink-0 select-none">−</span>
              <span className="w-12 shrink-0" aria-hidden />
              <span className="break-all">
                {row.label}
                {row.verb === 'replaced' ? (
                  <span className="text-ink-ghost"> · old text not echoed</span>
                ) : null}
              </span>
            </div>
          )
        }
        return (
          <div
            key={row.lineNo}
            className={cn('px-3 flex', row.added && 'bg-ok/5')}
          >
            <span
              className={cn(
                'w-4 shrink-0 select-none',
                row.added ? 'text-ok' : 'text-ink-ghost',
              )}
            >
              {row.added ? '+' : ''}
            </span>
            <span className="w-12 shrink-0 pr-2 text-right tabular-nums text-ink-ghost select-none">
              {row.lineNo}
            </span>
            <span
              className={cn(
                'whitespace-pre-wrap break-all',
                row.added ? 'text-ok' : 'text-ink',
              )}
            >
              {row.text}
            </span>
          </div>
        )
      })}
    </div>
  )
}

function EchoGroupBlock({
  group,
  op,
  fileOps,
}: {
  group: EchoGroup
  op?: UpdateOp
  fileOps?: readonly UpdateOp[]
}) {
  // total_replacements is duplicated on each site of the op.
  const total = group.echoes[0]?.total_replacements
  const extra =
    typeof total === 'number' && total > group.echoes.length
      ? total - group.echoes.length
      : 0

  return (
    <div className="border-b border-rule-2 last:border-b-0 py-1.5 space-y-1">
      <div className="px-3 flex flex-wrap items-center gap-1.5">
        {op ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint cursor-help">
                op {group.opIndex}
              </span>
            </TooltipTrigger>
            <TooltipContent className="max-w-[64ch] break-words">
              {formatUpdateOp(op)}
            </TooltipContent>
          </Tooltip>
        ) : (
          <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            op {group.opIndex}
          </span>
        )}
      </div>
      {group.echoes.map((echo, i) => (
        <div
          /* Two matches on the same line (e.g. replacing `foo` in
             `foo(foo)`) echo identical from_line AND lines, so content
             keys collide — the site ordinal is unique by construction. */
          // biome-ignore lint/suspicious/noArrayIndexKey: echoes are a static wire snapshot; sites never reorder or get removed
          key={`${echo.from_line}:${i}`}
          className="space-y-0.5"
        >
          {echo.total_replacements != null ? (
            <div className="px-3">
              <Chip label="site">{echo.total_replacements} replaced</Chip>
            </div>
          ) : null}
          <EchoLines echo={echo} op={op} fileOps={fileOps} />
        </div>
      ))}
      {extra > 0 ? (
        // Sites are capped at 5 — extras have NO echo, only this count.
        <div className="px-3 font-mono text-[11px] text-ink-ghost">
          · {extra} more {extra === 1 ? 'replacement' : 'replacements'} not
          echoed
        </div>
      ) : null}
    </div>
  )
}

/** Failed file: echoes is [] and only `error` is populated; applied /
    new_line_count are on the wire but meaningless — grayed out. The
    prescriptive message renders in full (never swallow agent guidance). */
function FileFailureRow({ result }: { result: UpdateFileResult }) {
  return (
    <div className="border-b border-rule-2 last:border-b-0 px-3 py-2 font-mono text-[12px]">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-ink">{result.path}</span>
        {result.error ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="inline-flex items-center gap-1 text-warn cursor-help">
                <TriangleAlert aria-hidden className="w-3.5 h-3.5" />
                <Chip className="border-warn text-warn">
                  {result.error.code}
                </Chip>
              </span>
            </TooltipTrigger>
            <TooltipContent className="max-w-[64ch] break-words">
              {result.error.message}
            </TooltipContent>
          </Tooltip>
        ) : (
          <span className="text-warn">err</span>
        )}
        <Chip label="applied" className="text-ink-ghost">
          {result.applied}
        </Chip>
        <Chip label="lines" className="text-ink-ghost">
          {result.new_line_count}
        </Chip>
      </div>
      {result.error ? (
        <div className="mt-1 font-mono text-[11px] text-warn break-words">
          {result.error.message}
        </div>
      ) : null}
    </div>
  )
}

function FileEchoSection({
  file,
  result,
}: {
  file: UpdateFileSpec
  result: UpdateFileResult
}) {
  if (!result.success) return <FileFailureRow result={result} />
  const groups = groupEchoesByOp(result.echoes)

  return (
    <div className="border-b border-rule-2 last:border-b-0">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-1.5">
        {/* Canonical absolute path from the result, not the request. */}
        <span className="font-mono text-[12px] text-ink">{result.path}</span>
        <Chip label="ops">{file.ops.length}</Chip>
        <Chip label="applied">{result.applied}</Chip>
        <Chip label="lines">{result.new_line_count}</Chip>
        {result.echoes_truncated ? (
          <Chip label="echoes" className="border-warn text-warn">
            truncated
          </Chip>
        ) : null}
      </div>
      {groups.length > 0 ? (
        groups.map((group) => (
          <EchoGroupBlock
            key={group.opIndex}
            group={group}
            op={file.ops[group.opIndex]}
            fileOps={file.ops}
          />
        ))
      ) : (
        <div className="px-3 py-2 font-mono text-[12px] text-ink-ghost">
          · no echoes
        </div>
      )}
      {result.echoes_truncated ? (
        <div className="bg-paper-2 border-t border-rule-2 px-3 py-1.5 font-mono text-[11px] text-warn">
          ⋯ echo budget (~4 KiB) exhausted before all ops — coder::read-file to
          inspect
        </div>
      ) : null}
    </div>
  )
}

/**
 * Request-side op summary — pending approval, running, and the fallback
 * when the response is missing or unparseable.
 */
function OpSummaryTable({ req }: { req: UpdateFileRequest }) {
  return (
    <table className="w-full font-mono text-[12px] text-ink">
      <thead>
        <tr className="border-b border-rule-2 text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          <th className="text-left font-normal px-3 py-1.5">path</th>
          <th className="text-left font-normal px-3 py-1.5">ops</th>
        </tr>
      </thead>
      <tbody>
        {req.files.map((file) => {
          const opSummary = file.ops
            .map((op) => {
              const head = formatUpdateOp(op)
              if (op.op === 'insert' || op.op === 'update_lines') {
                return `${head} · ${truncateInline(op.content)}`
              }
              return head
            })
            .join('; ')

          return (
            <tr
              key={file.path}
              className="border-b border-rule-2 last:border-b-0 align-top"
            >
              <td className="px-3 py-1.5 text-ink whitespace-nowrap">
                {file.path}
              </td>
              <td className="px-3 py-1.5 text-ink-faint break-all">
                {opSummary}
              </td>
            </tr>
          )
        })}
      </tbody>
    </table>
  )
}

export function UpdateFileView({
  input,
  output,
  running,
  preview,
}: UpdateFileViewProps) {
  const req = safeParseRequest(updateFileRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(updateFileResponseSchema, output)
      : null

  const showEchoes = !preview && !running && resp !== null
  const failedCount = resp?.results.filter((r) => !r.success).length ?? 0

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="files">{req.files.length}</Chip>
        {showEchoes && failedCount > 0 ? (
          <Chip label="failed" className="border-warn text-warn">
            {failedCount}
          </Chip>
        ) : null}
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · applying…
          </span>
        ) : null}
      </div>

      {showEchoes && resp ? (
        req.files.map((file, i) => {
          const result = resp.results[i]
          if (!result) {
            return (
              <div
                key={file.path}
                className="border-b border-rule-2 last:border-b-0 px-3 py-2 font-mono text-[12px] text-ink-faint"
              >
                {file.path} · no result
              </div>
            )
          }
          return <FileEchoSection key={file.path} file={file} result={result} />
        })
      ) : (
        <OpSummaryTable req={req} />
      )}

      {showEchoes ? <ChecksList checks={resp?.checks} /> : null}
    </div>
  )
}

export function UpdateFilePreview({ input }: { input: unknown }) {
  return <UpdateFileView input={input} preview />
}
