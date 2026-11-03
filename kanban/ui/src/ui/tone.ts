/** The console's six glyph hues: the only colours a worker page paints with besides ok/warn/alert. */
export type Tone = 'blue' | 'purple' | 'teal' | 'green' | 'amber' | 'rose'

export const TONES: readonly Tone[] = ['blue', 'purple', 'teal', 'green', 'amber', 'rose']

/** A stable hue for a free-form name (a label, an agent without a colour): the same word, the same colour. */
export function toneOf(name: string): Tone {
  let hash = 0
  for (let i = 0; i < name.length; i += 1) hash = (hash * 31 + name.charCodeAt(i)) | 0
  return TONES[Math.abs(hash) % TONES.length]
}

/** An agent profile's colour when it is one of the hues, else one derived from its id. */
export function agentTone(id: string, color: string | null | undefined): Tone {
  return color && (TONES as readonly string[]).includes(color) ? (color as Tone) : toneOf(id)
}

/** `Backend Engineer` → `BE`, `tech-lead` → `TL`, `iii` → `II`. */
export function initials(name: string): string {
  const words = name
    .replace(/[-_]+/g, ' ')
    .split(/\s+/)
    .filter(Boolean)
  if (words.length === 0) return '?'
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase()
  return (words[0][0] + words[1][0]).toUpperCase()
}

/** `backend-engineer` → `Backend engineer`: how a bare assignee id reads when no profile names it. */
export function humanize(id: string): string {
  const text = id.replace(/[-_]+/g, ' ').trim()
  return text ? text.charAt(0).toUpperCase() + text.slice(1) : id
}
