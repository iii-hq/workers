/**
 * Send a real request to an http binding's endpoint.
 *
 * This is the one fire path that does not go over the bus: an http trigger
 * fires when the http worker receives a request, so the honest test is an
 * actual request to the port that worker listens on. The base URL comes from
 * the worker's own configuration entry (`configuration::get id=iii-http`),
 * never a guess, and the panel says plainly when it cannot be read.
 */

import {
  Button,
  CodeEditor,
  type Host,
  Input,
  JsonHighlight,
  Select,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import { errorMessage, useResource } from './engine'
import { pretty } from './schema'
import type { HttpBinding } from './trigger-kinds'
import { Chip, ErrorNote, Note } from './widgets'

const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD'] as const
const BODY_METHODS = new Set(['POST', 'PUT', 'PATCH'])

interface HttpEndpoint {
  baseUrl: string
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * Where the http worker listens. `host` is what the worker binds, which is
 * `127.0.0.1` on a local rig; a tab opened over the LAN cannot reach that, so
 * a loopback bind is rewritten to the hostname the console itself was
 * loaded from.
 */
async function readEndpoint(host: Host): Promise<HttpEndpoint> {
  // `http` is the current worker; `iii-http` is its deprecated predecessor.
  // Whichever entry exists with a port wins, current name first.
  let value: Record<string, unknown> | null = null
  for (const id of ['http', 'iii-http']) {
    try {
      const entry = await host.iii.trigger('configuration::get', { id })
      const candidate =
        isRecord(entry) && isRecord(entry.value) ? entry.value : null
      if (candidate && typeof candidate.port === 'number') {
        value = candidate
        break
      }
    } catch {
      // Entry absent under this id; try the next.
    }
  }
  if (!value) throw new Error('no http worker configuration with a port found')
  const port = value.port
  if (typeof port !== 'number') throw new Error('http config carries no port')
  const bound = typeof value.host === 'string' ? value.host : '127.0.0.1'
  const reachable =
    bound === '0.0.0.0' || bound === '127.0.0.1' || bound === 'localhost'
      ? window.location.hostname
      : bound
  return { baseUrl: `${window.location.protocol}//${reachable}:${port}` }
}

interface QueryRow {
  id: number
  key: string
  value: string
}

let queryRowSeq = 0

interface Outcome {
  ok: boolean
  status: number | null
  durationMs: number
  body: string
  error?: string
}

export function HttpTester({
  host,
  binding,
}: {
  host: Host
  binding: HttpBinding
}) {
  const load = useCallback(() => readEndpoint(host), [host])
  const endpoint = useResource(load)

  const [method, setMethod] = useState(binding.method)
  const [params, setParams] = useState<Record<string, string>>({})
  // Rows carry their own id: the key is empty until typed, so nothing else
  // about a row is stable enough to key React on.
  const [query, setQuery] = useState<QueryRow[]>([])
  const [body, setBody] = useState('{}')
  const [sending, setSending] = useState(false)
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  const [invalid, setInvalid] = useState<string | null>(null)

  // A new selection resets the whole form; a stale path parameter filled for
  // a different endpoint is worse than an empty one. Keyed on the endpoint's
  // VALUES, not the binding's identity — live catalog refreshes rebuild the
  // object every tick, and resetting on identity would wipe the form
  // mid-typing.
  const paramKey = binding.params.join(',')
  useEffect(() => {
    setMethod(binding.method)
    setParams(Object.fromEntries(binding.params.map((p) => [p, ''])))
    setQuery([])
    setBody('{}')
    setOutcome(null)
    setInvalid(null)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [binding.method, binding.path, paramKey])

  const filledPath = binding.params.reduce(
    (path, name) =>
      path.replace(`:${name}`, encodeURIComponent(params[name] ?? `:${name}`)),
    binding.path,
  )
  const queryString = query
    .filter((q) => q.key)
    .map((q) => `${encodeURIComponent(q.key)}=${encodeURIComponent(q.value)}`)
    .join('&')
  const url = endpoint.data
    ? `${endpoint.data.baseUrl}${filledPath}${queryString ? `?${queryString}` : ''}`
    : null

  const send = async () => {
    if (!url) return
    const missing = binding.params.filter((name) => !params[name])
    if (missing.length > 0) {
      setInvalid(
        `fill the path parameter${missing.length > 1 ? 's' : ''}: ${missing.join(', ')}`,
      )
      setOutcome(null)
      return
    }
    const init: RequestInit = { method }
    if (BODY_METHODS.has(method)) {
      try {
        JSON.parse(body)
      } catch (err) {
        setInvalid(err instanceof Error ? err.message : 'invalid JSON body')
        setOutcome(null)
        return
      }
      init.headers = { 'Content-Type': 'application/json' }
      init.body = body
    }
    setInvalid(null)
    setSending(true)
    const started = performance.now()
    try {
      const response = await fetch(url, init)
      const text = await response.text()
      const contentType = response.headers.get('content-type') ?? ''
      setOutcome({
        ok: response.ok,
        status: response.status,
        durationMs: performance.now() - started,
        body: contentType.includes('json') ? pretty(safeJson(text)) : text,
      })
    } catch (err) {
      setOutcome({
        ok: false,
        status: null,
        durationMs: performance.now() - started,
        body: '',
        error: errorMessage(err),
      })
    } finally {
      setSending(false)
    }
  }

  if (endpoint.error) {
    return <ErrorNote call="configuration::get http" message={endpoint.error} />
  }
  if (!endpoint.data) return <Note>reading the http worker's address…</Note>

  return (
    <div className="console-catalog-invoke">
      <div className="console-catalog-endpoint">
        <span className="method">{method}</span>
        <code>{url}</code>
        <Button
          variant="pill"
          size="sm"
          onClick={() => url && navigator.clipboard.writeText(url)}
        >
          copy
        </Button>
      </div>

      <div className="console-catalog-field-row">
        <Select
          value={method}
          options={METHODS.map((m) => ({ value: m, label: m }))}
          onChange={setMethod}
          aria-label="http method"
          className="console-catalog-method"
        />
        <code className="console-catalog-path">{binding.path}</code>
      </div>

      {binding.params.length > 0 ? (
        <div className="console-catalog-fields">
          <span className="console-catalog-field-label">path parameters</span>
          {binding.params.map((name) => (
            <div key={name} className="console-catalog-field-row">
              <label htmlFor={`param-${name}`} className="console-catalog-key">
                :{name}
              </label>
              <Input
                id={`param-${name}`}
                value={params[name] ?? ''}
                onChange={(next) =>
                  setParams((prev) => ({ ...prev, [name]: next }))
                }
                preserveCase
                placeholder={name}
              />
            </div>
          ))}
        </div>
      ) : null}

      <div className="console-catalog-fields">
        <span className="console-catalog-field-label">
          query parameters
          <Button
            variant="pill"
            size="sm"
            onClick={() => {
              queryRowSeq += 1
              setQuery((prev) => [
                ...prev,
                { id: queryRowSeq, key: '', value: '' },
              ])
            }}
          >
            add
          </Button>
        </span>
        {query.length === 0 ? (
          <span className="console-catalog-hint">none</span>
        ) : (
          query.map((entry) => (
            <div key={entry.id} className="console-catalog-field-row">
              <Input
                value={entry.key}
                onChange={(next) =>
                  setQuery((prev) =>
                    prev.map((q) =>
                      q.id === entry.id ? { ...q, key: next } : q,
                    ),
                  )
                }
                preserveCase
                placeholder="key"
                aria-label="query parameter name"
              />
              <Input
                value={entry.value}
                onChange={(next) =>
                  setQuery((prev) =>
                    prev.map((q) =>
                      q.id === entry.id ? { ...q, value: next } : q,
                    ),
                  )
                }
                preserveCase
                placeholder="value"
                aria-label="query parameter value"
              />
              <Button
                variant="pill"
                size="sm"
                onClick={() =>
                  setQuery((prev) => prev.filter((q) => q.id !== entry.id))
                }
              >
                remove
              </Button>
            </div>
          ))
        )}
      </div>

      {BODY_METHODS.has(method) ? (
        <CodeEditor
          value={body}
          onChange={setBody}
          language="json"
          className="console-catalog-editor"
          aria-label="request body"
        />
      ) : null}

      <div className="console-catalog-invoke-foot">
        <Button size="sm" onClick={send} disabled={sending}>
          {sending ? 'sending…' : 'send request'}
        </Button>
        {invalid ? (
          <span className="console-catalog-invalid">{invalid}</span>
        ) : null}
        {outcome ? (
          <span
            className={
              outcome.ok ? 'console-catalog-ok' : 'console-catalog-invalid'
            }
          >
            {outcome.status ?? 'failed'} · {Math.round(outcome.durationMs)}ms
          </span>
        ) : null}
      </div>

      {outcome?.error ? (
        <div className="console-catalog-error">{outcome.error}</div>
      ) : null}
      {outcome && !outcome.error ? (
        <JsonHighlight
          code={outcome.body || '(empty response)'}
          className="console-catalog-result"
          wrap
        />
      ) : null}
      <Chip k="fires" v="the real endpoint, through the http worker" />
    </div>
  )
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return text
  }
}
