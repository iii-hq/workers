/**
 * Injected function-trigger message renderer for `state::set` and
 * `state::get` — registered through `host.functionTriggers`, so it
 * dispatches BEFORE the console's built-in state family and overrides how
 * those two triggers render in chat and in the traces span tab.
 *
 * Deliberately narrow: other `state::*` ids (delete/update/list/…) return
 * null and fall through to the console's built-in rendering, as do error
 * outputs (the built-in family has richer error views). The card carries a
 * small "state ui" tag so an override is distinguishable from first-party
 * rendering at a glance.
 */

import {
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
  JsonHighlight,
} from '@iii-dev/console-ui'

const RENDERED = new Set(['state::set', 'state::get'])

/** `{ content: [...], details }` harness result envelope → details. */
function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}

function isErrorOutput(value: unknown): boolean {
  return (
    !!value &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    'error' in (value as Record<string, unknown>)
  )
}

interface StateRequest {
  scope?: string
  key?: string
  value?: unknown
}

function parseRequest(input: unknown): StateRequest {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return {}
  const obj = input as Record<string, unknown>
  return {
    scope: typeof obj.scope === 'string' ? obj.scope : undefined,
    key: typeof obj.key === 'string' ? obj.key : undefined,
    // `data` is an accepted alias for `value` on state::set.
    value: 'value' in obj ? obj.value : obj.data,
  }
}

function isEmptyValue(v: unknown): boolean {
  if (v === null || v === undefined) return true
  if (typeof v === 'string') return v.length === 0
  if (Array.isArray(v)) return v.length === 0
  if (typeof v === 'object') {
    return Object.keys(v as Record<string, unknown>).length === 0
  }
  return false
}

function Chips({ req }: { req: StateRequest }) {
  return (
    <>
      {req.scope ? (
        <span className="state-ui-chip">
          <span className="k">scope </span>
          {req.scope}
        </span>
      ) : null}
      {req.key ? (
        <span className="state-ui-chip">
          <span className="k">key </span>
          {req.key}
        </span>
      ) : null}
    </>
  )
}

function CardShell({
  op,
  req,
  running,
  children,
}: {
  op: string
  req: StateRequest
  running?: boolean
  children?: React.ReactNode
}) {
  return (
    <div className="state-ui-msg">
      <div className="state-ui-msg-head">
        <span className={`state-ui-pill${running ? ' quiet' : ''}`}>{op}</span>
        <Chips req={req} />
        <span className="state-ui-msg-tag">state ui</span>
      </div>
      {children}
    </div>
  )
}

function SettledView({
  host,
  message,
}: {
  host: Host
  message: FunctionTriggerMessage
}) {
  const req = parseRequest(message.input)
  const op = message.functionId.slice('state::'.length)
  const result = unwrapEnvelope(message.output)

  if (message.functionId === 'state::set') {
    const set =
      result && typeof result === 'object' && !Array.isArray(result)
        ? (result as { old_value?: unknown; new_value?: unknown })
        : {}
    const created = set.old_value === null || set.old_value === undefined
    return (
      <CardShell op={op} req={req}>
        <div className="state-ui-msg-note">
          {created ? 'created — no previous value' : 'replaced the previous value'}
        </div>
        <JsonHighlight
          code={JSON.stringify(set.new_value ?? req.value ?? null, null, 2)}
        />
      </CardShell>
    )
  }

  // state::get — the raw stored value, or null when absent.
  return (
    <CardShell op={op} req={req}>
      {isEmptyValue(result) ? (
        <div className="state-ui-msg-note">· empty — no value stored</div>
      ) : (
        <JsonHighlight code={JSON.stringify(result, null, 2)} />
      )}
    </CardShell>
  )
}

function RunningView({ message }: { message: FunctionTriggerMessage }) {
  const req = parseRequest(message.input)
  const op = message.functionId.slice('state::'.length)
  return (
    <CardShell op={op} req={req} running>
      <div className="state-ui-msg-note pulse">· running…</div>
    </CardShell>
  )
}

/** Pending-approval preview: what a state::set is about to write. */
function SetPreview({
  host,
  message,
}: {
  host: Host
  message: FunctionTriggerMessage
}) {
  const req = parseRequest(message.input)
  return (
    <CardShell op="set" req={req}>
      <div className="state-ui-msg-note">will write this value:</div>
      <JsonHighlight code={JSON.stringify(req.value ?? null, null, 2)} />
    </CardShell>
  )
}

function FunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('state::')) {
    return <span style={{ color: 'var(--color-ink)' }}>{functionId}</span>
  }
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>state::</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>
        {functionId.slice('state::'.length)}
      </span>
    </>
  )
}

export function createStateTriggerRenderer(host: Host): FunctionTriggerRenderer {
  const render = (
    message: FunctionTriggerMessage,
    running: boolean,
  ): React.ReactNode | null => {
    if (!RENDERED.has(message.functionId)) return null
    if (message.pendingApproval) return null // host renders preview + approval bar
    if (running) return <RunningView message={message} />
    // Errors fall through to the console's built-in family (richer views).
    if (isErrorOutput(message.output)) return null
    return <SettledView host={host} message={message} />
  }
  return {
    id: 'state/page.js#set-get',
    isMatch: (functionId) => RENDERED.has(functionId),
    tryRender: (message) => render(message, !!message.running),
    tryRenderRunning: (message) => render(message, true),
    tryRenderPreview: (message) =>
      message.pendingApproval && message.functionId === 'state::set'
        ? <SetPreview host={host} message={message} />
        : null,
    FunctionIdLabel,
  }
}
