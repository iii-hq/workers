/**
 * `coder::apply-patch` — V4A patch text rendered as per-file diffs.
 *
 * The request is one opaque string: `*** Begin Patch` … `*** End Patch`
 * wrapping per-file hunks (`*** Add File:` / `*** Delete File:` /
 * `*** Update File:` with an optional `*** Move to:`). Add hunks render
 * like CreateFileView (empty old side); Update hunks reconstruct both
 * sides from the change lines (old = context+`-`, new = context+`+` —
 * the wire carries no full bodies); Delete hunks are a header row only,
 * like DeleteFileView. Results pair with hunks BY INDEX (one result per
 * hunk, in patch order) and carry the canonical absolute path, a kind
 * badge, and an optional bounded post-apply echo.
 */
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import { ChecksList } from './ChecksView'
import { CoderNewFilePreview, CoderTextDiff } from './CoderDiff'
import {
  type ApplyPatchResult,
  applyPatchRequestSchema,
  applyPatchResponseSchema,
  type PatchEcho,
  type PatchResultKind,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface ApplyPatchViewProps {
  input: unknown
  output?: unknown
  running?: boolean
  preview?: boolean
}

/* ---------------- pure V4A parsing (unit-tested) ---------------- */

export type PatchHunk =
  | { kind: 'add'; path: string; content: string }
  | { kind: 'delete'; path: string }
  | {
      kind: 'update'
      path: string
      moveTo: string | null
      oldText: string
      newText: string
    }

const ADD_PREFIX = '*** Add File: '
const DELETE_PREFIX = '*** Delete File: '
const UPDATE_PREFIX = '*** Update File: '
const MOVE_PREFIX = '*** Move to: '

/** Any `*** ` line ends the current hunk body — file headers, the
    Begin/End envelope, and forward-compat markers like `*** End of File`. */
function isSectionLine(line: string): boolean {
  return line.startsWith('*** ')
}

/**
 * Parse V4A patch text into per-file hunks. Tolerant by construction:
 * a missing Begin/End envelope, unknown `*** ` markers and stray body
 * lines are skipped rather than failing the whole patch — garbage input
 * yields `[]` and the view falls back to the raw text.
 */
export function parsePatch(patch: string): PatchHunk[] {
  const lines = patch.split('\n')
  const hunks: PatchHunk[] = []
  let i = 0

  while (i < lines.length) {
    const line = lines[i]

    if (line.startsWith(ADD_PREFIX)) {
      const path = line.slice(ADD_PREFIX.length)
      i += 1
      const body: string[] = []
      while (i < lines.length && !isSectionLine(lines[i])) {
        // Every Add body line is `+`-prefixed; skip malformed strays.
        if (lines[i].startsWith('+')) body.push(lines[i].slice(1))
        i += 1
      }
      hunks.push({
        kind: 'add',
        path,
        content: body.length > 0 ? `${body.join('\n')}\n` : '',
      })
      continue
    }

    if (line.startsWith(DELETE_PREFIX)) {
      hunks.push({ kind: 'delete', path: line.slice(DELETE_PREFIX.length) })
      i += 1
      continue
    }

    if (line.startsWith(UPDATE_PREFIX)) {
      const path = line.slice(UPDATE_PREFIX.length)
      i += 1
      let moveTo: string | null = null
      if (i < lines.length && lines[i].startsWith(MOVE_PREFIX)) {
        moveTo = lines[i].slice(MOVE_PREFIX.length)
        i += 1
      }
      const oldLines: string[] = []
      const newLines: string[] = []
      while (i < lines.length && !isSectionLine(lines[i])) {
        const body = lines[i]
        if (body.startsWith('@@')) {
          // `@@ <context>` locators carry a real line of the file — keep
          // it as shared context so hunk sections stay anchored; bare
          // `@@` separators carry nothing.
          const ctx = body.slice(2).trimStart()
          if (ctx !== '') {
            oldLines.push(ctx)
            newLines.push(ctx)
          }
        } else if (body.startsWith('+')) {
          newLines.push(body.slice(1))
        } else if (body.startsWith('-')) {
          oldLines.push(body.slice(1))
        } else if (body.startsWith(' ')) {
          oldLines.push(body.slice(1))
          newLines.push(body.slice(1))
        } else if (body === '') {
          // Some emitters write context blank lines without the leading
          // space — treat as shared context.
          oldLines.push('')
          newLines.push('')
        }
        i += 1
      }
      hunks.push({
        kind: 'update',
        path,
        moveTo,
        oldText: oldLines.join('\n'),
        newText: newLines.join('\n'),
      })
      continue
    }

    // Begin/End envelope, unknown markers, stray text — skip.
    i += 1
  }

  return hunks
}

/** Result kind when the wire hasn't answered yet, derived from the hunk. */
export function derivedKind(hunk: PatchHunk): PatchResultKind {
  switch (hunk.kind) {
    case 'add':
      return 'added'
    case 'delete':
      return 'deleted'
    case 'update':
      return hunk.moveTo != null ? 'moved' : 'modified'
  }
}

/* ---------------- rendering ---------------- */

const KIND_TONE: Record<PatchResultKind, 'accent' | 'warn' | 'default'> = {
  added: 'accent',
  modified: 'default',
  deleted: 'warn',
  moved: 'default',
}

function KindBadge({ kind }: { kind: PatchResultKind }) {
  return <FooterPill tone={KIND_TONE[kind]}>{kind}</FooterPill>
}

/** Optional bounded post-apply snapshot — numbered mono lines. */
function PatchEchoBlock({ echo }: { echo: PatchEcho }) {
  if (echo.lines.length === 0) return null
  // Post-apply line numbers are unique within one echo — stable keys.
  const rows = echo.lines.map((text, i) => ({
    lineNo: echo.from_line + i,
    text,
  }))
  return (
    <div className="font-mono text-[12px] leading-[1.55] py-1 border-b border-rule-2 last:border-b-0">
      {rows.map((row) => (
        <div key={row.lineNo} className="px-3 flex">
          <span className="w-12 shrink-0 pr-2 text-right tabular-nums text-ink-ghost select-none">
            {row.lineNo}
          </span>
          <span className="whitespace-pre-wrap break-all text-ink">
            {row.text}
          </span>
        </div>
      ))}
    </div>
  )
}

function HunkSection({
  hunk,
  result,
}: {
  hunk: PatchHunk
  result?: ApplyPatchResult
}) {
  const kind = result?.kind ?? derivedKind(hunk)
  const moveTo = hunk.kind === 'update' ? hunk.moveTo : null

  return (
    <div className="border-b border-rule-2 last:border-b-0">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-1.5">
        {/* Canonical absolute path once the wire responds; patch-relative
            until. Moved files keep the from → to reading. */}
        <span className="font-mono text-[12px] text-ink break-all">
          {moveTo != null ? (
            <>
              {hunk.path} <span className="text-ink-ghost">→</span>{' '}
              {result?.path ?? moveTo}
            </>
          ) : (
            (result?.path ?? hunk.path)
          )}
        </span>
        <KindBadge kind={kind} />
        {result?.new_line_count != null ? (
          <Chip label="lines">{result.new_line_count}</Chip>
        ) : null}
      </div>
      {hunk.kind === 'add' ? (
        <CoderNewFilePreview path={hunk.path} content={hunk.content} />
      ) : hunk.kind === 'update' ? (
        <CoderTextDiff
          oldPath={hunk.path}
          newPath={moveTo ?? hunk.path}
          oldContent={hunk.oldText}
          newContent={hunk.newText}
        />
      ) : null}
      {result?.echo ? <PatchEchoBlock echo={result.echo} /> : null}
    </div>
  )
}

/** Fallback row when the patch text didn't parse but the response did. */
function ResultOnlyRow({ result }: { result: ApplyPatchResult }) {
  return (
    <div className="border-b border-rule-2 last:border-b-0 px-3 py-1.5 flex flex-wrap items-center gap-1.5 font-mono text-[12px]">
      <span className="text-ink break-all">{result.path}</span>
      <KindBadge kind={result.kind} />
      {result.new_line_count != null ? (
        <Chip label="lines">{result.new_line_count}</Chip>
      ) : null}
    </div>
  )
}

export function ApplyPatchView({
  input,
  output,
  running,
  preview,
}: ApplyPatchViewProps) {
  const req = safeParseRequest(applyPatchRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(applyPatchResponseSchema, output)
      : null

  const hunks = parsePatch(req.patch)

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="files">{hunks.length || (resp?.results.length ?? 0)}</Chip>
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · applying…
          </span>
        ) : null}
      </div>

      {hunks.length > 0 ? (
        hunks.map((hunk, i) => (
          <HunkSection
            key={`${hunk.kind}:${hunk.path}`}
            hunk={hunk}
            result={resp?.results[i]}
          />
        ))
      ) : resp ? (
        resp.results.map((result) => (
          <ResultOnlyRow key={result.path} result={result} />
        ))
      ) : (
        // Unparseable patch, no response — never swallow the text.
        <pre className="px-3 py-2 font-mono text-[12px] leading-[1.55] text-ink whitespace-pre-wrap break-all">
          {req.patch}
        </pre>
      )}

      <ChecksList checks={resp?.checks} />
    </div>
  )
}

export function ApplyPatchPreview({ input }: { input: unknown }) {
  return <ApplyPatchView input={input} preview />
}
