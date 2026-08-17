/**
 * Shared building blocks for every sandbox-code-runner function-trigger
 * renderer.
 *
 * The three cards (run / register_function / teardown) differ only in their
 * body — the frame, the runtime-id chip, the terminal streams, the exit
 * status and the id list are identical, and live here.
 *
 * SECURITY — `runtime_id` is a capability: whoever holds one can run into
 * or tear down that runtime. It is NEVER rendered in full. `RuntimeChip` is
 * the only sanctioned way to show one: truncated, with the full value
 * reachable only by an explicit click-to-copy. Any other text that could
 * embed one (an error message, a line of stdout, a function id) goes through
 * `redactRuntimeIds` first.
 *
 * What sandbox-code-runner is NOT: node-engine. A run here returns a PROCESS
 * result — stdout, stderr, exit code — not a completion value plus captured
 * console lines. A non-zero exit is a normal response carrying the user's own
 * compiler or runtime message; errors are reserved for infrastructure
 * failures. `Stream` and `ExitStatus` exist to keep that distinction visible.
 */

import { Tooltip, TooltipContent, TooltipTrigger } from '@iii-dev/console-ui'
import { useState } from 'react'
import { useCopyFlash } from './clipboard'
import { asRecord, unwrapEnvelope } from './payload'

/* --- payload helpers -------------------------------------------------- */

export { asRecord, unwrapEnvelope }

export function isErrorOutput(value: unknown): boolean {
  return (
    !!value &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    'error' in (value as Record<string, unknown>)
  )
}

export const FUNCTION_PREFIX = 'sandbox-code-runner::'

/** `sandbox-code-runner::run` → `run` (the op pill's label). */
export function opName(functionId: string): string {
  return functionId.startsWith(FUNCTION_PREFIX)
    ? functionId.slice(FUNCTION_PREFIX.length)
    : functionId
}

/**
 * The Prism id for a run request's `lang`, or `undefined` when there is no
 * honest answer.
 *
 * Only `run` carries a language: `register_function`'s request has no `lang`
 * field at all (the language belongs to the runtime the function is being
 * registered into), so that card CANNOT know it from the payload. `undefined`
 * means "render unhighlighted" — never guess a language onto a code block,
 * a wrong one reads as a claim about what will execute.
 */
export function langToPrism(lang: unknown): string | undefined {
  if (lang === 'node') return 'javascript'
  if (lang === 'python') return 'python'
  return undefined
}

/* --- runtime id (capability) ------------------------------------------ */

/** `rt-3f9a2c1e-…` → `rt-3f9a…`. Short ids are shown whole. */
export function truncateRuntimeId(runtimeId: string): string {
  return runtimeId.length > 8 ? `${runtimeId.slice(0, 7)}…` : runtimeId
}

/**
 * `manager.rs` mints `rt-<uuid>` (`format!("rt-{}", Uuid::new_v4())`), and
 * sandbox-code-runner's own error MESSAGES quote it by design — `RuntimeNotFound` is
 * "unknown runtime_id {id}" and `Expired` is "runtime {id} expired: …"
 * (error.rs, a documented exception to the redaction convention: those go to
 * the caller who already holds the id). The console feed is not that caller,
 * so every string that could embed one — an error message, a line of program
 * output, a bus function id — runs through this before it is rendered.
 *
 * No `\b` anchors: hex digits are word characters, so an id glued to
 * `[A-Za-z0-9_]` (`<id>_worker`, `app::<id>_a`, `/tmp/<id>_out.json`) matches
 * neither boundary and would pass through whole. Matching the bare 39-char
 * shape unanchored can only redact MORE, never less — it is the capability
 * regardless of what touches it.
 */
const RUNTIME_ID_PATTERN =
  /rt-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi

/** Replace every `rt-<uuid>` substring of `text` with its truncated form. */
export function redactRuntimeIds(text: string): string {
  return text.replace(RUNTIME_ID_PATTERN, (id) => truncateRuntimeId(id))
}

/**
 * `redactRuntimeIds` over EVERY string in an arbitrary JSON-ish value —
 * object keys included (a namespace-less runtime's id shows up as a key
 * whenever a payload maps registrations by namespace). Objects, arrays,
 * numbers, booleans and null keep their shape; the input is never mutated.
 *
 * This is what the renderers hand the console as `redactRaw`: the card's
 * `raw json` tab renders the request/response verbatim and its copy button
 * copies them, so a card that shows only `RuntimeChip` has not contained the
 * capability until the raw value is filtered too.
 *
 * `seen` is the current PATH, not every visited node: a value referenced
 * twice is redacted twice (correct), while a cycle collapses to
 * `'[circular]'` rather than hanging the console. JSON off the wire cannot
 * be cyclic, but `redactRaw` must be total for whatever it is handed.
 */
export function redactRuntimeIdsDeep(
  value: unknown,
  seen: WeakSet<object> = new WeakSet(),
): unknown {
  if (typeof value === 'string') return redactRuntimeIds(value)
  if (value === null || typeof value !== 'object') return value
  if (seen.has(value)) return '[circular]'
  seen.add(value)
  const out = Array.isArray(value)
    ? value.map((entry) => redactRuntimeIdsDeep(entry, seen))
    : Object.fromEntries(
        Object.entries(value as Record<string, unknown>).map(([k, v]) => [
          redactRuntimeIds(k),
          redactRuntimeIdsDeep(v, seen),
        ]),
      )
  seen.delete(value)
  return out
}

/**
 * The truncated runtime-id chip. The full id is a capability, so it is only
 * ever handed over on an explicit click (copied to the clipboard), never
 * printed into the feed.
 */
export function RuntimeChip({ runtimeId }: { runtimeId: string }) {
  const { state, copy } = useCopyFlash(runtimeId)

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          className={`cr-ui-rt${state === 'idle' ? '' : ` ${state}`}`}
          onClick={copy}
          // Never the id itself: an aria-label puts it in the DOM and reads
          // it aloud, which is exactly the exposure the truncation prevents.
          aria-label="copy the full runtime id"
        >
          <span className="k">runtime </span>
          {truncateRuntimeId(runtimeId)}
          {state === 'idle' ? null : (
            <span className="cr-ui-rt-flash">
              {state === 'copied' ? ' copied' : ' copy failed'}
            </span>
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent>
        A runtime id is a capability — anyone holding it can run into or tear
        down that sandbox, so only a prefix is shown. Click to copy the full id.
      </TooltipContent>
    </Tooltip>
  )
}

/* --- card frame -------------------------------------------------------- */

/**
 * The frame every sandbox-code-runner card shares: op pill, caller-supplied
 * chips, the "sandbox-code-runner ui" attribution tag (so an override is
 * distinguishable from first-party rendering), and the body.
 */
export function CardShell({
  op,
  running,
  tag = 'sandbox-code-runner ui',
  chips,
  children,
}: {
  op: string
  running?: boolean
  tag?: string
  chips?: React.ReactNode
  children?: React.ReactNode
}) {
  return (
    <div className="cr-ui-msg">
      <div className="cr-ui-msg-head">
        <span className={`cr-ui-pill${running ? ' quiet' : ''}`}>{op}</span>
        {chips}
        <span className="cr-ui-msg-tag">{tag}</span>
      </div>
      {children}
    </div>
  )
}

/* --- terminal streams --------------------------------------------------- */

/** Lines of a stream shown before it collapses behind a toggle. */
const STREAM_CLAMP_LINES = 12
/** …and a character ceiling, for the one 400 KB line a minifier emits. */
const STREAM_CLAMP_CHARS = 2000

/**
 * `stdout` / `stderr` as terminal output: monospace, whitespace preserved,
 * clamped so a chatty script cannot flood the chat (the CSS caps the height
 * and scrolls; this caps what is in the DOM at all). `null` for an empty
 * string — the caller decides what "no output" should say, if anything.
 *
 * `tone="err"` tints the stream, and nothing more: stderr on a non-zero exit
 * is the user's own compiler or runtime message, not a system error.
 */
export function Stream({
  label,
  text,
  tone = 'out',
}: {
  label: string
  text: string
  tone?: 'out' | 'err'
}) {
  const [expanded, setExpanded] = useState(false)
  if (text.length === 0) return null

  const safe = redactRuntimeIds(text)
  const lines = safe.split('\n')
  const long =
    lines.length > STREAM_CLAMP_LINES || safe.length > STREAM_CLAMP_CHARS
  const collapsed = long && !expanded
  const shown = collapsed
    ? lines.slice(0, STREAM_CLAMP_LINES).join('\n').slice(0, STREAM_CLAMP_CHARS)
    : safe

  return (
    <div className={`cr-ui-stream ${tone}`}>
      <div className="cr-ui-stream-label">{label}</div>
      <pre className="cr-ui-stream-body">{shown}</pre>
      {long ? (
        <button
          type="button"
          className="cr-ui-toggle"
          onClick={() => setExpanded((v) => !v)}
        >
          {collapsed
            ? `expand · ${lines.length} lines, ${safe.length} chars`
            : 'collapse'}
        </button>
      ) : null}
    </div>
  )
}

/* --- exit status -------------------------------------------------------- */

/**
 * The one-line verdict on a run: exit code, what it means, how long it took.
 *
 * A non-zero exit is NOT an error. sandbox-code-runner reserves errors for
 * infrastructure failures; a script that throws comes back as an ordinary
 * response with its message in `stderr`. So a failing exit is `--color-warn`
 * ("your program failed") and never `--color-alert` ("the system failed").
 */
export function ExitStatus({
  exitCode,
  success,
  durationMs,
}: {
  exitCode?: number
  success?: boolean
  durationMs?: number
}) {
  const cleanExit = exitCode === 0
  // An omitted `success` (undefined) never gets promoted to a claim either
  // way — exit code 0 alone reads as clean, same as before. What must NOT
  // happen is treating `success: false` on a 0 exit code as the ordinary
  // "non-zero exit" case: manager.rs's `success: …unwrap_or(false)` makes
  // exactly that pair reachable from an honest daemon reply, and the note
  // has to describe what is actually shown (`exit 0`), not contradict it.
  const ok = cleanExit && success !== false
  const note =
    exitCode === undefined
      ? 'no exit code in the response'
      : ok
        ? 'clean exit'
        : cleanExit
          ? 'exit 0, but the response reported success: false — its own message is in stderr'
          : 'the script exited non-zero — its own message is in stderr'

  return (
    <div className="cr-ui-exit">
      <span className={`cr-ui-exit-code${ok ? ' ok' : ' failed'}`}>
        {exitCode === undefined ? 'exit ?' : `exit ${exitCode}`}
      </span>
      <span className="cr-ui-exit-note">{note}</span>
      {durationMs === undefined ? null : (
        <span className="cr-ui-exit-dur">{durationMs}ms</span>
      )}
    </div>
  )
}

/* --- registered function ids ------------------------------------------- */

/**
 * Compact list of bus function ids a call touched. `null` when empty.
 *
 * The shared sink for every id list in this UI — `id` is redacted here so
 * every call site gets the fix once. Ids are caller-chosen, but nothing stops
 * a caller naming one after its runtime.
 */
export function RegisteredIds({ ids }: { ids: readonly string[] }) {
  if (ids.length === 0) return null
  return (
    <div className="cr-ui-ids">
      {ids.map((id) => (
        <span className="cr-ui-id" key={id}>
          {redactRuntimeIds(id)}
        </span>
      ))}
    </div>
  )
}

/* --- timeout chip ------------------------------------------------------- */

/** `timeout_ms`, when the request carried one. */
export function TimeoutChip({ ms }: { ms?: number }) {
  if (ms === undefined) return null
  return (
    <span className="cr-ui-chip">
      <span className="k">timeout </span>
      {ms}ms
    </span>
  )
}

/* --- errors -------------------------------------------------------------- */

/**
 * Pull the message out of a sandbox-code-runner error output. Checked at
 * both the raw value and its unwrapped envelope — the same two places every
 * renderer's `isErrorOutput` check looks — since which level carries the
 * `{ error }` key depends on the path the failure took.
 */
export function errorInfo(output: unknown): { message: string } | undefined {
  const direct = asRecord(output)
  const nested = asRecord(unwrapEnvelope(output))
  const rec = isErrorOutput(direct)
    ? direct
    : isErrorOutput(nested)
      ? nested
      : undefined
  if (!rec) return undefined
  const err = rec.error
  const errObj = asRecord(err)
  const message =
    typeof err === 'string'
      ? err
      : typeof errObj?.message === 'string'
        ? errObj.message
        : JSON.stringify(err)
  return { message }
}

/**
 * The error card every sandbox-code-runner renderer shows instead of falling
 * through to the console's default error view: sandbox-code-runner's error
 * MESSAGES carry the runtime_id capability by design (`unknown runtime_id {id}`,
 * `runtime {id} expired: …` — error.rs), so the unredacted default view would
 * print it verbatim on an ordinary mistake.
 *
 * This is an infrastructure failure — the runtime is gone, the daemon refused,
 * the deadline blew. A script that merely exited non-zero never lands here.
 * A call the approval gate DENIED never lands here either — see `DeniedCard`.
 */
export function ErrorCard({
  op,
  runtimeId,
  message,
}: {
  op: string
  runtimeId?: string
  message: string
}) {
  return (
    <CardShell
      op={op}
      chips={runtimeId ? <RuntimeChip runtimeId={runtimeId} /> : null}
    >
      <div className="cr-ui-msg-note cr-ui-alert">
        {redactRuntimeIds(message)}
      </div>
    </CardShell>
  )
}

/* --- gate denials ------------------------------------------------------- */

/**
 * A deny/timeout resolution from the approval gate rides in `error.details`
 * as the gate's DenialEnvelope (`{ status: 'denied', denied_by, reason,
 * args_excerpt, … }` — approval-gate/src/types.rs, assembled by
 * approval-gate/src/functions/resolve.rs). This mirrors the console's own
 * `isDeniedOutput` (FunctionTriggerCard.tsx) so the two agree on what a
 * denial looks like — checked at both the raw value and its unwrapped
 * envelope, the same two places `errorInfo` looks.
 */
export function isDeniedOutput(output: unknown): boolean {
  const direct = asRecord(output)
  const nested = asRecord(unwrapEnvelope(output))
  const rec = isErrorOutput(direct)
    ? direct
    : isErrorOutput(nested)
      ? nested
      : undefined
  if (!rec) return false
  const details = asRecord(asRecord(rec.error)?.details)
  return !!details && details.status === 'denied' && 'denied_by' in details
}

export interface DenialInfo {
  reason: string
  deniedBy?: string
}

/** `{ reason, deniedBy }` out of a denial output, or `undefined` when
 * `output` is not one — see `isDeniedOutput`. */
export function deniedInfo(output: unknown): DenialInfo | undefined {
  if (!isDeniedOutput(output)) return undefined
  const direct = asRecord(output)
  const nested = asRecord(unwrapEnvelope(output))
  const rec = (isErrorOutput(direct) ? direct : nested) as Record<
    string,
    unknown
  >
  const err = asRecord(rec.error)
  const details = asRecord(err?.details) ?? {}
  const reason =
    typeof details.reason === 'string'
      ? details.reason
      : typeof err?.message === 'string'
        ? err.message
        : 'denied at the gate'
  const deniedBy =
    typeof details.denied_by === 'string' ? details.denied_by : undefined
  return { reason, deniedBy }
}

/**
 * A gate denial: the call was stopped at the approval gate and never reached
 * a runtime. Distinct from `ErrorCard` on purpose — `ErrorCard` means an
 * infrastructure failure AFTER the call landed (the runtime is gone, the
 * daemon refused, the deadline blew), and its RuntimeChip would wrongly
 * imply a runtime was involved in a call that never reached one. The
 * envelope's `args_excerpt` can carry the runtime_id capability (a caller is
 * free to pass one as an argument), so this never falls through to the
 * console's default view and never renders anything but the redacted
 * `reason` — the envelope itself is not printed.
 */
export function DeniedCard({
  op,
  reason,
  deniedBy,
}: {
  op: string
  reason: string
  deniedBy?: string
}) {
  return (
    <CardShell op={op}>
      <div className="cr-ui-msg-note cr-ui-warn">
        · denied at the gate — this never ran
        {deniedBy ? ` · denied by ${deniedBy}` : ''}
      </div>
      <div className="cr-ui-msg-note">{redactRuntimeIds(reason)}</div>
    </CardShell>
  )
}
