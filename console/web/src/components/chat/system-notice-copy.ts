/**
 * Copy helpers for one-line system notices. The console authors these
 * notices as lowercase "headline — detail" sentences (`could not read x.pdf —
 * file too large`). The row shows the two halves on separate lines and
 * raises only a plain leading word, so a slash command, a path, or a machine
 * identifier at the start of a notice is left exactly as authored.
 */

const HEADLINE_LIMIT = 96

export interface NoticeCopy {
  headline: string
  detail?: string
}

/**
 * Raise a leading plain lowercase word; leave `/compact`, `max_turns`, paths,
 * and `provider::model` ids alone. A colon counts as a word boundary only when
 * a space follows it (`compact: another…`), so `openai::gpt-5` stays intact.
 */
export function sentenceStart(text: string): string {
  return text.replace(
    /^[a-z]+(?=[\s.,;!?…]|:\s|$)/,
    (word) => word.charAt(0).toUpperCase() + word.slice(1),
  )
}

export function splitNotice(content: string): NoticeCopy {
  const trimmed = content.trim()
  const dash = /\s[—–]\s/.exec(trimmed)
  if (!dash || dash.index === 0 || dash.index > HEADLINE_LIMIT) {
    return { headline: sentenceStart(trimmed) }
  }
  const headline = trimmed.slice(0, dash.index).trim()
  const detail = trimmed.slice(dash.index + dash[0].length).trim()
  if (!headline || !detail) return { headline: sentenceStart(trimmed) }
  return { headline: sentenceStart(headline), detail }
}
