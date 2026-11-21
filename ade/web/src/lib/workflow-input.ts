/**
 * Reading back the `<workflow_input>` fence the workflow worker wraps around
 * a node's input (`build_opening` in `workflow/src/functions/tick.rs`): the
 * operator's instruction prose, then the untrusted upstream data as one line
 * of JSON, then an optional output instruction. The chat shows the data as a
 * card instead of a raw tag and a wall of JSON.
 */

export interface WorkflowInput {
  /** Instruction prose before the fence, trimmed ('' when none). */
  before: string
  /** Instruction prose after the fence, trimmed ('' when none). */
  after: string
  /** The fence's `note` attribute, or null when it has none. */
  note: string | null
  /** The parsed body; undefined when `parsed` is false. */
  value: unknown
  /** The body as written, trimmed. */
  raw: string
  /** Whether the body was valid JSON. */
  parsed: boolean
}

// Only the exact shape the worker writes: the fence is its own paragraph and
// the body one line of compact JSON (with `<`/`>` escaped). A tag quoted in a
// person's code block, inline code or a template's prose is left alone.
const FENCE =
  /(?<=^|\n\n)<workflow_input\b([^>\n]*)>\n([^\n]*)\n<\/workflow_input>(?=\n\n|$)/

/** The first complete `<workflow_input>` block in `text`, or null. */
export function parseWorkflowInput(text: string): WorkflowInput | null {
  const match = FENCE.exec(text)
  if (!match) return null
  const raw = match[2].trim()
  let value: unknown
  let parsed = true
  try {
    value = JSON.parse(raw)
  } catch {
    parsed = false
  }
  return {
    before: text.slice(0, match.index).trim(),
    after: text.slice(match.index + match[0].length).trim(),
    note: /\bnote="([^"]*)"/.exec(match[1])?.[1] ?? null,
    value,
    raw,
    parsed,
  }
}
