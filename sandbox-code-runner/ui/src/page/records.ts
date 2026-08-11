/**
 * The per-sandbox exec timeline, persisted through the state worker so a
 * console reload (or another tab) keeps the history: scope `sandbox-ui`,
 * key = the sandbox id, value = the ExecRecord array itself.
 *
 * HOUSE GOTCHA — `state::get` resolves to the stored VALUE ITSELF, not a
 * `{ value }` wrapper; the array comes back bare and absent keys resolve
 * to null/undefined. Writes are debounced (SAVE_DEBOUNCE_MS) and capped at
 * MAX_RECORDS newest records so a busy session cannot grow the entry
 * unbounded.
 */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'

const RECORDS_SCOPE = 'sandbox-ui'
const MAX_RECORDS = 50
const SAVE_DEBOUNCE_MS = 800

export interface ExecRecord {
  id: string
  /** Epoch ms of submission. */
  at: number
  /** The operator-entered command line (or code snippet label for runs). */
  cmd: string
  shell: boolean
  detached: boolean
  workdir?: string
  timeout_ms?: number
  env?: Record<string, string>
  /** Composer exec, probe button, `run code` result, or a shell background job. */
  source: 'exec' | 'probe' | 'run' | 'job'
  /** shell::exec_bg job id — present only for source 'job'. */
  job_id?: string
  /** Last observed shell job status (running/exited/killed/timed_out). */
  job_state?: string
  stdout: string
  stderr: string
  exit_code: number | null
  timed_out: boolean
  duration_ms: number | null
  /** Infrastructure failure message, when the call itself threw. */
  error?: string
}

/** String-valued env entries only; `undefined` when empty or invalid, so
 *  a junk shape never renders a phantom "0 env" chip. */
function parseEnv(value: unknown): Record<string, string> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  const out: Record<string, string> = {}
  for (const [key, v] of Object.entries(value)) {
    if (typeof v === 'string') out[key] = v
  }
  return Object.keys(out).length > 0 ? out : undefined
}

/** Tolerant read of a persisted timeline — anything that is not a record
 *  array (older shapes, hand-edited state) degrades to empty. */
export function parseRecords(value: unknown): ExecRecord[] {
  if (!Array.isArray(value)) return []
  const out: ExecRecord[] = []
  for (const item of value) {
    if (!item || typeof item !== 'object' || Array.isArray(item)) continue
    const rec = item as Record<string, unknown>
    if (typeof rec.cmd !== 'string') continue
    out.push({
      id: typeof rec.id === 'string' ? rec.id : `r-${out.length}-${Date.now()}`,
      at: typeof rec.at === 'number' ? rec.at : 0,
      cmd: rec.cmd,
      shell: rec.shell === true,
      detached: rec.detached === true,
      workdir: typeof rec.workdir === 'string' ? rec.workdir : undefined,
      timeout_ms: typeof rec.timeout_ms === 'number' ? rec.timeout_ms : undefined,
      env: parseEnv(rec.env),
      source:
        rec.source === 'probe' || rec.source === 'run' || rec.source === 'job'
          ? rec.source
          : 'exec',
      job_id: typeof rec.job_id === 'string' ? rec.job_id : undefined,
      job_state: typeof rec.job_state === 'string' ? rec.job_state : undefined,
      stdout: typeof rec.stdout === 'string' ? rec.stdout : '',
      stderr: typeof rec.stderr === 'string' ? rec.stderr : '',
      exit_code: typeof rec.exit_code === 'number' ? rec.exit_code : null,
      timed_out: rec.timed_out === true,
      duration_ms:
        typeof rec.duration_ms === 'number' ? rec.duration_ms : null,
      error: typeof rec.error === 'string' ? rec.error : undefined,
    })
  }
  return out.slice(-MAX_RECORDS)
}

export interface ExecRecords {
  records: ExecRecord[]
  /** True once the persisted timeline for the current sandbox resolved. */
  loaded: boolean
  append(record: ExecRecord): void
  /** Patch one record in place (job status refresh); persists like append. */
  update(id: string, patch: Partial<ExecRecord>): void
  clear(): void
}

export function useExecRecords(
  host: Host,
  sandboxId: string | null,
): ExecRecords {
  const [records, setRecords] = useState<ExecRecord[]>([])
  const [loaded, setLoaded] = useState(false)
  const recordsRef = useRef(records)
  // Synced post-commit, never during render (concurrent renders may be
  // thrown away); flush paths all run after commit, so the ref is current.
  useEffect(() => {
    recordsRef.current = records
  }, [records])
  const saveTimer = useRef<number | null>(null)

  const flush = useCallback(
    (key: string) => {
      host.iii
        .trigger('state::set', {
          scope: RECORDS_SCOPE,
          key,
          value: recordsRef.current.slice(-MAX_RECORDS),
        })
        .catch(() => {
          /* persistence is best-effort; the in-memory timeline stands */
        })
    },
    [host],
  )

  // Load on select; the cleanup flushes a pending debounce for the sandbox
  // being left (recordsRef still holds ITS records at cleanup time — the
  // reset in the next run's body has not happened yet), so switching away
  // right after a run cannot lose the record or write into the wrong key.
  useEffect(() => {
    setRecords([])
    setLoaded(false)
    if (!sandboxId) return
    let cancelled = false
    host.iii
      .trigger('state::get', { scope: RECORDS_SCOPE, key: sandboxId })
      .then((value) => {
        // value IS the stored array (no { value } wrapper) — see header.
        if (!cancelled) {
          // Merge, never replace: records appended while this load was
          // pending are the newest and win on id collision.
          setRecords((pending) => {
            const stored = parseRecords(value)
            const pendingIds = new Set(pending.map((r) => r.id))
            return [
              ...stored.filter((r) => !pendingIds.has(r.id)),
              ...pending,
            ].slice(-MAX_RECORDS)
          })
          setLoaded(true)
        }
      })
      .catch(() => {
        if (!cancelled) setLoaded(true)
      })
    return () => {
      cancelled = true
      if (saveTimer.current !== null) {
        window.clearTimeout(saveTimer.current)
        saveTimer.current = null
        flush(sandboxId)
      }
    }
  }, [host, sandboxId, flush])

  const scheduleSave = useCallback(() => {
    if (!sandboxId) return
    if (saveTimer.current !== null) window.clearTimeout(saveTimer.current)
    saveTimer.current = window.setTimeout(() => {
      saveTimer.current = null
      flush(sandboxId)
    }, SAVE_DEBOUNCE_MS)
  }, [flush, sandboxId])

  const append = useCallback(
    (record: ExecRecord) => {
      setRecords((prev) => [...prev, record].slice(-MAX_RECORDS))
      scheduleSave()
    },
    [scheduleSave],
  )

  const update = useCallback(
    (id: string, patch: Partial<ExecRecord>) => {
      setRecords((prev) =>
        prev.map((r) => (r.id === id ? { ...r, ...patch, id: r.id } : r)),
      )
      scheduleSave()
    },
    [scheduleSave],
  )

  const clear = useCallback(() => {
    setRecords([])
    scheduleSave()
  }, [scheduleSave])

  return { records, loaded, append, update, clear }
}
