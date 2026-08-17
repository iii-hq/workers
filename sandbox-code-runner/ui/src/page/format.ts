/**
 * Pure formatting helpers for the sandbox fleet page. No React, no DOM —
 * deterministic transforms over wire values, kept separate so the vitest
 * suite can cover them without a renderer.
 */

import { formatBytes } from '../lib/format'

export { formatBytes }

/** The three states a fleet row can be in, derived from `sandbox::list`.
 *  Busy wins over running; stopped wins over both. */
export type SandboxState = 'running' | 'busy' | 'stopped'

export function sandboxState(sandbox: {
  stopped: boolean
  exec_in_flight: number
}): SandboxState {
  if (sandbox.stopped) return 'stopped'
  if (sandbox.exec_in_flight > 0) return 'busy'
  return 'running'
}

/** Humanize an age in seconds, mirroring `sandbox::list`'s `age_secs`. */
export function formatAgeSecs(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return '—'
  if (secs < 60) return `${Math.floor(secs)}s`
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h`
  return `${Math.floor(hours / 24)}d`
}

/** Age to display NOW: the wire's `age_secs` plus the seconds since the
 *  snapshot carrying it landed — fleet events are change-only, so ages
 *  must grow client-side between them. */
export function displayAgeSecs(
  ageSecs: number,
  snapshotAt: number | null,
  nowMs: number,
): number {
  if (snapshotAt === null || nowMs <= snapshotAt) return ageSecs
  return ageSecs + (nowMs - snapshotAt) / 1000
}

/** Under this many seconds to the reap, the row takes a warn tint. */
export const REAP_WARN_SECS = 30

/** Seconds left before the reap, aged forward from the snapshot the
 *  reap figure rode in on. Floors at 0 — the caller renders "reaping…". */
export function reapCountdownSecs(
  reapInSecs: number,
  snapshotAt: number | null,
  nowMs: number,
): number {
  const elapsed =
    snapshotAt === null || nowMs <= snapshotAt ? 0 : (nowMs - snapshotAt) / 1000
  return Math.max(0, reapInSecs - elapsed)
}

/** `300000` → `5m`, `90500` → `1m 30s`, `800.7` → `800ms` — sub-second
 *  values floor to whole ms, everything else to whole seconds. */
export function formatMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '—'
  if (ms < 1000) return `${Math.floor(ms)}ms`
  const s = Math.floor(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  const rest = s % 60
  return rest ? `${m}m ${rest}s` : `${m}m`
}

/** `900` → `15m`, `45` → `45s`. Seconds-denominated config knobs. */
export function formatSecs(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return '—'
  return formatMs(secs * 1000)
}

/** `sbx-3f9a2c1e-…` → `sbx-3f9a2c1e…` — enough head to tell rows apart. */
export function truncateId(id: string, keep = 13): string {
  return id.length > keep + 1 ? `${id.slice(0, keep)}…` : id
}

/** `/very/long/path/to/file.txt` → `file.txt`; `/` stays `/`. */
export function basename(path: string): string {
  const clean = path.replace(/\/+$/, '')
  if (!clean) return path ? '/' : ''
  const idx = clean.lastIndexOf('/')
  return idx < 0 ? clean : clean.slice(idx + 1) || '/'
}
