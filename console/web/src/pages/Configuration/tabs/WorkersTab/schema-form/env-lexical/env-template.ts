/**
 * Parser / serializer for the `${VAR}` and `${VAR:default}` template
 * syntax that the `configuration` worker expands on read.
 *
 * The grammar mirrors the engine-side regex used in
 * `engine/src/workers/config.rs::EngineConfig::expand_env_vars`:
 *
 *   \$\{([^}:]+)(?::([^}]*))?\}
 *
 *   - variable name: one or more characters that are not `}` or `:`
 *   - default value (optional): any characters up to the closing `}`
 *
 * Anything that doesn't match is plain text. The parser never throws;
 * malformed inputs (`${unterminated`, `$invalid`, etc.) are returned as
 * literal text segments so what the user typed always round-trips.
 *
 * Why this lives in its own module: the Lexical decorator node, the
 * transform plugin, and the read-only hover preview all need the same
 * parse → render mapping. Centralizing it keeps the three in sync.
 */

export interface PlaceholderSegment {
  kind: 'placeholder'
  variable: string
  /** `null` when no `:default` was supplied (distinct from empty string). */
  defaultValue: string | null
}

export interface TextSegment {
  kind: 'text'
  text: string
}

export type Segment = PlaceholderSegment | TextSegment

/**
 * Global regex used by the parser. We rebuild it per-call (no shared
 * `lastIndex` mutation) so concurrent parses can't trample each other.
 */
function placeholderPattern(): RegExp {
  return /\$\{([^}:]+)(?::([^}]*))?\}/g
}

/**
 * Split `input` into a flat list of `{ text }` and `{ placeholder }`
 * segments in source order. Empty text segments between consecutive
 * placeholders are coalesced out.
 */
export function parseTemplate(input: string): Segment[] {
  const segments: Segment[] = []
  if (!input) return segments
  const re = placeholderPattern()
  let cursor = 0
  let match: RegExpExecArray | null = re.exec(input)
  while (match !== null) {
    const start = match.index
    if (start > cursor) {
      segments.push({ kind: 'text', text: input.slice(cursor, start) })
    }
    segments.push({
      kind: 'placeholder',
      variable: match[1],
      defaultValue: match[2] === undefined ? null : match[2],
    })
    cursor = start + match[0].length
    match = re.exec(input)
  }
  if (cursor < input.length) {
    segments.push({ kind: 'text', text: input.slice(cursor) })
  }
  return segments
}

/**
 * Inverse of `parseTemplate`: rebuild the original string from a list
 * of segments. `${VAR}` is emitted when `defaultValue === null`,
 * `${VAR:default}` otherwise (an explicit empty-string default round-trips
 * as `${VAR:}`).
 */
export function serializeSegments(segments: readonly Segment[]): string {
  let out = ''
  for (const segment of segments) {
    if (segment.kind === 'text') {
      out += segment.text
    } else {
      out += serializePlaceholder(segment)
    }
  }
  return out
}

export function serializePlaceholder(segment: PlaceholderSegment): string {
  if (segment.defaultValue === null) return `\${${segment.variable}}`
  return `\${${segment.variable}:${segment.defaultValue}}`
}

/**
 * Find the first placeholder match in `input` starting at `offset`.
 * Returned positions are absolute (relative to `input`, not to the
 * sliced search window). Used by the Lexical transform plugin to
 * locate the next `${…}` to convert into a decorator node, one match
 * per pass.
 */
export function findFirstPlaceholder(
  input: string,
  offset: number = 0,
): { start: number; end: number; segment: PlaceholderSegment } | null {
  if (offset < 0) offset = 0
  if (offset >= input.length) return null
  const re = placeholderPattern()
  re.lastIndex = offset
  const match = re.exec(input)
  if (!match) return null
  return {
    start: match.index,
    end: match.index + match[0].length,
    segment: {
      kind: 'placeholder',
      variable: match[1],
      defaultValue: match[2] === undefined ? null : match[2],
    },
  }
}
