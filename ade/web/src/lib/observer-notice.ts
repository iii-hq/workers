/**
 * The harness writes its observer notices for the agent that reads them, in
 * its own internals ("wake expired unfired", "binding sub_1 … was RETIRED",
 * "the binding stays armed"). A person reads the same entry in the timeline
 * and the queued strip, so each known notice gets a headline in iii's words
 * — what stopped or failed observing which function — while the full prose
 * stays visible beneath it, untouched, because it says what to do next.
 */

/** The first function id in the part of a notice that names what was
 * observed — never one from its reason ("`harness::spawn` is not a target"). */
function functionIn(subject: string): string | undefined {
  return /`?([\w.-]+(?:::[\w.-]+)+)`?/.exec(subject)?.[1]
}

/**
 * The headline for a harness observer notice, or `undefined` for anything
 * else (a delivered event, a typed message). `text` may keep its
 * `[notification] ` prefix.
 */
export function observerNoticeHeadline(text: string): string | undefined {
  const body = text.replace(/^\[notification\]\s*/, '')
  // "wake expired unfired: `fn` args … (binding sub_1) passed …"
  const lost =
    /^wake (expired unfired|removed):\s*(.*?)(?:\s+\(binding\b|$)/s.exec(body)
  // "binding sub_1 (was observing `fn`) was RETIRED: why"
  const subject = lost
    ? lost[2]
    : (/^(?:binding|observer) \S+(.*?)\s(?:was RETIRED|is DROPPING fires|fired but was NOT delivered)\b/s.exec(
        body,
      )?.[1] ?? '')
  const fn = functionIn(subject)
  if (/^wake expired unfired\b/.test(body)) {
    return fn
      ? `Stopped observing ${fn} · expired before it fired`
      : 'Observer expired before it fired'
  }
  if (/^wake removed\b/.test(body)) {
    return fn
      ? `Stopped observing ${fn} · removed before it fired`
      : 'Observer removed before it fired'
  }
  if (/^(?:binding|observer) \S+.*\bwas RETIRED\b/.test(body)) {
    return fn ? `Stopped observing ${fn}` : 'Observer retired'
  }
  if (/^(?:binding|observer) \S+.*\bis DROPPING fires\b/.test(body)) {
    return fn
      ? `Observing ${fn} · events are being dropped`
      : 'Observer is dropping events'
  }
  if (/^(?:binding|observer) \S+.*\bfired but was NOT delivered\b/.test(body)) {
    return fn
      ? `Observing ${fn} · fired but not delivered`
      : 'Observer fired but was not delivered'
  }
  return undefined
}
