/**
 * Simulated real-time span feed for the timeline stories. History for the
 * past ~70s is pre-seeded so the window is full at mount, then a 200ms
 * scheduler keeps spawning spans, completing them at their planned end,
 * and pruning ones that scrolled out.
 *
 * Shared by Timeline.stories (raw component lab) and TimelineStrip.stories
 * (the masthead composition) — CSF treats named exports as stories, so the
 * sim lives here instead of inside a .stories file.
 */

import { useEffect, useState } from 'react'
import type {
  TimelineSpan,
  TimelineSpanKind,
} from '../../components/timeline/Timeline'

export interface Scenario {
  /** random delay range between spawns */
  spawnEveryMs: readonly [number, number]
  /** random span duration range */
  durationMs: readonly [number, number]
  kinds?: readonly TimelineSpanKind[]
  /** 0..1 chance a span resolves as an error */
  errorRate?: number
  /** skip spawns while this many spans are already running */
  maxConcurrent?: number
  /** periodically fire a clump of spans at once */
  burst?: { everyMs: number; size: number }
}

type SimSpan = TimelineSpan & { plannedEnd: number; willError: boolean }

interface SimState {
  seq: number
  list: SimSpan[]
  nextSpawn: number
  nextBurst: number
}

export const ALL_KINDS = ['zap', 'sparkle', 'flame', 'lambda'] as const

// Colors come from the production path: each label hashes into
// `SERVICE_PALETTE` via `getServiceColor`, exactly like real trace data.
export const LABELS = [
  'ingest',
  'route',
  'invoke',
  'persist',
  'notify',
  'render',
  'index',
  'fanout',
] as const

const SEED_MS = 72_000
const PRUNE_AFTER_MS = 75_000

function rand(min: number, max: number): number {
  return min + Math.random() * (max - min)
}

function pick<T>(values: readonly T[]): T {
  return values[Math.floor(Math.random() * values.length)]
}

function countRunning(list: readonly SimSpan[], t: number): number {
  let n = 0
  for (const s of list) if (s.startTime <= t && s.plannedEnd > t) n++
  return n
}

function spawnAt(state: SimState, t: number, scenario: Scenario): void {
  const duration = rand(...scenario.durationMs)
  state.list.push({
    id: `sim-${state.seq++}`,
    label: pick(LABELS),
    startTime: t,
    endTime: null,
    status: 'pending',
    kind: pick(scenario.kinds ?? ALL_KINDS),
    plannedEnd: t + duration,
    willError: Math.random() < (scenario.errorRate ?? 0),
  })
}

/** Replay the spawn/burst schedule up to wall-clock `t` (also used to seed). */
function advance(state: SimState, t: number, scenario: Scenario): boolean {
  let spawned = false
  while (state.nextSpawn <= t) {
    const at = state.nextSpawn
    const cap = scenario.maxConcurrent
    if (!cap || countRunning(state.list, at) < cap) {
      spawnAt(state, at, scenario)
      spawned = true
    }
    state.nextSpawn = at + rand(...scenario.spawnEveryMs)
  }
  if (scenario.burst) {
    while (state.nextBurst <= t) {
      for (let i = 0; i < scenario.burst.size; i++) {
        spawnAt(state, state.nextBurst + i * 120, scenario)
      }
      spawned = true
      state.nextBurst += scenario.burst.everyMs
    }
  }
  return spawned
}

/**
 * Drive a live span feed from a scenario. `frozen` seeds the past window
 * once and never ticks again — for "paused" story states.
 */
export function useLiveSpans(
  scenario: Scenario,
  opts?: { frozen?: boolean },
): TimelineSpan[] {
  const frozen = opts?.frozen ?? false
  const [spans, setSpans] = useState<TimelineSpan[]>([])

  useEffect(() => {
    const now = Date.now()
    const state: SimState = {
      seq: 0,
      list: [],
      nextSpawn: now - SEED_MS,
      nextBurst: scenario.burst
        ? now - SEED_MS + scenario.burst.everyMs / 2
        : Number.POSITIVE_INFINITY,
    }

    const step = (): void => {
      const t = Date.now()
      const spawned = advance(state, t, scenario)
      let changed = spawned
      state.list = state.list.map((s) => {
        if (s.endTime == null && s.plannedEnd <= t) {
          changed = true
          return {
            ...s,
            endTime: s.plannedEnd,
            status: s.willError ? 'error' : 'ok',
          }
        }
        return s
      })
      const before = state.list.length
      state.list = state.list.filter(
        (s) => (s.endTime ?? t) >= t - PRUNE_AFTER_MS,
      )
      changed ||= state.list.length !== before
      if (changed) setSpans([...state.list])
    }

    step() // seed the past window synchronously so the story mounts full
    if (frozen) return
    const interval = setInterval(step, 200)
    return () => clearInterval(interval)
  }, [scenario, frozen])

  return spans
}

/* ------------------------------------------------------------------ */
/* scenario presets                                                     */
/* ------------------------------------------------------------------ */

export const STEADY: Scenario = {
  spawnEveryMs: [900, 2200],
  durationMs: [800, 7000],
  errorRate: 0.05,
}

export const SPARSE: Scenario = {
  spawnEveryMs: [6000, 12000],
  durationMs: [700, 2600],
  kinds: ['zap'],
}

export const BURSTS: Scenario = {
  spawnEveryMs: [2600, 5200],
  durationMs: [1500, 6000],
  errorRate: 0.04,
  burst: { everyMs: 14_000, size: 9 },
}

export const OVERLOADED: Scenario = {
  spawnEveryMs: [260, 700],
  durationMs: [6000, 16_000],
  errorRate: 0.03,
  maxConcurrent: 18,
}

export const LONG_RUNNING: Scenario = {
  spawnEveryMs: [2800, 6500],
  durationMs: [15_000, 40_000],
  kinds: ['lambda', 'zap'],
}

export const MIXED: Scenario = {
  spawnEveryMs: [650, 1500],
  durationMs: [400, 9000],
  kinds: ALL_KINDS,
  errorRate: 0.06,
}

export const WITH_ERRORS: Scenario = {
  spawnEveryMs: [1000, 2400],
  durationMs: [800, 6000],
  errorRate: 0.35,
}

/**
 * Dead air between clumps: a couple of spans roughly every 30s and nothing
 * in between — exercises the freeze-at-last-span park and the catch-up
 * whoosh when the next clump arrives.
 */
export const IDLE_GAPS: Scenario = {
  spawnEveryMs: [26_000, 36_000],
  durationMs: [1_200, 4_500],
  kinds: ALL_KINDS,
  errorRate: 0.1,
  burst: { everyMs: 31_000, size: 2 },
}
