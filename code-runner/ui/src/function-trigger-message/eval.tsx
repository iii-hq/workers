/**
 * Injected function-trigger renderer for `code-runner::eval`.
 *
 * The default card prints the request as JSON, which turns `code` into one
 * escaped single-line string and `stdout`/`stderr` into two more — unreadable,
 * and this is the worker's highest-traffic call. Here the source is
 * highlighted in the language it will actually run as, and the response is
 * rendered as what it is: a PROCESS result.
 *
 * What this card exists to make obvious:
 *
 *  - A NON-ZERO EXIT IS NOT AN ERROR. code-runner reserves errors for
 *    infrastructure failures (`error.rs`); a script that throws comes back as
 *    an ordinary response with its own compiler/runtime message in `stderr`.
 *    So a failing exit reads as "your code exited N, here is stderr" (warn),
 *    never as a system fault (alert) — see `ExitStatus` in ../lib/shared.
 *  - Which runtime the call ran in, and for how long. Three cases, and the
 *    card names exactly which one applies: a `runtime_id` in the request
 *    means a REUSE of a runtime the caller already holds; `keep: true` with
 *    none means the call is minting one to keep; neither means ONE-SHOT —
 *    the default — which boots a VM, runs the code, and destroys it before
 *    this response is even sent. A one-shot response carries no
 *    `runtime_id` (there is nothing left to address), which is why
 *    `EvalResponse.runtime_id` is optional on the wire now.
 *  - `network` — create-time only, and the flag that makes `npm install` /
 *    `pip install` possible inside the guest. `sandbox::run` — which now
 *    backs both the one-shot and `keep: true` paths — has no way to enable
 *    it at all, so `network: true` without an explicit `runtime_id` is
 *    always refused by the worker, never silently ignored. It is the
 *    security-relevant field of the request, so it is a chip rather than a
 *    boolean buried in JSON. See `NetworkChip` for what it may and may not
 *    claim.
 *  - That `runtime_id` is a capability: only ever shown through `RuntimeChip`
 *    (truncated, full value on an explicit copy), and every other string on
 *    the card — stdout, stderr, an error message, even the submitted source —
 *    routed through `redactRuntimeIds` first.
 *
 * Error outputs render their own compact card rather than falling through:
 * code-runner's error messages quote the runtime_id BY DESIGN
 * (`unknown runtime_id {id}`, `runtime {id} expired: …` — error.rs), so the
 * console's default view would print the capability verbatim on an ordinary
 * mistake.
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
  ExitStatus,
  deniedInfo,
  errorInfo,
  langToPrism,
  opName,
  RuntimeChip,
  redactRuntimeIds,
  Stream,
  TimeoutChip,
  unwrapEnvelope,
} from '../lib/shared'

const FUNCTION_ID = 'code-runner::eval'

/** Lines of source shown before the block collapses behind a toggle… */
const CODE_CLAMP_LINES = 14
/** …and a character ceiling, for the one-line 400 KB blob a minifier emits. */
const CODE_CLAMP_CHARS = 2000

type Lang = 'node' | 'python'

interface EvalRequest {
  code?: string
  runtimeId?: string
  /** Only ever a value `langToPrism` can honestly map; anything else is dropped
   * rather than echoed as a claim about what will execute. */
  lang?: Lang
  /** Only meaningful when `runtimeId` is absent — see `Chips`. */
  keep: boolean
  network: boolean
  timeoutMs?: number
}

function parseRequest(input: unknown): EvalRequest {
  const obj = asRecord(input) ?? {}
  return {
    code: typeof obj.code === 'string' ? obj.code : undefined,
    runtimeId: typeof obj.runtime_id === 'string' ? obj.runtime_id : undefined,
    lang: obj.lang === 'node' || obj.lang === 'python' ? obj.lang : undefined,
    keep: obj.keep === true,
    network: obj.network === true,
    timeoutMs: typeof obj.timeout_ms === 'number' ? obj.timeout_ms : undefined,
  }
}

/**
 * Header chips. `runtimeId` is the effective one (the response's, falling back
 * to the request's) and is absent on the one-shot path — nothing was left
 * running to address.
 */
function Chips({
  req,
  runtimeId,
}: {
  req: EvalRequest
  runtimeId?: string
}) {
  return (
    <>
      {runtimeId ? <RuntimeChip runtimeId={runtimeId} /> : null}
      {req.runtimeId ? (
        <span className="cr-ui-chip">reused runtime</span>
      ) : req.keep ? (
        <span
          className="cr-ui-chip cr-eval-fresh"
          title="No runtime_id, keep: true: boots a fresh VM and leaves it running — the response's runtime_id addresses it for later evals."
        >
          keeps the VM
        </span>
      ) : (
        <span
          className="cr-ui-chip cr-eval-fresh"
          title="No runtime_id, no keep: one-shot — boots a fresh VM, runs the code, and destroys the VM before this response is sent. Nothing persists: no files, no installed packages."
        >
          one-shot
        </span>
      )}
      {req.lang ? (
        <span className="cr-ui-chip">
          <span className="k">lang </span>
          {req.lang}
        </span>
      ) : null}
      <NetworkChip req={req} />
      <TimeoutChip ms={req.timeoutMs} />
    </>
  )
}

/**
 * `network` is create-time only ("Ignored when `runtime_id` is set" —
 * eval.rs), so on an explicitly reused runtime it is reported as ignored
 * rather than as a capability this eval has.
 *
 * Without a `runtime_id`, `sandbox::run` — which now backs both the
 * one-shot and `keep: true` paths — has no way to enable networking at
 * all, so the worker always REFUSES `network: true` there rather than
 * silently dropping it. `network: false` needs no chip: it is simply
 * guaranteed, the same way it always was on this path.
 */
function NetworkChip({ req }: { req: EvalRequest }) {
  if (!req.runtimeId) {
    if (!req.network) return null
    return (
      <span
        className="cr-ui-chip cr-eval-net off"
        title="Neither a one-shot eval nor keep: true can create a networked VM — sandbox::run has no network flag at all. This request will be refused; pass an explicit runtime_id for a runtime that already has network."
      >
        <span className="k">network </span>
        refused: no runtime_id
      </span>
    )
  }
  if (!req.network) return null
  return (
    <span className="cr-ui-chip cr-eval-net off">
      <span className="k">network </span>
      ignored on reuse
    </span>
  )
}

/**
 * The submitted source, highlighted as the language it runs as and clamped so
 * a generated 5000-line program cannot own the viewport.
 *
 * `lang` is `undefined` on a reuse that omitted it (the language belongs to
 * the runtime, and the request does not say) — that renders unhighlighted
 * rather than guessing, since a wrong language reads as a claim.
 *
 * `clipped` means the string came from the approval gate's
 * `arguments_excerpt`, already truncated: its line count is not the program's,
 * so no count is quoted from it.
 */
function CodeSection({
  code,
  lang,
  clipped,
}: {
  code: string
  lang?: Lang
  clipped?: boolean
}) {
  const [expanded, setExpanded] = useState(false)

  // Source can embed a runtime id — a script that calls back into the engine
  // carries one as a literal. The feed is not the caller that already holds it.
  const safe = redactRuntimeIds(code)
  if (safe.trim().length === 0) {
    return <div className="cr-ui-msg-note">· empty code — nothing to run</div>
  }

  const lines = safe.split('\n')
  const long = lines.length > CODE_CLAMP_LINES || safe.length > CODE_CLAMP_CHARS
  const collapsed = long && !expanded
  const shown = collapsed
    ? lines.slice(0, CODE_CLAMP_LINES).join('\n').slice(0, CODE_CLAMP_CHARS)
    : safe

  return (
    <div className="cr-ui-section">
      <div className="cr-ui-section-label">
        {clipped ? 'code (excerpt)' : 'code'}
      </div>
      <div className="cr-ui-code">
        <CodeHighlight code={shown} language={langToPrism(lang) ?? 'text'} />
      </div>
      {long ? (
        <button
          type="button"
          className="cr-ui-toggle"
          onClick={() => setExpanded((v) => !v)}
        >
          {collapsed
            ? clipped
              ? 'expand'
              : `expand · ${lines.length} lines, ${safe.length} chars`
            : 'collapse'}
        </button>
      ) : null}
    </div>
  )
}

/**
 * One process stream. A present-but-not-a-string field is a malformed
 * response, which gets a placeholder — dropping it silently would show a
 * card that quietly disagrees with the payload.
 */
function StreamOrNote({
  label,
  value,
  tone,
}: {
  label: string
  value: unknown
  tone?: 'out' | 'err'
}) {
  if (typeof value === 'string') {
    return <Stream label={label} text={value} tone={tone} />
  }
  if (value === undefined || value === null) return null
  return (
    <div className="cr-ui-msg-note cr-ui-warn">
      · {label} was not a string ({typeof value}) — malformed response
    </div>
  )
}

function SettledView({ message }: { message: FunctionTriggerMessage }) {
  const req = parseRequest(message.input)
  const res = asRecord(unwrapEnvelope(message.output)) ?? {}
  const runtimeId =
    typeof res.runtime_id === 'string' ? res.runtime_id : req.runtimeId

  return (
    <CardShell
      op={opName(message.functionId)}
      chips={<Chips req={req} runtimeId={runtimeId} />}
    >
      {/* The verdict first: in a chat feed you want "did it run?" before you
          want the source, and a failing exit points at the stderr below. */}
      <ExitStatus
        exitCode={typeof res.exit_code === 'number' ? res.exit_code : undefined}
        success={typeof res.success === 'boolean' ? res.success : undefined}
        durationMs={
          typeof res.duration_ms === 'number' ? res.duration_ms : undefined
        }
      />
      {req.code === undefined ? (
        <div className="cr-ui-msg-note">· the request carried no code</div>
      ) : (
        <CodeSection code={req.code} lang={req.lang} />
      )}
      <StreamOrNote label="stdout" value={res.stdout} />
      <StreamOrNote label="stderr" value={res.stderr} tone="err" />
      {res.stdout === '' && res.stderr === '' ? (
        <div className="cr-ui-msg-note">· no output on stdout or stderr</div>
      ) : null}
    </CardShell>
  )
}

/** In-flight: what is being run, and where. No verdict to show yet. */
function RunningView({ message }: { message: FunctionTriggerMessage }) {
  const req = parseRequest(message.input)
  return (
    <CardShell
      op={opName(message.functionId)}
      running
      chips={<Chips req={req} runtimeId={req.runtimeId} />}
    >
      <div className="cr-ui-msg-note pulse">· running…</div>
      {req.code === undefined ? null : (
        <CodeSection code={req.code} lang={req.lang} />
      )}
    </CardShell>
  )
}

/**
 * Pending approval: the source that is about to run, before saying yes.
 *
 * The approval gate may hand this renderer `arguments_excerpt` instead of the
 * real request — every string clipped to 256 code points with a trailing `…`
 * (approval-gate/src/redact.rs `ARGS_EXCERPT_LEN_CAP`). A trailing `…` is that
 * marker, so the code is labeled a possibly-partial excerpt rather than
 * presented as the whole program on the one card gating arbitrary code
 * execution.
 */
function PreviewView({
  message,
  code,
}: {
  message: FunctionTriggerMessage
  code: string
}) {
  const req = parseRequest(message.input)
  const clipped = code.endsWith('…')
  return (
    <CardShell
      op={opName(message.functionId)}
      chips={<Chips req={req} runtimeId={req.runtimeId} />}
    >
      <div className="cr-ui-msg-note">will run this code:</div>
      {clipped ? (
        <div className="cr-ui-msg-note cr-ui-warn">
          · clipped to 256 characters by the approval gate — an excerpt, not
          necessarily the whole program
        </div>
      ) : null}
      <CodeSection code={code} lang={req.lang} clipped={clipped} />
    </CardShell>
  )
}

export function createEvalRenderer(host: Host): FunctionTriggerRenderer {
  void host // the eval card reads nothing off the host

  const render = (
    message: FunctionTriggerMessage,
    running: boolean,
  ): React.ReactNode | null => {
    if (message.functionId !== FUNCTION_ID) return null
    if (message.pendingApproval) return null // tryRenderPreview handles it
    // Not a record — e.g. a double-encoded (stringified) payload, which the
    // default card knows how to unpack and this one does not. Fall through
    // rather than asserting anything about a request we cannot read.
    if (!asRecord(message.input)) return null
    if (running) return <RunningView message={message} />
    // Denied at the gate — this call never reached a runtime, so it must not
    // read as one of the infrastructure failures `ErrorCard` means. Checked
    // before `errorInfo`: a denial is also `'error' in output`-shaped.
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
    // Our own error card, never a fall-through: code-runner's error messages
    // carry the runtime_id capability by design (error.rs), so the console's
    // default view would print it unredacted.
    const err = errorInfo(message.output)
    if (err) {
      return (
        <ErrorCard
          op={opName(message.functionId)}
          runtimeId={parseRequest(message.input).runtimeId}
          message={err.message}
        />
      )
    }
    // No parseable response body — an aborted call, or a reloaded session
    // whose last call never paired. A normal state, and NOT a completed eval,
    // so let the console's own "response · empty" card have it rather than
    // asserting an exit status that never happened.
    if (!asRecord(unwrapEnvelope(message.output))) return null
    return <SettledView message={message} />
  }

  return {
    id: 'code-runner/page.js#eval',
    isMatch: (functionId) => functionId === FUNCTION_ID,
    tryRender: (message) => render(message, !!message.running),
    tryRenderRunning: (message) => render(message, true),
    tryRenderPreview: (message) => {
      if (!message.pendingApproval) return null
      if (message.functionId !== FUNCTION_ID) return null
      if (!asRecord(message.input)) return null
      // No readable `code`: the gate's default card shows the raw payload,
      // which is strictly more than this card could honestly claim on an
      // approval prompt for arbitrary code execution.
      const { code } = parseRequest(message.input)
      if (code === undefined) return null
      return <PreviewView message={message} code={code} />
    },
  }
}
