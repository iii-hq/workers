/**
 * Shared building blocks for every code-runner function-trigger renderer.
 *
 * The three cards (run / register_function / teardown) differ only in their
 * body — the frame, the runtime-id chip, the terminal streams, the exit
 * status, the completion value and the id list are identical, and live here.
 *
 * SECURITY — `runtime_id` is a capability: whoever holds one can run into or
 * tear down that runtime. It is NEVER rendered in full. `RuntimeChip` is the
 * only sanctioned way to show one: truncated, with the full value reachable
 * only by an explicit click-to-copy. Any other text that could embed one (an
 * error message, a line of stdout, a function id) goes through
 * `redactRuntimeIds` first.
 *
 * What code-runner is: ONE api over TWO in-process engines — untrusted
 * JavaScript in a deno_core V8 isolate, untrusted Python as CPython compiled
 * to WebAssembly in wasmtime. No microVM, no /dev/kvm. A run returns a
 * PROCESS result — stdout, stderr, exit code — AND a completion value, which
 * is the one field `sandbox-code-runner`'s wire does not carry. `Stream`,
 * `ExitStatus` and `ResultValue` exist to keep those two things distinct: the
 * streams are what the code printed, `result` is what it returned.
 *
 * A non-zero exit is NOT an error. code-runner reserves errors for
 * infrastructure failures (`error.rs`); a script that throws comes back as an
 * ordinary response with its own message in `stderr`.
 */

import { Tooltip, TooltipContent, TooltipTrigger } from '@iii-dev/console-ui'
import { useCallback, useState } from 'react'

/* --- payload helpers -------------------------------------------------- */

/** Narrow to a plain object, or `undefined` for anything else (incl. arrays). */
export function asRecord(value: unknown): Record<string, unknown> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  return value as Record<string, unknown>
}

/** `{ content: [...], details }` harness result envelope → details. */
export function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}

export function isErrorOutput(value: unknown): boolean {
  return !!value && typeof value === 'object' && !Array.isArray(value) && 'error' in (value as Record<string, unknown>)
}

export const FUNCTION_PREFIX = 'code-runner::'

/** `code-runner::run` → `run` (the op pill's label). */
export function opName(functionId: string): string {
  return functionId.startsWith(FUNCTION_PREFIX) ? functionId.slice(FUNCTION_PREFIX.length) : functionId
}

/**
 * The Prism id for a request's `lang`, or `undefined` when there is no honest
 * answer — which renders unhighlighted. Never guess a language onto a code
 * block: a wrong one reads as a claim about what will execute.
 */
export function langToPrism(lang: unknown): string | undefined {
  if (lang === 'node') return 'javascript'
  if (lang === 'python') return 'python'
  return undefined
}

/**
 * How a run's code is evaluated, per engine — the thing that decides whether
 * a given snippet returns anything at all.
 *
 * Node source is a FUNCTION BODY: `return 2 + 2` yields 4, a bare `2 + 2`
 * yields nothing. Python source is a MODULE: assigning `result` is what
 * returns a value. Getting this wrong is the single most common way a call
 * comes back `result: null` having "worked", so the card says which
 * convention applied rather than leaving the reader to guess.
 */
export function resultConvention(lang: unknown): string | undefined {
  if (lang === 'node') return 'node code is a function body — `return` to yield a value'
  if (lang === 'python') return 'python code is a module — assign `result` to yield a value'
  return undefined
}

/* --- runtime id (capability) ------------------------------------------ */

/** `rt-3f9a2c1e-…` → `rt-3f9a…`. Short ids are shown whole. */
export function truncateRuntimeId(runtimeId: string): string {
  return runtimeId.length > 8 ? `${runtimeId.slice(0, 7)}…` : runtimeId
}

/**
 * BOTH engines mint `rt-<uuid>` — `format!("rt-{}", Uuid::new_v4())` in
 * node-core's `manager.rs` and python-core's `manager.rs` alike — and
 * code-runner's own error MESSAGES quote it by design (`RuntimeNotFound` is
 * "unknown runtime_id {id}"; a documented exception to the redaction
 * convention, since those go to the caller who already holds the id). The
 * console feed is not that caller, so every string that could embed one — an
 * error message, a line of program output, a bus function id — runs through
 * this before it is rendered.
 *
 * No `\b` anchors: hex digits are word characters, so an id glued to
 * `[A-Za-z0-9_]` (`<id>_worker`, `app::<id>_a`, `/work/<id>_out.json`) matches
 * neither boundary and would pass through whole. Matching the bare 39-char
 * shape unanchored can only redact MORE, never less — it is the capability
 * regardless of what touches it.
 */
const RUNTIME_ID_PATTERN = /rt-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi

/** Replace every `rt-<uuid>` substring of `text` with its truncated form. */
export function redactRuntimeIds(text: string): string {
  return text.replace(RUNTIME_ID_PATTERN, (id) => truncateRuntimeId(id))
}

/**
 * `redactRuntimeIds` over EVERY string in an arbitrary JSON-ish value —
 * object keys included. Objects, arrays, numbers, booleans and null keep
 * their shape; the input is never mutated.
 *
 * This is what the renderers hand the console as `redactRaw`: the card's
 * `raw json` tab renders the request/response verbatim and its copy button
 * copies them, so a card that shows only `RuntimeChip` has not contained the
 * capability until the raw value is filtered too.
 *
 * `seen` is the current PATH, not every visited node: a value referenced
 * twice is redacted twice (correct), while a cycle collapses to
 * `'[circular]'` rather than hanging the console. JSON off the wire cannot be
 * cyclic, but `redactRaw` must be total for whatever it is handed.
 */
export function redactRuntimeIdsDeep(value: unknown, seen: WeakSet<object> = new WeakSet()): unknown {
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

async function copyText(text: string): Promise<boolean> {
  try {
    // Undefined outside a secure context (plain http:// over a LAN).
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch {
    /* fall through to the textarea path */
  }
  try {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    const ok = document.execCommand('copy')
    document.body.removeChild(ta)
    return ok
  } catch {
    return false
  }
}

/**
 * The truncated runtime-id chip. The full id is a capability, so it is only
 * ever handed over on an explicit click (copied to the clipboard), never
 * printed into the feed.
 */
export function RuntimeChip({ runtimeId }: { runtimeId: string }) {
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle')

  const copy = useCallback(() => {
    void copyText(runtimeId).then((ok) => {
      setState(ok ? 'copied' : 'failed')
      window.setTimeout(() => setState('idle'), 1400)
    })
  }, [runtimeId])

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          className={`cr-ui-rt${state === 'idle' ? '' : ` ${state}`}`}
          onClick={copy}
          // Never the id itself: an aria-label puts it in the DOM and reads it
          // aloud, which is exactly the exposure the truncation prevents.
          aria-label="copy the full runtime id"
        >
          <span className="k">runtime </span>
          {truncateRuntimeId(runtimeId)}
          {state === 'idle' ? null : (
            <span className="cr-ui-rt-flash">{state === 'copied' ? ' copied' : ' copy failed'}</span>
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent>
        A runtime id is a capability — anyone holding it can run into or tear down that runtime, so only a prefix is
        shown. Click to copy the full id.
      </TooltipContent>
    </Tooltip>
  )
}

/* --- card frame -------------------------------------------------------- */

/**
 * The frame every code-runner card shares: op pill, caller-supplied chips,
 * the "code-runner ui" attribution tag (so an override is distinguishable
 * from first-party rendering), and the body.
 */
export function CardShell({
  op,
  running,
  tag = 'code-runner ui',
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
 * is the user's own traceback or runtime message, not a system error.
 */
export function Stream({ label, text, tone = 'out' }: { label: string; text: string; tone?: 'out' | 'err' }) {
  const [expanded, setExpanded] = useState(false)
  if (text.length === 0) return null

  const safe = redactRuntimeIds(text)
  const lines = safe.split('\n')
  const long = lines.length > STREAM_CLAMP_LINES || safe.length > STREAM_CLAMP_CHARS
  const collapsed = long && !expanded
  const shown = collapsed ? lines.slice(0, STREAM_CLAMP_LINES).join('\n').slice(0, STREAM_CLAMP_CHARS) : safe

  return (
    <div className={`cr-ui-stream ${tone}`}>
      <div className="cr-ui-stream-label">{label}</div>
      <pre className="cr-ui-stream-body">{shown}</pre>
      {long ? (
        <button type="button" className="cr-ui-toggle" onClick={() => setExpanded((v) => !v)}>
          {collapsed ? `expand · ${lines.length} lines, ${safe.length} chars` : 'collapse'}
        </button>
      ) : null}
    </div>
  )
}

/* --- completion value --------------------------------------------------- */

/** Characters of pretty-printed JSON shown before the block collapses. */
const RESULT_CLAMP_CHARS = 1200

/**
 * The run's `result` — the completion value, which is what makes
 * code-runner's wire a SUPERSET of `sandbox-code-runner`'s rather than a copy
 * of it.
 *
 * `result` is always present on the wire and never skipped, because a null
 * result is information ("the code returned nothing"), not an absence
 * (run.rs). So this renders `null` explicitly rather than hiding the section:
 * a reader who returned something and got null back needs to see that, and it
 * is usually the `return`-vs-`result` convention mismatch that `hint`
 * explains.
 *
 * The value is arbitrary tenant JSON, so it goes through
 * `redactRuntimeIdsDeep` before being stringified — a script that calls back
 * into the engine can return a runtime id, and the string form would
 * otherwise carry the capability into the feed.
 */
export function ResultValue({
  value,
  hint,
}: {
  value: unknown
  /** The engine's return convention, shown only when the result is null. */
  hint?: string
}) {
  const [expanded, setExpanded] = useState(false)

  const isNull = value === null || value === undefined
  let text: string
  try {
    text = JSON.stringify(redactRuntimeIdsDeep(value) ?? null, null, 2) ?? 'null'
  } catch {
    // Non-serializable (a BigInt survives the wire as a string, but be total).
    text = String(value)
  }

  const long = text.length > RESULT_CLAMP_CHARS
  const collapsed = long && !expanded
  const shown = collapsed ? text.slice(0, RESULT_CLAMP_CHARS) : text

  return (
    <div className="cr-ui-section">
      <div className="cr-ui-section-label">result</div>
      {isNull ? (
        <div className="cr-ui-result-null">
          null — the code returned nothing
          {hint ? <span className="cr-ui-result-hint"> · {hint}</span> : null}
        </div>
      ) : (
        <>
          <pre className="cr-ui-result-body">{shown}</pre>
          {long ? (
            <button type="button" className="cr-ui-toggle" onClick={() => setExpanded((v) => !v)}>
              {collapsed ? `expand · ${text.length} chars` : 'collapse'}
            </button>
          ) : null}
        </>
      )}
    </div>
  )
}

/* --- exit status -------------------------------------------------------- */

/**
 * The one-line verdict on a run: exit code, what it means, how long it took.
 *
 * A non-zero exit is NOT an error. code-runner reserves errors for
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
  // An omitted `success` never gets promoted to a claim either way. What must
  // NOT happen is treating `success: false` on a 0 exit code as the ordinary
  // "non-zero exit" case — the note has to describe what is actually shown
  // (`exit 0`), not contradict it.
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
      {durationMs === undefined ? null : <span className="cr-ui-exit-dur">{durationMs}ms</span>}
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
 * Pull the message out of a code-runner error output. Checked at both the raw
 * value and its unwrapped envelope — the same two places every renderer's
 * `isErrorOutput` check looks — since which level carries the `{ error }` key
 * depends on the path the failure took.
 */
export function errorInfo(output: unknown): { message: string } | undefined {
  const direct = asRecord(output)
  const nested = asRecord(unwrapEnvelope(output))
  const rec = isErrorOutput(direct) ? direct : isErrorOutput(nested) ? nested : undefined
  if (!rec) return undefined
  const err = rec.error
  const errObj = asRecord(err)
  const message =
    typeof err === 'string' ? err : typeof errObj?.message === 'string' ? errObj.message : JSON.stringify(err)
  return { message }
}

/**
 * The error card every code-runner renderer shows instead of falling through
 * to the console's default error view: code-runner's error MESSAGES carry the
 * runtime_id capability by design (`unknown runtime_id {id}` — error.rs), so
 * the unredacted default view would print it verbatim on an ordinary mistake.
 *
 * This is an infrastructure failure — the runtime is gone, an engine refused,
 * the deadline blew, a cap was hit. A script that merely exited non-zero
 * never lands here. A call the approval gate DENIED never lands here either —
 * see `DeniedCard`.
 */
export function ErrorCard({ op, runtimeId, message }: { op: string; runtimeId?: string; message: string }) {
  return (
    <CardShell op={op} chips={runtimeId ? <RuntimeChip runtimeId={runtimeId} /> : null}>
      <div className="cr-ui-msg-note cr-ui-alert">{redactRuntimeIds(message)}</div>
    </CardShell>
  )
}

/* --- gate denials ------------------------------------------------------- */

/**
 * A deny/timeout resolution from the approval gate rides in `error.details`
 * as the gate's DenialEnvelope (`{ status: 'denied', denied_by, reason,
 * args_excerpt, … }`). This mirrors the console's own `isDeniedOutput` so the
 * two agree on what a denial looks like — checked at both the raw value and
 * its unwrapped envelope, the same two places `errorInfo` looks.
 */
export function isDeniedOutput(output: unknown): boolean {
  const direct = asRecord(output)
  const nested = asRecord(unwrapEnvelope(output))
  const rec = isErrorOutput(direct) ? direct : isErrorOutput(nested) ? nested : undefined
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
  const rec = (isErrorOutput(direct) ? direct : nested) as Record<string, unknown>
  const err = asRecord(rec.error)
  const details = asRecord(err?.details) ?? {}
  const reason =
    typeof details.reason === 'string'
      ? details.reason
      : typeof err?.message === 'string'
        ? err.message
        : 'denied at the gate'
  const deniedBy = typeof details.denied_by === 'string' ? details.denied_by : undefined
  return { reason, deniedBy }
}

/**
 * A gate denial: the call was stopped at the approval gate and never reached
 * an engine. Distinct from `ErrorCard` on purpose — `ErrorCard` means an
 * infrastructure failure AFTER the call landed, and its RuntimeChip would
 * wrongly imply a runtime was involved in a call that never reached one. The
 * envelope's `args_excerpt` can carry the runtime_id capability (a caller is
 * free to pass one as an argument), so this never falls through to the
 * console's default view and never renders anything but the redacted
 * `reason` — the envelope itself is not printed.
 */
export function DeniedCard({ op, reason, deniedBy }: { op: string; reason: string; deniedBy?: string }) {
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
