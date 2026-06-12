/* Pure formatting helpers shared by the shell renderers. No React, no
   DOM access — deterministic transforms over the parsed shell payloads.
   Generic byte/mode/time/quoting helpers come from sandbox/format. */

import { formatMtime, quoteShellArg } from '@/components/chat/sandbox/format'
import type { JobStatus } from './parsers'

/** Render an ExecRequest command line for the terminal prompt.
    `args` absent/null ⇒ `command` is a shell line the server shell-words
    tokenizes — display verbatim. `args` present (even `[]`) ⇒ verbatim
    argv — quote every slot including the program. Preserves the wire
    semantic distinction. */
export function formatShellCommand(req: {
  command: string
  args?: string[] | null
}): string {
  if (req.args == null) return req.command
  return [req.command, ...req.args].map(quoteShellArg).join(' ')
}

/** Quote-join a server-resolved argv (`JobRecord.argv`,
    `ExecBgResponse.argv`) into a paste-able shell line. */
export function formatArgv(argv: string[]): string {
  return argv.map(quoteShellArg).join(' ')
}

/** Epoch-millis (`started_at_ms`/`finished_at_ms`) → short relative
    time, via sandbox `formatMtime` (which takes unix seconds). */
export function formatEpochMs(ms: number): string {
  return formatMtime(Math.floor(ms / 1000))
}

/** Job wall-clock duration. Null while running (`finished_at_ms` null);
    clamps negative clock skew to 0. */
export function jobDurationMs(rec: {
  started_at_ms: number
  finished_at_ms: number | null
}): number | null {
  return rec.finished_at_ms == null
    ? null
    : Math.max(0, rec.finished_at_ms - rec.started_at_ms)
}

/** Status pill mapping — single source of truth for status/kill/list. */
export function jobStatusPill(status: JobStatus): {
  label: string
  tone: 'accent' | 'warn' | 'alert' | 'default'
} {
  switch (status) {
    case 'running':
      return { label: 'running', tone: 'accent' }
    case 'finished':
      return { label: 'finished', tone: 'default' }
    case 'killed':
      return { label: 'killed', tone: 'warn' }
    case 'failed':
      return { label: 'failed', tone: 'alert' }
  }
}
