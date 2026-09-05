/**
 * `#file(<path>[:<from>[-<to>]])` mention expansion for the composer send
 * path.
 *
 * The composer inserts `#file(…)` tokens (FileMentionNode pills). At send
 * time the console reads every mentioned file — or just the mentioned
 * lines — through one `coder::read-file` batch call scoped to the
 * conversation's working directory, and appends one `<attached-file …>`
 * text block per mention to the outgoing user message. Failures never
 * block the send: the failed mention becomes a self-closing placeholder
 * block the model can see, plus a `failures` entry the caller surfaces as a
 * chat notice.
 */

import {
  ATTACHED_FILE_PREFIX,
  escapeAttr,
  failureBlock,
  type TriggerFn,
  triggerOr,
} from '@/lib/attachments/shared'
import {
  FILE_MENTION_RE,
  type FileMentionRef,
  formatFileMentionInner,
  formatLineRange,
  parseFileMentionInner,
} from '@/lib/file-mention-token'
import { workspaceScope } from '@/lib/fs-scope'

export type { FileMentionRef } from '@/lib/file-mention-token'

export const READ_FILE_FUNCTION_ID = 'coder::read-file'

/** Max unique mentions expanded per send; extras are ignored. */
export const MAX_MENTIONS_PER_SEND = 20

export interface FileMentionFailure {
  /** The mention as written, e.g. `src/a.ts:12-40`. */
  path: string
  reason: string
}

export interface ExpandedMentions {
  /** One `<attached-file …>` text block per requested mention, request order. */
  blocks: string[]
  /** Chip data for the optimistic user row: the mention label and bytes attached. */
  attachments: Array<{ path: string; size: number }>
  failures: FileMentionFailure[]
}

/** Header fields parsed back out of an `<attached-file …>` block. */
export interface AttachedFileHeader {
  path: string
  /** `12-40` when only a window of the file was attached. */
  lines?: string
  size?: number
  totalLines?: number
  truncated?: boolean
  error?: string
}

/** Unique `#file(...)` references in first-appearance order, capped. */
export function parseFileMentions(text: string): FileMentionRef[] {
  const seen = new Map<string, FileMentionRef>()
  for (const m of text.matchAll(FILE_MENTION_RE)) {
    const ref = parseFileMentionInner(m[1])
    if (ref.path.length === 0) continue
    const key = formatFileMentionInner(ref)
    if (!seen.has(key)) seen.set(key, ref)
    if (seen.size >= MAX_MENTIONS_PER_SEND) break
  }
  return [...seen.values()]
}

// --- wire subset of coder::read-file batch mode ---------------------------

interface ReadEntryResultWire {
  path: string
  success: boolean
  content?: string | null
  is_utf8?: boolean | null
  total_lines?: number | null
  more_lines?: boolean | null
  size?: number | null
  error?: { code?: string; message?: string } | null
}

interface ReadFileBatchWire {
  results?: ReadEntryResultWire[]
}

/** A batch entry: the bare path for a whole file, a window object for lines. */
type ReadTargetWire =
  | string
  | { path: string; line_from: number; line_to: number }

function readTarget(ref: FileMentionRef): ReadTargetWire {
  return ref.range
    ? { path: ref.path, line_from: ref.range.from, line_to: ref.range.to }
    : ref.path
}

/**
 * Read every mentioned file (or line window) in one jail-validated batch
 * call and format the attachment blocks. Batch results come back in request
 * order (the wire contract), so blocks are labeled with the caller's
 * references.
 *
 * Folder mentions (trailing `/`) attach nothing: the `#file(dir/)` token
 * stays in the message text and the agent lists the folder on demand.
 */
export async function expandFileMentions(
  workingDir: string,
  refs: readonly FileMentionRef[],
  trigger?: TriggerFn,
): Promise<ExpandedMentions> {
  const fileRefs = refs.filter((ref) => !ref.path.endsWith('/'))
  if (fileRefs.length === 0) {
    return { blocks: [], attachments: [], failures: [] }
  }
  const labels = fileRefs.map(formatFileMentionInner)

  const call = triggerOr(trigger)

  let results: ReadEntryResultWire[]
  try {
    const res = (await call(READ_FILE_FUNCTION_ID, {
      paths: fileRefs.map(readTarget),
      fs_scope: workspaceScope(workingDir),
    })) as ReadFileBatchWire
    results = res?.results ?? []
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err)
    return {
      blocks: labels.map((label) => failureBlock(label, reason)),
      attachments: [],
      failures: labels.map((label) => ({ path: label, reason })),
    }
  }

  const blocks: string[] = []
  const attachments: Array<{ path: string; size: number }> = []
  const failures: FileMentionFailure[] = []

  for (const [i, ref] of fileRefs.entries()) {
    const label = labels[i]
    const entry = results[i]
    if (!entry?.success || typeof entry.content !== 'string') {
      const reason = shortReason(entry?.error?.message) ?? 'read failed'
      blocks.push(failureBlock(label, reason))
      failures.push({ path: label, reason })
      continue
    }
    if (entry.is_utf8 === false) {
      const reason = 'binary file'
      blocks.push(failureBlock(label, reason))
      failures.push({ path: label, reason })
      continue
    }
    blocks.push(contentBlock(ref, entry))
    attachments.push({
      path: label,
      size: ref.range
        ? entry.content.length
        : (entry.size ?? entry.content.length),
    })
  }
  return { blocks, attachments, failures }
}

function contentBlock(ref: FileMentionRef, entry: ReadEntryResultWire): string {
  const attrs = [`path="${escapeAttr(ref.path)}"`]
  if (ref.range) attrs.push(`lines="${formatLineRange(ref.range)}"`)
  if (typeof entry.size === 'number') attrs.push(`size="${entry.size}"`)
  if (typeof entry.total_lines === 'number') {
    attrs.push(`total-lines="${entry.total_lines}"`)
  }
  if (entry.more_lines === true && !ref.range) attrs.push('truncated="true"')
  return `${ATTACHED_FILE_PREFIX}${attrs.join(' ')}>\n${entry.content}\n</attached-file>`
}

function shortReason(message: string | undefined | null): string | undefined {
  if (!message) return undefined
  return message.length > 120 ? `${message.slice(0, 117)}…` : message
}

/** Whether a text content block is a console-authored file attachment. */
export function isAttachedFileBlock(text: string): boolean {
  return text.startsWith(ATTACHED_FILE_PREFIX)
}

/** The chip label for an attachment header: `src/a.ts`, `src/a.ts:12-40`. */
export function attachedFileLabel(header: AttachedFileHeader): string {
  return header.lines ? `${header.path}:${header.lines}` : header.path
}

/**
 * Parse the header of an `<attached-file …>` block. Returns null when the
 * text is not an attachment block or the header has no path.
 */
export function parseAttachedFileHeader(
  text: string,
): AttachedFileHeader | null {
  if (!isAttachedFileBlock(text)) return null
  const headerEnd = text.indexOf('>')
  if (headerEnd < 0) return null
  const header = text.slice(ATTACHED_FILE_PREFIX.length, headerEnd)

  /* Anchored at a boundary so `lines` never reads `total-lines`. */
  const attr = (name: string): string | undefined => {
    const m = header.match(new RegExp(`(?:^|\\s)${name}="([^"]*)"`))
    return m ? unescapeAttr(m[1]) : undefined
  }
  const path = attr('path')
  if (!path) return null

  const lines = attr('lines')
  const size = attr('size')
  const totalLines = attr('total-lines')
  return {
    path,
    ...(lines !== undefined ? { lines } : {}),
    ...(size !== undefined ? { size: Number(size) } : {}),
    ...(totalLines !== undefined ? { totalLines: Number(totalLines) } : {}),
    ...(attr('truncated') === 'true' ? { truncated: true } : {}),
    ...(attr('error') !== undefined ? { error: attr('error') } : {}),
  }
}

function unescapeAttr(value: string): string {
  return value
    .replaceAll('&gt;', '>')
    .replaceAll('&quot;', '"')
    .replaceAll('&amp;', '&')
}
