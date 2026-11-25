/**
 * The title of a `harness::observe` row, in iii's words: "Observing
 * <function>". The agent's own description of the call is free prose and
 * often borrows harness internals ("Arming the wake for …"), so it never
 * titles the row while the observed function is known. Without it (a paged
 * placeholder, or arguments still streaming) the row reads the intent out of
 * that prose instead: "Arming the observer for sub-agent" → "Observing
 * sub-agent".
 */

/** The one call whose row iii titles itself: "Observing <function>". */
export const OBSERVE_FUNCTION_ID = 'harness::observe'

export type ObserveRowTitle =
  /** "Observing <functionId>". */
  | { functionId: string }
  /** "Observing <intent>". */
  | { intent: string }
  /** The agent's description, as written: it names no observer verb. */
  | { text: string }

/** "Arming the wake for", "Re-arm wakes on", "Arming for" — arm, with or
 * without the thing it arms. */
const ARM =
  /^(?:re-?)?arm(?:ing|ed|s)?\b\s*(?:(?:a|an|the|one|new|another)\s+)*(?:(?:durable|one-shot|once|state|standing)\s+)*(?:(?:wake|observer|watch|subscription|listener|binding|trigger)s?\b)?/i

/** "Registering an observer:", "Setting up a watch until" — a setup verb
 * only counts with the observer noun after it ("Creating the ticket" is
 * not an observer). */
const SETUP =
  /^(?:register(?:ing|ed|s)?|set(?:ting)? up|creat(?:e|es|ing|ed)|add(?:ing|ed|s)?|install(?:ing|ed|s)?|schedul(?:e|es|ing|ed))\s+(?:(?:a|an|the|one|new|another)\s+)*(?:(?:durable|one-shot|once|state|standing)\s+)*(?:wake|observer|watch|subscription|listener|binding|trigger)s?\b/i

/** Already the right verb: "Observing sub-agent", "Watching the test run". */
const OBSERVE = /^(?:observ(?:e|es|ing)|watch(?:es|ing)?)\b/i

/** The joint between the verb phrase and what it is about. */
const JOINT =
  /^\s*(?:(?:for|on|to|until|of|that waits for|waiting for|so (?:we|i) (?:hear|know) (?:when|about))\b|[:\-–—])?\s*/i

function intentOf(description: string): ObserveRowTitle | null {
  const prose = description.trim().replace(/[.…]+$/, '')
  if (!prose) return null
  const verb = ARM.exec(prose) ?? SETUP.exec(prose) ?? OBSERVE.exec(prose)
  if (!verb) return { text: prose }
  const intent = prose.slice(verb[0].length).replace(JOINT, '').trim()
  return intent ? { intent } : null
}

export function observeRowTitle(
  input: unknown,
  description?: string,
): ObserveRowTitle | null {
  const functionId =
    input && typeof input === 'object' && !Array.isArray(input)
      ? (input as { function_id?: unknown }).function_id
      : undefined
  if (typeof functionId === 'string' && functionId.trim()) {
    return { functionId: functionId.trim() }
  }
  return description ? intentOf(description) : null
}
