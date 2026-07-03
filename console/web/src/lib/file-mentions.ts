/**
 * `#file(<path>)` mention expansion for the composer send path.
 *
 * The composer inserts `#file(<relpath>)` tokens (FileMentionNode pills). At
 * send time the console reads every mentioned file through one
 * `coder::read-file` batch call scoped to the conversation's working
 * directory, and appends one `<attached-file …>` text block per mention to
 * the outgoing user message. Failures never block the send: the failed
 * mention becomes a self-closing placeholder block the model can see, plus a
 * `failures` entry the caller surfaces as a chat notice.
 */

import { getIiiClient } from '@/lib/iii-client'

export const READ_FILE_FUNCTION_ID = 'coder::read-file'

/** Path may contain spaces but not `)` — see file-index (paren paths dropped). */
const FILE_MENTION_RE = /#file\(([^)]+)\)/g

/** Max unique mentions expanded per send; extras are ignored. */
export const MAX_MENTIONS_PER_SEND = 20

const ATTACHED_FILE_PREFIX = '<attached-file '

export interface FileMentionFailure {
  path: string
  reason: string
}

export interface ExpandedMentions {
  /** One `<attached-file …>` text block per requested path, request order. */
  blocks: string[]
  /** Chip data for the optimistic user row: bytes actually attached. */
  attachments: Array<{ path: string; size: number }>
  failures: FileMentionFailure[]
}

/** Header fields parsed back out of an `<attached-file …>` block. */
export interface AttachedFileHeader {
  path: string
  size?: number
  totalLines?: number
  truncated?: boolean
  error?: string
}

/** Unique `#file(...)` paths in first-appearance order, capped. */
export function parseFileMentions(text: string): string[] {
  const seen = new Set<string>()
  for (const m of text.matchAll(FILE_MENTION_RE)) {
    const path = m[1].trim()
    if (path.length > 0) seen.add(path)
    if (seen.size >= MAX_MENTIONS_PER_SEND) break
  }
  return [...seen]
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

type TriggerFn = (
  functionId: string,
  payload: Record<string, unknown>,
) => Promise<unknown>

/**
 * Read every mentioned file in one jail-validated batch call and format the
 * attachment blocks. Batch results come back in request order (the wire
 * contract), so blocks are labeled with the caller's relative paths.
 */
export async function expandFileMentions(
  workingDir: string,
  paths: string[],
  trigger?: TriggerFn,
): Promise<ExpandedMentions> {
  if (paths.length === 0) return { blocks: [], attachments: [], failures: [] }

  const call =
    trigger ??
    (async (functionId: string, payload: Record<string, unknown>) => {
      const client = await getIiiClient()
      return client.trigger(functionId, payload)
    })

  let results: ReadEntryResultWire[]
  try {
    const res = (await call(READ_FILE_FUNCTION_ID, {
      paths,
      fs_scope: { root: workingDir },
    })) as ReadFileBatchWire
    results = res?.results ?? []
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err)
    return {
      blocks: paths.map((path) => failureBlock(path, reason)),
      attachments: [],
      failures: paths.map((path) => ({ path, reason })),
    }
  }

  const blocks: string[] = []
  const attachments: Array<{ path: string; size: number }> = []
  const failures: FileMentionFailure[] = []

  for (const [i, path] of paths.entries()) {
    const entry = results[i]
    if (!entry?.success || typeof entry.content !== 'string') {
      const reason = shortReason(entry?.error?.message) ?? 'read failed'
      blocks.push(failureBlock(path, reason))
      failures.push({ path, reason })
      continue
    }
    if (entry.is_utf8 === false) {
      const reason = 'binary file'
      blocks.push(failureBlock(path, reason))
      failures.push({ path, reason })
      continue
    }
    blocks.push(contentBlock(path, entry))
    attachments.push({ path, size: entry.size ?? entry.content.length })
  }
  return { blocks, attachments, failures }
}

function contentBlock(path: string, entry: ReadEntryResultWire): string {
  const attrs = [`path="${escapeAttr(path)}"`]
  if (typeof entry.size === 'number') attrs.push(`size="${entry.size}"`)
  if (typeof entry.total_lines === 'number') {
    attrs.push(`total-lines="${entry.total_lines}"`)
  }
  if (entry.more_lines === true) attrs.push('truncated="true"')
  return `${ATTACHED_FILE_PREFIX}${attrs.join(' ')}>\n${entry.content}\n</attached-file>`
}

function failureBlock(path: string, reason: string): string {
  return `${ATTACHED_FILE_PREFIX}path="${escapeAttr(path)}" error="${escapeAttr(reason)}" />`
}

function shortReason(message: string | undefined | null): string | undefined {
  if (!message) return undefined
  return message.length > 120 ? `${message.slice(0, 117)}…` : message
}

/** Whether a text content block is a console-authored file attachment. */
export function isAttachedFileBlock(text: string): boolean {
  return text.startsWith(ATTACHED_FILE_PREFIX)
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

  const attr = (name: string): string | undefined => {
    const m = header.match(new RegExp(`${name}="([^"]*)"`))
    return m ? unescapeAttr(m[1]) : undefined
  }
  const path = attr('path')
  if (!path) return null

  const size = attr('size')
  const totalLines = attr('total-lines')
  return {
    path,
    ...(size !== undefined ? { size: Number(size) } : {}),
    ...(totalLines !== undefined ? { totalLines: Number(totalLines) } : {}),
    ...(attr('truncated') === 'true' ? { truncated: true } : {}),
    ...(attr('error') !== undefined ? { error: attr('error') } : {}),
  }
}

function escapeAttr(value: string): string {
  return value.replaceAll('&', '&amp;').replaceAll('"', '&quot;')
}

function unescapeAttr(value: string): string {
  return value.replaceAll('&quot;', '"').replaceAll('&amp;', '&')
}
