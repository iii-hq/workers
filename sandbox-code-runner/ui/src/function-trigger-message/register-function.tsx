/**
 * Injected function-trigger renderer for
 * `sandbox-code-runner::register_function`.
 *
 * The default card prints the request as JSON, which turns `source` — the
 * whole point of the call — into one escaped single-line string, and buries
 * the two things that decide whether the registration did what the caller
 * meant:
 *
 *   - the NAMESPACE the id claims (the segment before `::`). The first
 *     registration in a namespace claims it and every later id there must
 *     share it — AND must share its `lang`: sandbox-code-runner keeps one
 *     persistent runtime per (namespace, lang) automatically (manager.rs
 *     `namespace_of` + `namespace_runtime` + `reserve`), so this is the
 *     common surprise — it gets its own line, not a substring of a blob.
 *   - whether the source actually defines `handler(payload)`. That convention
 *     is what the runner loads and calls (register.rs's `source` doc); a
 *     source without it registers fine and then fails on every call.
 *
 * ONE function per call: unlike node-engine's `functions: [...]`, this request
 * carries a single `function_id` + `source` pair.
 *
 * NO `runtime_id` ON THIS WIRE AT ALL: the runtime backing a namespace is an
 * implementation detail this call never sees or names — `lang` (required)
 * decides which runner it needs, and sandbox-code-runner creates or reuses
 * that namespace's runtime itself. `lang` IS on the request, unlike before
 * this redesign, so the source is highlighted honestly rather than guessed —
 * `guessPrism` below is now only a fallback for a malformed request missing
 * it.
 *
 * No capability lives on this request either, but a caller is free to name a
 * function, description, or source after a runtime id it holds from
 * elsewhere (e.g. planting source that calls back into a `keep: true` run's
 * runtime) — every free-text field still goes through `redactRuntimeIds` as
 * belt-and-braces, and errors render through this card's own `ErrorCard`
 * rather than falling through to the console's default view.
 */

import {
  CodeHighlight,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
} from '@iii-dev/console-ui'
import { useState } from 'react'
import {
  asRecord,
  CardShell,
  DeniedCard,
  ErrorCard,
  deniedInfo,
  errorInfo,
  langToPrism,
  opName,
  redactRuntimeIds,
  unwrapEnvelope,
} from '../lib/shared'

const FUNCTION_ID = 'sandbox-code-runner::register_function'

/** Lines of source shown before the block collapses behind a toggle… */
const COLLAPSE_AFTER = 14
/** …and a character ceiling, for the one-line source a bundler emitted. */
const SOURCE_CHAR_CAP = 2000

/* --- request ------------------------------------------------------------- */

/** The free-text fields register.rs's `RegisterRequest` declares — `lang` is
 * excluded: it is a closed enum, not free text, checked separately below. */
const STRING_FIELDS = ['function_id', 'source', 'description'] as const

type Lang = 'node' | 'python'

interface RegisterRequest {
  functionId?: string
  source?: string
  description?: string
  /** Required on the wire; `undefined` here means missing or invalid — only
   * ever a value `langToPrism` can honestly map. */
  lang?: Lang
  /** Fields the request carried with a non-string value — surfaced, not dropped. */
  malformed: string[]
}

function parseRequest(input: unknown): RegisterRequest {
  const obj = asRecord(input) ?? {}
  const str = (k: string) => (typeof obj[k] === 'string' ? obj[k] : undefined)
  return {
    functionId: str('function_id'),
    source: str('source'),
    description: str('description'),
    lang: obj.lang === 'node' || obj.lang === 'python' ? obj.lang : undefined,
    malformed: STRING_FIELDS.filter(
      (k) => k in obj && obj[k] !== null && typeof obj[k] !== 'string',
    ),
  }
}

/** The request's own `lang`, when present. */
function LangChip({ req }: { req: RegisterRequest }) {
  if (!req.lang) return null
  return (
    <span className="cr-ui-chip">
      <span className="k">lang </span>
      {req.lang}
    </span>
  )
}

/**
 * The namespace this id claims: `app::greet` → `app::`. `undefined` when the
 * id has no `::` or nothing after it — exactly the two shapes `namespace_of`
 * (manager.rs) refuses, so the card can say so before the call lands.
 */
function namespaceOf(functionId: string): string | undefined {
  const i = functionId.indexOf('::')
  if (i <= 0 || i + 2 >= functionId.length) return undefined
  return `${functionId.slice(0, i)}::`
}

/**
 * A Prism id when the source is unmistakably one language, `undefined`
 * otherwise → rendered as `text`, i.e. unhighlighted. Only reached when the
 * request's own `lang` is missing or invalid — `lang` is required on this
 * wire, so this is a fallback for a malformed request, not the normal case.
 *
 * This is a HINT, never a claim. `def` is checked first because it is the
 * one marker JavaScript cannot produce.
 */
function guessPrism(source: string): string | undefined {
  if (/^[ \t]*(async[ \t]+)?def[ \t]+\w/m.test(source)) return 'python'
  if (/\bexport\s+function\b|\bfunction\s+\w|=>/.test(source))
    return 'javascript'
  return undefined
}

/** The approval gate clips every string to 256 code points + `…`. */
function looksClipped(value: string | undefined): boolean {
  return value?.endsWith('…') === true
}

/* --- body pieces --------------------------------------------------------- */

function MalformedFields({ names }: { names: readonly string[] }) {
  if (names.length === 0) return null
  return (
    <div className="cr-ui-msg-note cr-ui-warn">
      · the request carries a non-string {names.join(', ')} — the worker rejects
      it
    </div>
  )
}

function Head({
  req,
  resId,
  status,
}: {
  req: RegisterRequest
  /**
   * The response's `function_id` — the id actually live on the bus. Falls
   * back into the display when the request carried none, so a card can
   * never say "no function_id in the request" beside a green `registered`
   * badge while hiding the id that is actually callable. `undefined` in the
   * pending/running states, where there is no response yet.
   */
  resId?: string
  /** Undefined until the response settles, or when it carried no flag. */
  status?: 'live' | 'refused'
}) {
  // The request is still the id of record when it has one — a caller-echoed
  // response could in principle disagree, and `SettledView`'s mismatch note
  // covers that. `resId` only fills the gap when the request had nothing to
  // show at all.
  const displayId = req.functionId ?? resId
  const ns = displayId ? namespaceOf(displayId) : undefined
  return (
    <div className="cr-ui-section">
      <div className="cr-ui-section-label">function</div>
      <div className="cr-register-function-head">
        <span
          className={`cr-register-function-id${req.functionId ? '' : ' cr-ui-warn'}`}
        >
          {req.functionId
            ? redactRuntimeIds(req.functionId)
            : resId
              ? `${redactRuntimeIds(resId)} (from the response — the request carried no function_id)`
              : 'no function_id in the request'}
        </span>
        {status === 'live' ? (
          <span className="cr-register-function-status live">registered</span>
        ) : null}
        {status === 'refused' ? (
          <span className="cr-register-function-status refused">
            not registered
          </span>
        ) : null}
      </div>
      {displayId ? (
        <div className={`cr-register-function-ns${ns ? '' : ' cr-ui-warn'}`}>
          {ns ? (
            <>
              claims <code>{redactRuntimeIds(ns)}</code> for this runtime — the
              first registration takes the namespace and every later id must
              share it
            </>
          ) : (
            <>
              no namespace — an id must look like <code>app::name</code>, which
              this one does not
            </>
          )}
        </div>
      ) : null}
      <div
        className={`cr-register-function-desc${req.description ? '' : ' cr-ui-warn'}`}
      >
        {req.description
          ? redactRuntimeIds(req.description)
          : 'no description — engine::functions::info will show callers nothing'}
      </div>
    </div>
  )
}

/**
 * The source, clamped by both line count and character count (a one-line
 * bundle has no newlines to clamp on) and further capped in height by
 * `.cr-ui-code`.
 *
 * `clipped` means the string is the approval gate's excerpt, not the program:
 * its line count is meaningless, so the toggle drops the "+N more lines"
 * arithmetic.
 */
function SourceSection({
  source,
  lang,
  clipped,
}: {
  source: string
  lang?: Lang
  clipped: boolean
}) {
  const [expanded, setExpanded] = useState(false)
  const safe = redactRuntimeIds(source)
  const lines = safe.split('\n')
  const long = lines.length > COLLAPSE_AFTER || safe.length > SOURCE_CHAR_CAP
  const collapsed = long && !expanded
  const shown = collapsed
    ? lines.slice(0, COLLAPSE_AFTER).join('\n').slice(0, SOURCE_CHAR_CAP)
    : safe
  const hidden = lines.length - COLLAPSE_AFTER
  const known = langToPrism(lang)
  const guessed = known === undefined ? guessPrism(safe) : undefined
  const prism = known ?? guessed

  return (
    <div className="cr-ui-section">
      <div className="cr-ui-section-label">
        {clipped ? 'source (excerpt)' : 'source'}
      </div>
      <div className="cr-ui-code">
        <CodeHighlight code={shown} language={prism ?? 'text'} />
      </div>
      {long ? (
        <button
          type="button"
          className="cr-ui-toggle"
          onClick={() => setExpanded((v) => !v)}
        >
          {!collapsed
            ? 'collapse'
            : clipped || hidden <= 0
              ? `expand · ${safe.length} chars`
              : `+ ${hidden} more of ${lines.length} lines`}
        </button>
      ) : null}
      {guessed ? (
        <div className="cr-register-function-lang">
          highlighted as {guessed} — guessed from the source; this request's
          lang field is missing or invalid, so the runner it will actually
          run under is unconfirmed
        </div>
      ) : null}
    </div>
  )
}

/**
 * The source has to DEFINE `handler(payload)` — the runner loads the file and
 * calls `handler` (register.rs). Quiet, because a substring check is not a
 * parser: it is an advisory, never a verdict. Suppressed on a clipped excerpt,
 * where the definition may simply be past the cut.
 */
function HandlerAdvisory({
  source,
  clipped,
}: {
  source: string
  clipped: boolean
}) {
  if (clipped || /\bhandler\b/.test(source)) return null
  return (
    <div className="cr-ui-msg-note">
      · nothing named `handler` in this source — the runner loads the file and
      calls `handler(payload)`, so a source that never defines it registers and
      then fails on every call
    </div>
  )
}

/** Source block plus its advisories, or the placeholder when there is none. */
function SourceBlock({
  req,
  clipped,
}: {
  req: RegisterRequest
  clipped: boolean
}) {
  if (req.source === undefined) {
    return (
      <div className="cr-ui-msg-note cr-ui-warn">
        · no source in the request — the worker rejects a registration without
        one
      </div>
    )
  }
  if (req.source.length === 0) {
    return (
      <div className="cr-ui-msg-note cr-ui-warn">
        · empty source — the worker rejects it; it must define handler(payload)
      </div>
    )
  }
  return (
    <>
      <SourceSection source={req.source} lang={req.lang} clipped={clipped} />
      <HandlerAdvisory source={req.source} clipped={clipped} />
    </>
  )
}

/* --- cards --------------------------------------------------------------- */

function SettledView({ message }: { message: FunctionTriggerMessage }) {
  const req = parseRequest(message.input)
  const res = asRecord(unwrapEnvelope(message.output)) ?? {}
  const registered =
    typeof res.registered === 'boolean' ? res.registered : undefined
  const resId =
    typeof res.function_id === 'string' ? res.function_id : undefined
  const mismatch =
    resId !== undefined &&
    req.functionId !== undefined &&
    resId !== req.functionId

  return (
    <CardShell op={opName(message.functionId)} chips={<LangChip req={req} />}>
      <Head
        req={req}
        resId={resId}
        status={
          registered === undefined ? undefined : registered ? 'live' : 'refused'
        }
      />
      {registered === undefined ? (
        <div className="cr-ui-msg-note cr-ui-warn">
          · the response carried no `registered` flag, so whether this id is on
          the bus is unconfirmed
        </div>
      ) : null}
      {registered === false ? (
        <div className="cr-ui-msg-note cr-ui-warn">
          · the response reports `registered: false` — the function is not
          callable on the bus
        </div>
      ) : null}
      {mismatch ? (
        <div className="cr-ui-msg-note cr-ui-warn">
          · the response registered {redactRuntimeIds(resId)}, not the id in the
          request
        </div>
      ) : null}
      <MalformedFields names={req.malformed} />
      <SourceBlock req={req} clipped={false} />
    </CardShell>
  )
}

/**
 * Not settled yet: in flight, or held at the approval gate.
 *
 * At the gate the `input` may be the gate's `arguments_excerpt` — every string
 * clipped to 256 code points with a trailing `…`. This is the one card gating
 * arbitrary code publication onto the bus, so a clipped source is labelled an
 * excerpt rather than presented as the whole program.
 */
function PendingView({
  message,
  running,
}: {
  message: FunctionTriggerMessage
  running: boolean
}) {
  const req = parseRequest(message.input)
  const clipped = looksClipped(req.source)
  const anyClipped =
    clipped || looksClipped(req.description) || looksClipped(req.functionId)

  return (
    <CardShell
      op={opName(message.functionId)}
      running={running}
      chips={<LangChip req={req} />}
    >
      <div className={`cr-ui-msg-note${running ? ' pulse' : ''}`}>
        {running ? '· registering…' : '· will register this function:'}
      </div>
      {anyClipped ? (
        <div className="cr-ui-msg-note cr-ui-warn">
          · the approval gate clips strings to 256 characters — anything ending
          in … is partial
        </div>
      ) : null}
      <Head req={req} />
      <MalformedFields names={req.malformed} />
      <SourceBlock req={req} clipped={clipped} />
    </CardShell>
  )
}

export function createRegisterFunctionRenderer(
  host: Host,
): FunctionTriggerRenderer {
  void host // this card reads nothing off the host
  const isMatch = (functionId: string) => functionId === FUNCTION_ID

  const render = (
    message: FunctionTriggerMessage,
    running: boolean,
  ): React.ReactNode | null => {
    if (!isMatch(message.functionId)) return null
    // The host draws the approval bar around `tryRenderPreview`.
    if (message.pendingApproval) return null
    // Not a record — e.g. a double-encoded (stringified) payload, which the
    // default card knows how to unpack and this one does not. Fall through
    // rather than asserting "no source in the request" about a call that
    // publishes arbitrary code.
    if (!asRecord(message.input)) return null
    if (running) return <PendingView message={message} running />
    // Denied at the gate — no source was ever published to the bus, so this
    // must not read as one of the infrastructure failures `ErrorCard` means.
    // Checked before `errorInfo`: a denial is also `'error' in output`-shaped.
    const denied = deniedInfo(message.output)
    if (denied) {
      return (
        <DeniedCard
          op={opName(message.functionId)}
          reason={denied.reason}
          deniedBy={denied.deniedBy}
        />
      )
    }
    // Our own error card, never the default one: this request has no
    // runtime_id, but a caller-chosen function_id/description/source could
    // still embed one from elsewhere, and errorInfo's message goes through
    // redactRuntimeIds either way — belt-and-braces over trusting the
    // default view.
    const err = errorInfo(message.output)
    if (err) {
      return <ErrorCard op={opName(message.functionId)} message={err.message} />
    }
    // No parseable response body — an aborted call, or a reloaded session
    // whose last call never paired. That is a normal state, not a completed
    // registration, so let the console's own "response · empty" card say so.
    if (!asRecord(unwrapEnvelope(message.output))) return null
    return <SettledView message={message} />
  }

  return {
    id: 'sandbox-code-runner/page.js#register-function',
    isMatch,
    tryRender: (message) => render(message, !!message.running),
    tryRenderRunning: (message) => render(message, true),
    // Worth a preview: the approver is about to publish code onto the bus, and
    // the namespace it claims is what they most need to check before saying
    // yes.
    tryRenderPreview: (message) =>
      isMatch(message.functionId) &&
      message.pendingApproval &&
      asRecord(message.input) ? (
        <PendingView message={message} running={false} />
      ) : null,
  }
}
