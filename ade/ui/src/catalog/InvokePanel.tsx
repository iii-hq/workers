/**
 * Call one function with a JSON body and show what came back.
 *
 * The body opens on a template generated from the function's registered
 * `request_schema` (./schema.ts), and the editor is the console's Monaco
 * `CodeEditor` fed the schema's field names as completions — the same editor
 * the rest of the console uses, never a bundled second one.
 *
 * Two things the old console did not do:
 *
 * - required fields are checked against the schema BEFORE the call, so an
 *   obvious mistake reads as "scope is required" instead of a worker-side
 *   serialization error
 * - every call this panel makes is kept for the session and can be replayed,
 *   so tuning a payload is a loop rather than a retype
 */

import {
  Button,
  CodeEditor,
  Eyebrow,
  type Host,
  JsonHighlight,
  KeyCombo,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { Play } from 'lucide-react'
import {
  type MutableRefObject,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { formatDuration } from './ActivityFeed'
import { type InvokeOutcome, invoke } from './engine'
import { schemaFieldNames } from './SchemaTable'
import { pretty, templateFromSchema } from './schema'
import { CopyButton, CopyIconButton } from './widgets'

interface Attempt {
  id: number
  atMs: number
  body: string
  outcome: InvokeOutcome
}

let attemptSeq = 0

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Required top-level fields the body is missing, by the schema's own list. */
function missingRequired(schema: unknown, payload: unknown): string[] {
  if (!isRecord(schema) || !isRecord(payload)) return []
  const required = Array.isArray(schema.required)
    ? schema.required.filter((k): k is string => typeof k === 'string')
    : []
  return required.filter(
    (key) => payload[key] === undefined || payload[key] === '',
  )
}

/**
 * The same call as a CLI line. `iii trigger` takes `key=value` pairs, so a
 * scalar body copies verbatim and anything nested copies as JSON — which is
 * exactly the difference between a command that runs and one that does not.
 */
export function asCliCommand(functionId: string, payload: unknown): string {
  if (!isRecord(payload) || Object.keys(payload).length === 0) {
    return `iii trigger ${functionId}`
  }
  const args = Object.entries(payload).map(([key, value]) => {
    const literal =
      typeof value === 'string' ? value : (JSON.stringify(value) ?? '')
    if (!/[\s"']/.test(literal)) return `${key}=${literal}`
    // A single quote cannot appear inside single quotes: close the segment,
    // emit an escaped quote, reopen.
    return `${key}='${literal.replaceAll("'", `'\\''`)}'`
  })
  return `iii trigger ${functionId} ${args.join(' ')}`
}

export function InvokePanel({
  host,
  functionId,
  requestSchema,
  label = 'trigger',
  runningLabel = 'triggering…',
  hint,
  prefill,
  runRef,
  layout = 'stacked',
}: {
  host: Host
  functionId: string
  requestSchema: unknown
  /** Verb on the button — the triggers page fires a target function. */
  label?: string
  runningLabel?: string
  hint?: string
  /** `split` puts the request beside the response (the functions page's Run
      tab); `stacked` is the framed card the triggers page embeds. */
  layout?: 'stacked' | 'split'
  /** A recorded input pushed in from the activity feed; changes replace the body. */
  prefill?: { value: unknown; nonce: number }
  /** Kept current with this panel's run() while mounted, so the page's
      Mod+Enter command can reach it without lifting this form up. */
  runRef?: MutableRefObject<(() => void) | null>
}) {
  const [body, setBody] = useState('{}')
  const [running, setRunning] = useState(false)
  const [attempts, setAttempts] = useState<Attempt[]>([])
  const [invalid, setInvalid] = useState<string | null>(null)

  // A new selection resets the editor to that function's own template and
  // drops the previous function's attempts with it. Keyed on the schema's
  // CONTENT, not its identity — live catalog refreshes rebuild the object
  // every tick, and resetting on identity would wipe a body mid-edit.
  const schemaKey = JSON.stringify(requestSchema) ?? 'none'
  useEffect(() => {
    setBody(templateFromSchema(requestSchema))
    setAttempts([])
    setInvalid(null)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [functionId, schemaKey])

  useEffect(() => {
    if (!prefill) return
    setBody(pretty(prefill.value) || '{}')
    setInvalid(null)
  }, [prefill])

  const completions = useMemo(
    () => schemaFieldNames(requestSchema),
    [requestSchema],
  )

  const latest = attempts[0] ?? null
  const outcome = latest?.outcome ?? null

  const reset = () => {
    setBody(templateFromSchema(requestSchema))
    setInvalid(null)
  }

  const inFlightRef = useRef(false)
  const run = async () => {
    let payload: unknown
    try {
      payload = JSON.parse(body)
    } catch (err) {
      setInvalid(err instanceof Error ? err.message : 'invalid JSON')
      return
    }
    if (!isRecord(payload)) {
      setInvalid('the request body must be a JSON object')
      return
    }
    const missing = missingRequired(requestSchema, payload)
    if (missing.length > 0) {
      setInvalid(
        `${missing.join(', ')} ${missing.length > 1 ? 'are' : 'is'} required by the schema`,
      )
      return
    }
    if (inFlightRef.current) return
    inFlightRef.current = true
    setInvalid(null)
    setRunning(true)
    let result: Awaited<ReturnType<typeof invoke>>
    try {
      result = await invoke(host, functionId, payload)
    } finally {
      inFlightRef.current = false
      setRunning(false)
    }
    attemptSeq += 1
    setAttempts((prev) => [
      { id: attemptSeq, atMs: Date.now(), body, outcome: result },
      ...prev.slice(0, 9),
    ])
  }

  if (runRef) runRef.current = run
  useEffect(() => {
    return () => {
      if (runRef) runRef.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const split = layout === 'split'

  const request = (
    <>
      <div className="console-catalog-invoke-head">
        <div>
          <h3>{split ? 'Request' : 'Trigger function'}</h3>
          <p className="invoke-description">
            {split
              ? 'A JSON body; required fields are checked before the call.'
              : 'Provide the input payload and trigger this function.'}
          </p>
        </div>
        <Button
          variant={split ? 'ghost' : 'pill'}
          size="sm"
          type="button"
          onClick={reset}
          disabled={running}
        >
          {split ? 'Reset' : 'reset'}
        </Button>
      </div>
      {hint ? <div className="console-catalog-note">{hint}</div> : null}
      {split ? null : (
        <Eyebrow className="console-catalog-field-label">
          Input payload (JSON)
        </Eyebrow>
      )}
      <CodeEditor
        value={body}
        onChange={setBody}
        language="json"
        className="console-catalog-editor"
        completions={completions}
        aria-label={`request body for ${functionId}`}
      />
      <div className="console-catalog-invoke-foot">
        <Button
          type="button"
          variant={split ? 'primary' : undefined}
          size={split ? 'md' : 'sm'}
          onClick={run}
          disabled={running}
        >
          {split ? <Play aria-hidden /> : null}
          {running ? runningLabel : label}
          {split ? <KeyCombo binding="Mod+Enter" /> : null}
        </Button>
        <CopyButton
          value={asCliCommand(functionId, safeJson(body))}
          label={split ? 'Copy as command' : 'copy iii command'}
          title="copies this exact call as an `iii trigger …` line to run from the terminal"
        />
        {invalid ? (
          <span className="console-catalog-invalid">{invalid}</span>
        ) : null}
      </div>
    </>
  )

  const response = outcome ? (
    <div className="console-catalog-result-shell" data-ok={outcome.ok}>
      <div className="console-catalog-result-head">
        <span
          className={
            outcome.ok ? 'console-catalog-ok' : 'console-catalog-invalid'
          }
        >
          <StatusDot tone={outcome.ok ? 'ok' : 'alert'} />
          {split
            ? outcome.ok
              ? 'Success'
              : 'Error'
            : outcome.ok
              ? 'success'
              : 'error'}
        </span>
        <span className="result-meta">
          {latest ? clockTime(latest.atMs) : null}
          <span>{formatDuration(outcome.durationMs)}</span>
          {split && outcome.ok ? (
            <CopyIconButton
              value={pretty(outcome.data) || 'null'}
              label="Copy response"
            />
          ) : null}
        </span>
      </div>
      {outcome.error ? (
        <StatusPanel variant="alert" headline={outcome.error} />
      ) : (
        <JsonHighlight
          code={pretty(outcome.data) || 'null'}
          className="console-catalog-result"
          wrap
        />
      )}
    </div>
  ) : null

  // The stacked card lists only the earlier attempts (the latest is the
  // result above it); the split view lists all of them under the editor,
  // the latest marked, so the history reads as one column.
  const earlier = split ? attempts : attempts.slice(1)
  const history =
    earlier.length > 0 ? (
      <div className="console-catalog-attempts">
        <Eyebrow className="console-catalog-field-label">This session</Eyebrow>
        {earlier.map((attempt) => (
          <button
            key={attempt.id}
            type="button"
            className="console-catalog-attempt"
            data-latest={split && attempt.id === latest?.id}
            onClick={() => setBody(attempt.body)}
            title="put this body back in the editor"
          >
            <StatusDot tone={attempt.outcome.ok ? 'ok' : 'alert'} />
            {split ? (
              <span className="time">{clockTime(attempt.atMs)}</span>
            ) : null}
            <span className="body">{oneLine(attempt.body)}</span>
            <span className="duration">
              {formatDuration(attempt.outcome.durationMs)}
            </span>
          </button>
        ))}
      </div>
    ) : null

  if (split) {
    return (
      <div className="console-catalog-invoke" data-layout="split">
        <section className="invoke-column" aria-label="Request">
          {request}
          {history}
        </section>
        <section className="invoke-column" aria-label="Response">
          <div className="console-catalog-invoke-head">
            <div>
              <h3>Response</h3>
            </div>
          </div>
          {response ?? (
            <div className="console-catalog-response-empty">
              Run the function to see what it returns. Each run stays in this
              session's list, so you can load an earlier body back.
            </div>
          )}
        </section>
      </div>
    )
  }

  return (
    <div className="console-catalog-invoke">
      {request}
      {response}
      {history}
    </div>
  )
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour12: false })
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return {}
  }
}

function oneLine(body: string): string {
  const flat = body.replace(/\s+/g, ' ').trim()
  return flat.length > 64 ? `${flat.slice(0, 61)}…` : flat
}
