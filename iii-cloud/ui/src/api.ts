// iii cloud routes the page reads and writes as the person: `iii-cloud::api`
// (GET) and `iii-cloud::api::write` (the page's own, denied to agents). Paths
// are relative to an environment (`envId`) or, with `envId` null, to the team.

import type { Host } from '@iii-dev/console-ui'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import { plainMessage, workerError } from './model'
import type { Status, WorkerError } from './types'

export type CloudWriteMethod = 'POST' | 'PUT' | 'PATCH' | 'DELETE'

export type Cloud<T> = {
  /** The answer's body for the current path; null before the first answer and right after the path changes. */
  data: T | null
  /** `message` is already a plain sentence; `code` is the cloud's (or the worker's), `status` the HTTP status. */
  error: WorkerError | null
  loading: boolean
  refetch: () => void
}

/**
 * `/teams/{team}/environments/{envId}{path}` (`envId` null: `/teams/{team}{path}`); null before there is a team.
 * The team is the project's, else the API key's. With `envId` null, a path that starts with `/teams/` is
 * already whole (Start reads other teams' projects).
 */
export function cloudPath(s: Status, envId: string | null, path: string): string | null {
  if (envId === null && path.startsWith('/teams/')) return path
  const team = s.project?.team?.id ?? s.connection.api_key?.team_id
  if (!team) return null
  return `/teams/${team}${envId === null ? '' : `/environments/${envId}`}${path}`
}

/** The `iii-cloud::api` payload: its `path` refuses `?`, so a query string becomes the `query` object. */
function payload(method: string, full: string, body?: unknown): Record<string, unknown> {
  const q = full.indexOf('?')
  const path = q < 0 ? full : full.slice(0, q)
  const query = q < 0 ? null : Object.fromEntries(new URLSearchParams(full.slice(q + 1)))
  return { method, path, ...(query ? { query } : {}), ...(body === undefined ? {} : { body }) }
}

type Failure = Error & { code: string; status?: number }
const fail = (e: { code: string; message: string; status?: number }): Failure =>
  Object.assign(new Error(plainMessage(e)), { code: e.code, status: e.status })

async function send<T>(host: Host, fn: string, input: Record<string, unknown>): Promise<T> {
  let out: { status?: unknown; body?: unknown } | null
  try {
    out = await host.iii.call(fn, input, { timeoutMs: 60_000 })
  } catch (err) {
    throw fail(workerError(err, errorCode(err), errorMessage(err)))
  }
  const status = typeof out?.status === 'number' ? out.status : undefined
  if (status !== undefined && status >= 200 && status < 300) return out?.body as T
  // The cloud's error body is `{code, message, fix?}`; a 403 names the role, so its message is kept.
  const b = (out?.body ?? {}) as { code?: unknown; message?: unknown }
  throw fail({
    code: typeof b.code === 'string' ? b.code : status === 404 ? 'NOT_FOUND' : 'CLOUD_ERROR',
    message: typeof b.message === 'string' ? b.message : '',
    status,
  })
}

const asError = (err: unknown): WorkerError => {
  const e = err as Partial<Failure>
  return {
    code: e.code ?? 'ERROR',
    message: e.message ?? plainMessage({ code: 'ERROR', message: '' }),
    status: e.status,
  }
}

/**
 * GET one cloud route of `envId` (null: of the team) and keep it fresh: it reads again when the path
 * changes, when a deploy of that environment changes in the status (its events), when the status is
 * refreshed (the Refresh button and the 30 s poll), on `refetch`, and every `pollMs` while the tab is
 * visible. An answer to an older request is dropped. `path` null reads nothing.
 */
export function useCloud<T>(
  host: Host,
  status: Status,
  envId: string | null,
  path: string | null,
  opts: { pollMs?: number } = {},
): Cloud<T> {
  const full = path === null ? null : cloudPath(status, envId, path)
  const [state, setState] = useState<{ key: string | null; data: T | null; error: WorkerError | null; busy: boolean }>({
    key: null,
    data: null,
    error: null,
    busy: false,
  })
  const [tick, setTick] = useState(0)
  const seq = useRef(0)
  const refetch = useCallback(() => setTick((t) => t + 1), [])
  // The deploys of the env read (a whole `/teams/…` path names its own, else none): a deploy elsewhere reads nothing.
  const own =
    envId === null && path?.startsWith('/teams/') ? (path.match(/\/environments\/([^/?]+)/)?.[1] ?? '') : envId
  const deploys = status.deploys
    .filter((d) => own === null || d.environment_id === own)
    .map((d) => `${d.id}:${d.status}:${d.outcome}`)
    .join()
  const refreshed = status.refreshed_at

  useEffect(() => {
    const mine = ++seq.current
    if (!full) {
      setState({ key: null, data: null, error: null, busy: false })
      return
    }
    // Same route: keep showing the last answer while reading it again.
    setState((s) => (s.key === full ? { ...s, busy: true } : { key: full, data: null, error: null, busy: true }))
    // Only the newest request may answer: `seq` moved on means a newer one is out.
    send<T>(host, 'iii-cloud::api', payload('GET', full)).then(
      (data) => {
        if (mine === seq.current) setState({ key: full, data, error: null, busy: false })
      },
      (err) => {
        if (mine === seq.current) setState((s) => ({ ...s, key: full, error: asError(err), busy: false }))
      },
    )
  }, [host, full, deploys, refreshed, tick])

  useEffect(() => {
    if (!opts.pollMs) return
    const t = setInterval(() => {
      if (document.visibilityState === 'visible') refetch()
    }, opts.pollMs)
    return () => clearInterval(t)
  }, [opts.pollMs, refetch])

  const fresh = state.key === full
  return {
    data: fresh ? state.data : null,
    error: fresh ? state.error : null,
    loading: full !== null && (!fresh || state.busy),
    refetch,
  }
}

/**
 * GET one route once, now (a re-read right before a write): `useCloud` keeps a route fresh, this asks once.
 * Rejects like `cloudWrite`, with the same plain messages as `useCloud`.
 */
export function cloudRead<T>(host: Host, status: Status, envId: string | null, path: string): Promise<T> {
  const full = cloudPath(status, envId, path)
  if (!full) return Promise.reject(fail({ code: 'NO_TEAM', message: 'Choose a project first.' }))
  return send<T>(host, 'iii-cloud::api', payload('GET', full))
}

/**
 * One change in iii cloud through `iii-cloud::api::write`, with a fresh `idempotency_key` per call:
 * call it once per click. Resolves the cloud's body; rejects with `code`, `status` and a plain
 * `message` (read them with `errorCode`/`errorMessage`). Secret values never go here: `iii-cloud::secret`.
 */
export function cloudWrite<T = unknown>(
  host: Host,
  status: Status,
  envId: string | null,
  method: CloudWriteMethod,
  path: string,
  body?: unknown,
): Promise<T> {
  const full = cloudPath(status, envId, path)
  if (!full) return Promise.reject(fail({ code: 'NO_TEAM', message: 'Choose a project first.' }))
  return send<T>(host, 'iii-cloud::api::write', {
    ...payload(method, full, body),
    idempotency_key: crypto.randomUUID(),
  })
}
