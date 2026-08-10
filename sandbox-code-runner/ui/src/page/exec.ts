/**
 * Exec-shaping helpers for the console tab: argv building for the shell /
 * detached toggles, the copy-as-CLI formatter, response parsing, and the
 * base64 download decoder. All pure — covered directly by exec.test.ts.
 */

import { quoteShellArg } from '../lib/format'
import { asRecord, unwrapEnvelope } from '../lib/payload'

export { quoteShellArg }

export interface ExecShape {
  /** Wrap the line in `sh -c` so pipes/globs/redirects work. */
  shell: boolean
  /** Detach: survive the exec call, free the slot immediately. */
  detached: boolean
}

/**
 * Why the detached shape is `setsid sh -c '{ <cmd> ; } >/dev/null 2>&1 &'`:
 *
 * - the brace group makes a multi-statement line (`a; b`) one unit, so the
 *   redirect and the `&` apply to the whole line, not just its last command;
 * - the trailing `&` backgrounds the group so `sh` exits immediately and
 *   the exec call returns (exit 0, no output) instead of blocking a slot;
 * - the `>/dev/null 2>&1` redirect is load-bearing, not cosmetic — without
 *   it the surviving child inherits the exec's stdout/stderr pipes, and the
 *   daemon's read loop only sees EOF when the child dies, so the "detached"
 *   run would still pin the slot for the child's whole life;
 * - `setsid` re-parents the child into its own session, so cleanup signals
 *   aimed at the exec's process group cannot reach it after the call ends.
 */
export const DETACH_TITLE =
  "run detached: wraps the command as setsid sh -c '{ <cmd> ; } >/dev/null 2>&1 &' — " +
  'the brace group covers every statement of the line, the & backgrounds it ' +
  'so sh (and the exec) return immediately, the >/dev/null 2>&1 redirect ' +
  'closes the exec pipes so the slot frees at once (without it the child ' +
  'would hold stdout open and pin the slot), and setsid gives it its own ' +
  'session so end-of-exec cleanup cannot kill it. ' +
  'Expect exit 0 and no output; check on it with ps.'

/**
 * The command line → the exec request's command fields.
 *
 * - plain: `{ cmd }` untouched — the daemon shlex-splits a whitespace `cmd`
 *   with no `args` server-side, so quoting stays the daemon's business;
 * - shell: `{ argv: ['sh', '-c', line] }`;
 * - detached (implies a shell wrap by construction): see DETACH_TITLE.
 */
export function buildExecArgv(
  line: string,
  shape: ExecShape,
): { argv: string[] } | { cmd: string } {
  if (shape.detached)
    return { argv: ['setsid', 'sh', '-c', `{ ${line} ; } >/dev/null 2>&1 &`] }
  if (shape.shell) return { argv: ['sh', '-c', line] }
  return { cmd: line }
}

export interface ExecFormValues extends ExecShape {
  line: string
  workdir: string
  timeoutMs: string
  env: { key: string; value: string }[]
}

/** The full `sandbox::exec` payload for a composer submission. Empty
 *  optional fields are omitted, never sent as `''`/`0`. */
export function buildExecPayload(
  sandboxId: string,
  form: ExecFormValues,
): Record<string, unknown> {
  const payload: Record<string, unknown> = {
    sandbox_id: sandboxId,
    ...buildExecArgv(form.line, form),
  }
  if (form.workdir.trim()) payload.workdir = form.workdir.trim()
  const timeout = Number(form.timeoutMs)
  if (form.timeoutMs.trim() && Number.isFinite(timeout) && timeout > 0)
    payload.timeout_ms = timeout
  const env: Record<string, string> = {}
  for (const row of form.env) if (row.key.trim()) env[row.key.trim()] = row.value
  if (Object.keys(env).length > 0) payload.env = env
  return payload
}

/** `iii trigger sandbox::exec '{"sandbox_id":"…"}'` — paste-able as-is. */
export function formatTriggerCommand(
  functionId: string,
  payload: Record<string, unknown>,
): string {
  return `iii trigger ${functionId} ${quoteShellArg(JSON.stringify(payload))}`
}

export interface ExecResult {
  stdout: string
  stderr: string
  exit_code: number | null
  timed_out: boolean
  duration_ms: number | null
  success: boolean
}

/** Tolerant read of a `sandbox::exec` / `::run` response. Missing fields
 *  degrade (`exit_code: null`, empty streams) instead of throwing. */
export function parseExecResult(value: unknown): ExecResult {
  const rec = asRecord(unwrapEnvelope(value)) ?? {}
  const exit = rec.exit_code
  return {
    stdout: typeof rec.stdout === 'string' ? rec.stdout : '',
    stderr: typeof rec.stderr === 'string' ? rec.stderr : '',
    exit_code: typeof exit === 'number' ? exit : null,
    timed_out: rec.timed_out === true,
    duration_ms: typeof rec.duration_ms === 'number' ? rec.duration_ms : null,
    success: rec.success === true || exit === 0,
  }
}

/**
 * `base64 <path>` output → bytes. The tool wraps lines and may leave a
 * trailing newline, so all whitespace is stripped before decoding. Throws
 * on non-base64 input — the caller shows the error card.
 */
export function decodeBase64(text: string): Uint8Array<ArrayBuffer> {
  const compact = text.replace(/\s+/g, '')
  const binary = atob(compact)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i)
  return bytes
}

/** The exit pill for a record. Non-zero exit is the user's program failing,
 *  never a system error — warn, not alert (house rule, see lib/shared). */
export function exitPill(record: {
  timed_out: boolean
  exit_code: number | null
  error?: string
}): { label: string; tone: 'ok' | 'warn' | 'alert' } {
  if (record.error) return { label: 'error', tone: 'alert' }
  if (record.timed_out) return { label: 'timed out', tone: 'warn' }
  if (record.exit_code === 0) return { label: 'exit 0', tone: 'ok' }
  if (record.exit_code === null) return { label: 'no exit', tone: 'warn' }
  return { label: `exit ${record.exit_code}`, tone: 'warn' }
}
