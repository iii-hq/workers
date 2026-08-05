/**
 * `sandbox-code-runner::teardown` — destroys one or more runtimes: it
 * unregisters every bus function they put on the bus and stops the sandbox
 * microVM(s) behind them (`RuntimeManager::destroy_runtime` drains in-flight
 * runs, unregisters, then calls `sandbox::stop` — manager.rs).
 *
 * The card answers one question: WHICH function ids stopped resolving. That is
 * the consequence a reader cannot recover from anywhere else. An empty
 * `unregistered` list is the COMMON case — a runtime that only ever ran code
 * registered nothing — so it reads as normal, never as a failure.
 *
 * TWO addressing modes, never both, never neither: `runtime_id` (a kept
 * run's runtime — `sandbox-code-runner::run keep=true`) or `namespace` (every
 * runtime, one per language, backing a `register_function` namespace). The
 * response echoes whichever one addressed the call, never both — see
 * `targetOf`.
 *
 * A preview IS offered even for this single-field-or-the-other request, for
 * the same reason the error card is rendered here rather than fallen through
 * to: the console's default card prints `runtime_id` in full, and a runtime
 * id is a capability (see ../lib/shared.tsx). The one case that still falls
 * through is a non-record `input` — see `tryRenderPreview`.
 */

import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
  Host,
} from '@iii-dev/console-ui'
import { useState } from 'react'
import {
  asRecord,
  CardShell,
  DeniedCard,
  ErrorCard,
  deniedInfo,
  errorInfo,
  redactRuntimeIds,
  RegisteredIds,
  RuntimeChip,
  unwrapEnvelope,
} from '../lib/shared'

const FUNCTION_ID = 'sandbox-code-runner::teardown'

/** Ids listed before the list collapses — a runtime may hold up to 64. */
const GONE_CLAMP = 12

type Target =
  | { kind: 'runtime'; id: string }
  | { kind: 'namespace'; name: string }

/**
 * Which of the two addressing modes a record (request or response) carries.
 * `runtime_id` wins if a malformed record somehow carried both non-empty —
 * the worker itself refuses that combination, so this is purely a display
 * tie-break, never a claim about what the worker did.
 */
function targetOf(value: unknown): Target | undefined {
  const rec = asRecord(value)
  const id = rec?.runtime_id
  if (typeof id === 'string' && id.length > 0) return { kind: 'runtime', id }
  const ns = rec?.namespace
  if (typeof ns === 'string' && ns.length > 0) return { kind: 'namespace', name: ns }
  return undefined
}

function TargetChip({ target }: { target?: Target }) {
  if (!target) return null
  if (target.kind === 'runtime') return <RuntimeChip runtimeId={target.id} />
  return (
    <span className="cr-ui-chip">
      <span className="k">namespace </span>
      {redactRuntimeIds(target.name)}
    </span>
  )
}

/**
 * `unregistered`, with non-string entries kept as visible placeholders rather
 * than filtered away — a dropped entry would understate the blast radius.
 * `undefined` when the field is absent or not an array, so the card can say
 * "the response didn't list them" instead of claiming zero.
 */
function goneIds(result: Record<string, unknown> | undefined) {
  const raw = result?.unregistered
  if (!Array.isArray(raw)) return undefined
  return raw.map((v, i) =>
    typeof v === 'string' ? v : `⟨malformed entry ${i + 1}⟩`,
  )
}

function SettledView({ message }: { message: FunctionTriggerMessage }) {
  const [expanded, setExpanded] = useState(false)
  const result = asRecord(unwrapEnvelope(message.output))
  const gone = goneIds(result)
  // Only `torn_down === false` is a claim; a missing field is not.
  const kept = result?.torn_down === false
  // The response echoes whichever target addressed the call; the request is
  // the fallback (e.g. an error response that carries neither).
  const target = targetOf(result) ?? targetOf(message.input)

  const collapsed = !!gone && gone.length > GONE_CLAMP && !expanded
  const shown = collapsed ? gone.slice(0, GONE_CLAMP) : (gone ?? [])

  return (
    <CardShell op="teardown" chips={<TargetChip target={target} />}>
      <div className="cr-ui-msg-note">
        <div className={kept ? 'cr-ui-warn' : undefined}>
          {kept
            ? '· the worker reported this was NOT torn down'
            : '· destroyed — its sandbox microVM(s) were stopped'}
        </div>
        <div>
          {gone === undefined
            ? '· the response did not list which function ids were unregistered'
            : gone.length === 0
              ? '· it had registered no functions, so nothing stopped resolving on the bus'
              : gone.length === 1
                ? '· 1 function id no longer resolves on the bus'
                : `· ${gone.length} function ids no longer resolve on the bus`}
        </div>
      </div>
      <div className="cr-teardown-gone">
        <RegisteredIds ids={shown} />
        {gone && gone.length > GONE_CLAMP ? (
          <div className="cr-teardown-more">
            <button
              type="button"
              className="cr-ui-toggle"
              onClick={() => setExpanded((v) => !v)}
            >
              {collapsed ? `expand · ${gone.length} ids` : 'collapse'}
            </button>
          </div>
        ) : null}
      </div>
    </CardShell>
  )
}

function RunningView({ message }: { message: FunctionTriggerMessage }) {
  // `input` is the only source of the target here, and a non-record one
  // simply means no chip — the card claims nothing either way. Falling
  // through instead would hand the raw request to the default card, which
  // prints the capability in full.
  return (
    <CardShell op="teardown" running chips={<TargetChip target={targetOf(message.input)} />}>
      <div className="cr-ui-msg-note pulse">· tearing down…</div>
    </CardShell>
  )
}

/** Pending approval: what is about to be destroyed. */
function PreviewView({ message }: { message: FunctionTriggerMessage }) {
  // The approval gate clips string arguments to 256 code points; a
  // `rt-<uuid>` is 39 and a namespace is short, so the value shown (and
  // copied) here is always whole.
  const target = targetOf(message.input)
  return (
    <CardShell op="teardown" chips={<TargetChip target={target} />}>
      <div className="cr-ui-msg-note">
        {target?.kind === 'namespace'
          ? 'will destroy every runtime backing this namespace — every function it registered stops resolving on the bus, and its sandbox microVM(s) are stopped'
          : 'will destroy this runtime — every function it registered stops resolving on the bus, and its sandbox microVM is stopped'}
      </div>
    </CardShell>
  )
}

export function createTeardownRenderer(host: Host): FunctionTriggerRenderer {
  void host
  const render = (
    message: FunctionTriggerMessage,
    running: boolean,
  ): React.ReactNode | null => {
    if (message.functionId !== FUNCTION_ID) return null
    if (message.pendingApproval) return null // tryRenderPreview handles it
    if (running) return <RunningView message={message} />
    // Denied at the gate — no runtime was ever touched, so this must not
    // read as one of the infrastructure failures `ErrorCard` means. Checked
    // before `errorInfo`: a denial is also `'error' in output`-shaped.
    const denied = deniedInfo(message.output)
    if (denied) {
      return (
        <DeniedCard op="teardown" reason={denied.reason} deniedBy={denied.deniedBy} />
      )
    }
    // Our own error card, never a fall-through: sandbox-code-runner's error MESSAGES
    // carry the runtime_id by design on the by-id path — `unknown runtime_id
    // {id}`, `runtime {id} expired: …` (error.rs) — and the default view
    // would print that capability verbatim on an ordinary stale-id mistake.
    // `ErrorCard` redacts the message either way.
    const err = errorInfo(message.output)
    if (err) {
      const target = targetOf(message.input)
      return (
        <ErrorCard
          op="teardown"
          runtimeId={target?.kind === 'runtime' ? target.id : undefined}
          message={err.message}
        />
      )
    }
    // No response body at all (aborted call, or a reloaded session whose last
    // call never paired). Fall through so the console shows its own
    // "response · empty" card — never assert a teardown that may not have
    // happened.
    if (!asRecord(unwrapEnvelope(message.output))) return null
    return <SettledView message={message} />
  }
  return {
    id: 'sandbox-code-runner/page.js#teardown',
    isMatch: (functionId) => functionId === FUNCTION_ID,
    tryRender: (message) => render(message, !!message.running),
    tryRenderRunning: (message) => render(message, true),
    // A non-record `input` falls through: the console's default card decodes
    // double-encoded payloads, and an approver of a destructive call must see
    // the real request rather than a card that quietly shows no target.
    tryRenderPreview: (message) =>
      message.pendingApproval &&
      message.functionId === FUNCTION_ID &&
      asRecord(message.input) ? (
        <PreviewView message={message} />
      ) : null,
  }
}
