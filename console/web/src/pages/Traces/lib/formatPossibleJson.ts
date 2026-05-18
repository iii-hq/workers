/**
 * Pretty-print a value as JSON if and only if it looks like a JSON
 * object/array string.
 *
 * Returns the indented JSON string when the input is a string whose
 * trimmed form starts with `{` or `[` AND parses successfully. Returns
 * null otherwise (non-strings, empty strings, primitives wrapped as
 * strings like `"42"` or `"true"`, malformed JSON).
 *
 * Used by `SpanLogsTab` to pretty-print `iii.payload.json` event
 * attributes inside a <pre> block. Strings that are bare numbers,
 * booleans, or plain text fall through to the single-line renderer.
 *
 * Indent is fixed at 2 spaces.
 *
 * Performance: only attempts `JSON.parse` for strings with the right
 * leading character — bare quoted strings (`"hi"`) are valid JSON but
 * pretty-printing a single string would just add surrounding quotes,
 * which is noise. The leading-char filter avoids that and also short-
 * circuits cheaply.
 */
export function formatPossibleJson(value: unknown): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  if (trimmed.length === 0) return null
  const first = trimmed[0]
  if (first !== '{' && first !== '[') return null
  try {
    const parsed = JSON.parse(trimmed)
    return JSON.stringify(parsed, null, 2)
  } catch {
    return null
  }
}
