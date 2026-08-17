/**
 * Text and source files, inlined directly.
 *
 * These need no worker: the bytes are already in the browser and they are
 * already text. Reading them here rather than routing them through a
 * conversion worker keeps a dropped `.ts` file working on a rig where nothing
 * but the console is installed.
 *
 * The extension list exists because a browser's idea of a MIME type is
 * unreliable for source code — a `.ts` file arrives as `video/mp2t` (the
 * MPEG transport stream), a `.md` as an empty string, a `.rs` as nothing at
 * all. Trusting `type` alone would inline a video and refuse a Rust file.
 */

import type { Attachment } from '@/types/chat'
import {
  ATTACHED_FILE_PREFIX,
  type AttachmentFailure,
  type AttachmentReadSummary,
  escapeAttr,
  extensionOf,
  failureBlock,
  formatBytes,
} from './shared'

/** Text files inlined per message. */
export const MAX_TEXT_FILES_PER_SEND = 8

/** Characters inlined per file before the block says it stopped short. */
export const MAX_TEXT_CHARS = 20_000

/** Largest text file read from the composer. */
export const MAX_TEXT_BYTES = 2 * 1024 * 1024

/**
 * Extensions inlined as text regardless of what the browser calls them. Source
 * and configuration files people actually drag into a chat, not an exhaustive
 * list — anything missing still inlines when its declared type is `text/*`.
 */
export const TEXT_EXTENSIONS = new Set([
  'txt',
  'md',
  'markdown',
  'mdx',
  'rst',
  'log',
  'json',
  'jsonl',
  'yaml',
  'yml',
  'toml',
  'ini',
  'env',
  'xml',
  'svg',
  'html',
  'htm',
  'css',
  'scss',
  'sql',
  'sh',
  'bash',
  'zsh',
  'fish',
  'ps1',
  'js',
  'jsx',
  'mjs',
  'cjs',
  'ts',
  'tsx',
  'rs',
  'go',
  'py',
  'rb',
  'java',
  'kt',
  'swift',
  'c',
  'h',
  'cc',
  'cpp',
  'hpp',
  'cs',
  'php',
  'lua',
  'r',
  'dockerfile',
  'gitignore',
  'diff',
  'patch',
])

export interface ExpandedText {
  blocks: string[]
  read: AttachmentReadSummary[]
  failures: AttachmentFailure[]
}

/** Whether an attachment should be inlined as text. */
export function isTextAttachment(attachment: Attachment): boolean {
  if (TEXT_EXTENSIONS.has(extensionOf(attachment.name))) return true
  if (attachment.type.startsWith('text/')) return true
  return (
    attachment.type === 'application/json' ||
    attachment.type === 'application/xml' ||
    attachment.type === 'application/x-yaml'
  )
}

/**
 * Inline every text attachment as an `<attached-file …>` block.
 *
 * Attachments without their underlying `File` are skipped silently: a
 * conversation reloaded from history keeps the chip, not the bytes.
 */
export async function expandTextAttachments(
  attachments: Attachment[],
): Promise<ExpandedText> {
  const files = attachments.filter((a) => isTextAttachment(a) && a.file)
  if (files.length === 0) return { blocks: [], read: [], failures: [] }

  const blocks: string[] = []
  const read: AttachmentReadSummary[] = []
  const failures: AttachmentFailure[] = []

  for (const attachment of files.slice(0, MAX_TEXT_FILES_PER_SEND)) {
    if (attachment.size > MAX_TEXT_BYTES) {
      const reason = `${formatBytes(attachment.size)} is over the ${formatBytes(MAX_TEXT_BYTES)} limit for inlining a text file`
      blocks.push(failureBlock(attachment.name, reason))
      failures.push({ name: attachment.name, reason })
      continue
    }
    try {
      const full = await (attachment.file as File).text()
      const truncated = full.length > MAX_TEXT_CHARS
      const text = truncated ? full.slice(0, MAX_TEXT_CHARS) : full

      const attrs = [
        `path="${escapeAttr(attachment.name)}"`,
        `size="${attachment.size}"`,
      ]
      if (truncated) {
        attrs.push('truncated="true"')
        attrs.push(`total-chars="${full.length}"`)
      }
      const preamble = truncated
        ? `This is the first ${MAX_TEXT_CHARS} of ${full.length} characters.\n\n`
        : ''
      blocks.push(
        `${ATTACHED_FILE_PREFIX}${attrs.join(' ')}>\n${preamble}${text}\n</attached-file>`,
      )
      read.push({
        id: attachment.id,
        label: `${attachment.name} · ${full.length.toLocaleString('en-US')}${truncated ? '+' : ''} chars`,
      })
    } catch (err) {
      const reason = err instanceof Error ? err.message : String(err)
      blocks.push(failureBlock(attachment.name, reason))
      failures.push({ name: attachment.name, reason })
    }
  }

  for (const dropped of files.slice(MAX_TEXT_FILES_PER_SEND)) {
    const reason = `only ${MAX_TEXT_FILES_PER_SEND} text files are inlined per message`
    blocks.push(failureBlock(dropped.name, reason))
    failures.push({ name: dropped.name, reason })
  }

  return { blocks, read, failures }
}
