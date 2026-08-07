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
  type Host,
  JsonHighlight,
} from '@iii-dev/console-ui'
import { useEffect, useMemo, useState } from 'react'
import { type InvokeOutcome, invoke } from './engine'
import { schemaFieldNames } from './SchemaTable'
import { pretty, templateFromSchema } from './schema'
import { Chip, CopyButton } from './widgets'

interface Attempt {
  id: number
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
function asCliCommand(functionId: string, payload: unknown): string {
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
  label = 'invoke',
  runningLabel = 'invoking…',
  hint,
  prefill,
}: {
  host: Host
  functionId: string
  requestSchema: unknown
  /** Verb on the button — the triggers page fires a target function. */
  label?: string
  runningLabel?: string
  hint?: string
  /** A recorded input pushed in from the activity feed; changes replace the body. */
  prefill?: { value: unknown; nonce: number }
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

  const outcome = attempts[0]?.outcome ?? null

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
    setInvalid(null)
    setRunning(true)
    const result = await invoke(host, functionId, payload)
    setRunning(false)
    attemptSeq += 1
    setAttempts((prev) => [
      { id: attemptSeq, body, outcome: result },
      ...prev.slice(0, 9),
    ])
  }

  return (
    <div className="console-catalog-invoke">
      {hint ? <div className="console-catalog-note">{hint}</div> : null}
      <CodeEditor
        value={body}
        onChange={setBody}
        language="json"
        className="console-catalog-editor"
        completions={completions}
        aria-label={`request body for ${functionId}`}
      />
      <div className="console-catalog-invoke-foot">
        <Button size="sm" onClick={run} disabled={running}>
          {running ? runningLabel : label}
        </Button>
        <CopyButton
          value={asCliCommand(functionId, safeJson(body))}
          label="copy iii command"
          title="copies this exact call as an `iii trigger …` line to run from the terminal"
        />
        {invalid ? (
          <span className="console-catalog-invalid">{invalid}</span>
        ) : null}
        {outcome ? (
          <span
            className={
              outcome.ok ? 'console-catalog-ok' : 'console-catalog-invalid'
            }
          >
            {outcome.ok ? 'ok' : 'error'} · {Math.round(outcome.durationMs)}ms
          </span>
        ) : null}
      </div>

      {outcome?.error ? (
        <div className="console-catalog-error">{outcome.error}</div>
      ) : null}
      {outcome?.ok ? (
        <JsonHighlight
          code={pretty(outcome.data) || 'null'}
          className="console-catalog-result"
          wrap
        />
      ) : null}

      {attempts.length > 1 ? (
        <div className="console-catalog-attempts">
          <span className="console-catalog-field-label">this session</span>
          {attempts.slice(1).map((attempt) => (
            <button
              key={attempt.id}
              type="button"
              className="console-catalog-attempt"
              onClick={() => setBody(attempt.body)}
              title="put this body back in the editor"
            >
              <span className="dot" data-ok={attempt.outcome.ok} />
              <span className="body">{oneLine(attempt.body)}</span>
              <span className="duration">
                {Math.round(attempt.outcome.durationMs)}ms
              </span>
            </button>
          ))}
        </div>
      ) : null}

      <Chip k="calls" v={functionId} />
    </div>
  )
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
