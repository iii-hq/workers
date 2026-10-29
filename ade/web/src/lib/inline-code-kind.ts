import {
  INLINE_FILE_ICON_BY_EXT,
  INLINE_FILE_ICON_BY_NAME,
  INLINE_FILE_ICONS,
} from './inline-file-icons.generated'

/**
 * What an inline-code span in a reply is: a web address (rendered as a
 * link), a file name (rendered with its language mark), or plain code.
 */
export type InlineCodeKind =
  | { kind: 'url'; href: string }
  | { kind: 'file'; icon: string }
  | { kind: 'code' }

/** An absolute http(s) address and nothing else (no spaces, no prose). */
const URL_RE = /^https?:\/\/[^\s<>"'`]+$/i
/** A path or file name: no spaces, no call parentheses, no operators. */
const FILE_TOKEN_RE = /^[\w@~.\-/[\]+]+$/
/**
 * Extensions that double as member names in code (`console.log`,
 * `res.json`, `event.key`): a mark only when the token is clearly a file.
 */
const AMBIGUOUS_EXT = new Set([
  'log',
  'json',
  'key',
  'data',
  'lock',
  'font',
  'db',
])

/** The Material mark for a file name or path, or null when none fits. */
export function inlineFileIcon(token: string): string | null {
  if (token.length > 200 || !FILE_TOKEN_RE.test(token)) return null
  const slash = token.lastIndexOf('/')
  const base = token.slice(slash + 1).toLowerCase()
  const named = INLINE_FILE_ICON_BY_NAME[base]
  if (named) return named
  if (!base.includes('.')) return null
  // Longest compound extension first: `index.d.ts` before `ts`.
  for (
    let dot = base.indexOf('.');
    dot !== -1;
    dot = base.indexOf('.', dot + 1)
  ) {
    const ext = base.slice(dot + 1)
    const icon = INLINE_FILE_ICON_BY_EXT[ext]
    if (!icon) continue
    const stem = base.slice(0, dot)
    if (AMBIGUOUS_EXT.has(ext) && slash === -1 && /^[a-z$][\w$]*$/i.test(stem))
      return null
    return icon
  }
  return null
}

export function inlineCodeKind(text: string): InlineCodeKind {
  const value = text.trim()
  if (value !== text || !value) return { kind: 'code' }
  if (URL_RE.test(value)) return { kind: 'url', href: value }
  const icon = inlineFileIcon(value)
  return icon ? { kind: 'file', icon } : { kind: 'code' }
}

export function inlineFileIconSvg(icon: string) {
  return INLINE_FILE_ICONS[icon] ?? null
}
