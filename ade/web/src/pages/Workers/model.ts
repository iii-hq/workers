import type {
  Checkout,
  Container,
  ContainerEntry,
  DeclaredContainer,
  EditPatch,
} from './compose-api'
import {
  MANAGEMENT_LABEL,
  type WorkerConnectionStatus,
  type WorkerRow,
} from './types'

export type Tone = 'accent' | 'alert' | 'warn' | 'ink'
export type BadgeTone = 'ok' | 'warn' | 'alert' | 'default'

const TONES: Record<string, { dot: Tone; badge: BadgeTone }> = {
  ready: { dot: 'accent', badge: 'ok' },
  starting: { dot: 'warn', badge: 'warn' },
  restarting: { dot: 'warn', badge: 'warn' },
  failed: { dot: 'alert', badge: 'alert' },
  stopped: { dot: 'ink', badge: 'default' },
}
export const toneFor = (state: string) => TONES[state] ?? TONES.stopped
export const isRunning = (state: string) =>
  state === 'ready' || state === 'starting'
const needsAttention = (state: string) =>
  state === 'failed' || state === 'restarting'

export const basename = (path: string) =>
  path.replace(/\/+$/, '').split('/').pop() ?? path
export const dirname = (path: string) =>
  path.replace(/\/+$/, '').split('/').slice(0, -1).join('/') || '/'

/** `/home/me/x` → `~/x`; the daemon host's home is not known here, so match the usual shapes. */
export function shortPath(path: string): string {
  return path.replace(/^\/(?:home|Users)\/[^/]+(?=\/|$)/, '~')
}

export type ListItem = {
  name: string
  state: string
  source: DeclaredContainer['source'] | null
  /** One line under the name: the failure, the pinned version, or the checkout. */
  detail: string
  failed: boolean
}
export type ListGroup = {
  id: 'attention' | 'registry' | 'path' | 'other' | 'outside'
  label: string
  items: ListItem[]
}

export function groupContainers(
  containers: readonly Container[],
  declared: ReadonlyMap<string, DeclaredContainer>,
  query = '',
  workers: readonly WorkerRow[] = [],
): ListGroup[] {
  const needle = query.trim().toLowerCase()
  const groups: ListGroup[] = [
    { id: 'attention', label: 'Needs attention', items: [] },
    { id: 'registry', label: 'Registry', items: [] },
    { id: 'path', label: 'Local path', items: [] },
    { id: 'other', label: 'Other', items: [] },
    { id: 'outside', label: 'Outside compose', items: [] },
  ]
  for (const c of containers) {
    if (needle && !c.container.toLowerCase().includes(needle)) continue
    const d = declared.get(c.container)
    const failed = needsAttention(c.state)
    const detail = failed
      ? (c.last_error ?? c.state)
      : d?.source === 'package'
        ? (d.version ?? 'unpinned')
        : d?.source === 'path'
          ? basename(dirname(d.ref))
          : c.state
    const item = {
      name: c.container,
      state: c.state,
      source: d?.source ?? null,
      detail,
      failed,
    }
    const id = failed
      ? 'attention'
      : d?.source === 'package'
        ? 'registry'
        : d?.source === 'path'
          ? 'path'
          : 'other'
    groups.find((g) => g.id === id)?.items.push(item)
  }
  const outside = groups[4]
  for (const w of workers) {
    if (
      w.managementKind === 'compose' ||
      (needle && !w.name.toLowerCase().includes(needle))
    )
      continue
    outside.items.push({
      name: w.name,
      state: CONNECTION_STATE[w.status],
      source: null,
      detail: [MANAGEMENT_LABEL[w.managementKind], w.runtime, w.version]
        .filter(Boolean)
        .join(' · '),
      failed: false,
    })
  }
  return groups.filter((g) => g.items.length > 0)
}

/** An engine worker's connection, spoken as a compose state for dots and badges. */
const CONNECTION_STATE: Record<WorkerConnectionStatus, string> = {
  connected: 'ready',
  starting: 'starting',
  failed: 'failed',
  disconnected: 'stopped',
  stopped: 'stopped',
}

/** Containers grouped by `start_after` depth: step 1 starts with the engine. */
export function startWaves(declared: readonly DeclaredContainer[]): string[][] {
  const byName = new Map(declared.map((d) => [d.name, d]))
  const depth = new Map<string, number>()
  const visit = (name: string, trail: Set<string>): number => {
    const known = depth.get(name)
    if (known !== undefined) return known
    if (trail.has(name)) return 0
    trail.add(name)
    const deps = (byName.get(name)?.start_after ?? []).filter((dep) =>
      byName.has(dep),
    )
    const level = deps.length
      ? 1 + Math.max(...deps.map((dep) => visit(dep, trail)))
      : 0
    trail.delete(name)
    depth.set(name, level)
    return level
  }
  const waves: string[][] = []
  for (const d of declared) {
    const level = visit(d.name, new Set())
    waves[level] = [...(waves[level] ?? []), d.name]
  }
  return waves.filter(Boolean)
}

/** What a later wave waits for: its one dependency by name, or how many. */
export function waveLabel(
  wave: readonly string[],
  declared: readonly DeclaredContainer[],
): string {
  const names = new Set(declared.map((d) => d.name))
  const deps = new Set(
    declared
      .filter((d) => wave.includes(d.name))
      .flatMap((d) => d.start_after)
      .filter((dep) => names.has(dep)),
  )
  const [only] = deps
  return deps.size === 1 ? `After ${only}` : `After ${deps.size} containers`
}

export const dependentsOf = (
  declared: readonly DeclaredContainer[],
  name: string,
) => declared.filter((d) => d.start_after.includes(name)).map((d) => d.name)

/** Where `query` sits in `text`, for a highlight: before, match, after. */
export function matchParts(
  text: string,
  query: string,
): [string, string, string] {
  const needle = query.trim().toLowerCase()
  const at = needle ? text.toLowerCase().indexOf(needle) : -1
  return at < 0
    ? [text, '', '']
    : [
        text.slice(0, at),
        text.slice(at, at + needle.length),
        text.slice(at + needle.length),
      ]
}

export const branchLabel = (checkout: Pick<Checkout, 'branch'>) =>
  checkout.branch ?? 'detached HEAD'

/**
 * Checkouts a container can point at (their folder carries its name) apart
 * from the rest, filtered by branch or folder; the one it runs from first,
 * then the newest commit first.
 */
export function arrangeCheckouts<T extends Checkout>(
  checkouts: readonly T[],
  name: string,
  current: string,
  query = '',
): { usable: T[]; other: T[] } {
  const needle = query.trim().toLowerCase()
  const sorted = checkouts
    .filter(
      (c) =>
        !needle ||
        branchLabel(c).toLowerCase().includes(needle) ||
        shortPath(c.path).toLowerCase().includes(needle),
    )
    .sort(
      (a, b) =>
        Number(b.path === current) - Number(a.path === current) ||
        (b.committed_at ?? 0) - (a.committed_at ?? 0),
    )
  return {
    usable: sorted.filter((c) => basename(c.path) === name),
    other: sorted.filter((c) => basename(c.path) !== name),
  }
}

/** The folder most local workers live in, for the add dialog's starting point. */
export function usualParent(paths: readonly string[]): string | null {
  const counts = new Map<string, number>()
  for (const path of paths)
    counts.set(dirname(path), (counts.get(dirname(path)) ?? 0) + 1)
  let best: string | null = null
  for (const [dir, count] of counts)
    if (best === null || count > (counts.get(best) ?? 0)) best = dir
  return best
}

/* ── Settings drafts ─────────────────────────────────────────────────── */

export const MASK = '••••••••'

export type EnvDraft = {
  key: string
  value: string
  secret: boolean
  /** A secret being replaced: its new value is typed, the old one never shown. */
  replacing: boolean
  removed: boolean
  isNew: boolean
}
export type SettingsDraft = {
  run: string
  startAfter: string[]
  env: EnvDraft[]
  config: string
}

export function draftFrom(entry: ContainerEntry): SettingsDraft {
  return {
    run: entry.run ?? '',
    startAfter: [...entry.start_after],
    env: entry.environment.map((v) => ({
      key: v.key,
      value: v.value ?? '',
      secret: v.secret,
      replacing: false,
      removed: false,
      isNew: false,
    })),
    config: entry.config_override ?? '',
  }
}

const sameList = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && [...a].sort().every((v, i) => v === [...b].sort()[i])

export function settingsPatch(
  entry: ContainerEntry,
  draft: SettingsDraft,
): { patch: EditPatch; changes: number } {
  const patch: EditPatch = {}
  let changes = 0
  if (draft.run.trim() !== (entry.run ?? '')) {
    patch.run = draft.run.trim()
    changes += 1
  }
  if (!sameList(draft.startAfter, entry.start_after)) {
    patch.start_after = draft.startAfter
    changes += 1
  }
  const set: Record<string, string> = {}
  const unset: string[] = []
  draft.env.forEach((row, i) => {
    const original = entry.environment[i]
    if (row.isNew) {
      if (row.key.trim()) set[row.key.trim()] = row.value
    } else if (row.removed) {
      unset.push(row.key)
    } else if (
      row.secret
        ? row.replacing && row.value !== ''
        : row.value !== (original?.value ?? '')
    ) {
      set[row.key] = row.value
    }
  })
  const envChanges = Object.keys(set).length + unset.length
  if (envChanges) {
    patch.environment = { set, unset }
    changes += envChanges
  }
  if (draft.config.trim() !== (entry.config_override ?? '').trim()) {
    patch.config_override = draft.config
    changes += 1
  }
  return { patch, changes }
}

/* ── Preview YAML ────────────────────────────────────────────────────── */

const PLAIN = /^[\w./~${}@:+-]+$/
const scalar = (value: string) =>
  value !== '' &&
  PLAIN.test(value) &&
  !/^(true|false|null|yes|no|[\d.]+)$/i.test(value) &&
  !value.includes(': ')
    ? value
    : JSON.stringify(value)

export type EntryShape = {
  name: string
  worker: string
  version?: string | null
  startAfter?: readonly string[]
  envFile?: readonly string[]
  env?: readonly { key: string; value: string }[]
  run?: string | null
  config?: string | null
}

/** One container block as it reads in worker-compose.yaml, for `FileDiff`. */
export function entryYaml(e: EntryShape): string {
  const lines = [`  ${e.name}:`, `    worker: ${e.worker}`]
  if (e.version) lines.push(`    version: ${JSON.stringify(e.version)}`)
  if (e.startAfter?.length)
    lines.push(`    start_after: [${e.startAfter.join(', ')}]`)
  if (e.envFile?.length) lines.push(`    env_file: [${e.envFile.join(', ')}]`)
  if (e.config?.trim()) {
    lines.push('    config_override:')
    for (const line of e.config.trimEnd().split('\n'))
      lines.push(`      ${line}`)
  }
  if (e.env?.length) {
    lines.push('    environment:')
    for (const v of e.env)
      lines.push(
        `      ${v.key}: ${v.value.startsWith(MASK) ? v.value : scalar(v.value)}`,
      )
  }
  if (e.run) lines.push('    scripts:', `      run: ${e.run}`)
  return `${lines.join('\n')}\n`
}

export function entryShape(
  entry: ContainerEntry,
  draft?: SettingsDraft,
): EntryShape {
  const env = draft
    ? draft.env
        .filter((row) => !row.removed && row.key.trim())
        .map((row) => ({
          key: row.key.trim(),
          // A typed secret still never shows: the preview only marks it replaced.
          value: row.secret
            ? row.replacing && row.value
              ? `${MASK}  # new value`
              : MASK
            : row.value,
        }))
    : entry.environment.map((v) => ({
        key: v.key,
        value: v.secret ? MASK : (v.value ?? ''),
      }))
  return {
    name: entry.name,
    worker: entry.worker,
    version: entry.version,
    startAfter: draft ? draft.startAfter : entry.start_after,
    envFile: entry.env_file,
    env,
    run: draft ? draft.run.trim() : entry.run,
    config: draft ? draft.config : entry.config_override,
  }
}

/* ── Log lines ───────────────────────────────────────────────────────── */

export type LogParts = {
  /** Local wall-clock time, `HH:MM:SS.mmm`. */
  time: string
  /** The timestamp as written, for a title. */
  stamp: string
  level: 'TRACE' | 'DEBUG' | 'INFO' | 'WARN' | 'ERROR'
  target: string
  text: string
}

const TRACING =
  /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?Z\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s+(\S+?):\s(.*)$/s
const two = (n: number) => String(n).padStart(2, '0')

/** Split a `tracing` fmt line; anything else (or a coloured line) stays raw. */
export function parseLogLine(line: string): LogParts | null {
  const match = TRACING.exec(line)
  if (!match) return null
  const [, seconds, fraction = '', level, target, text] = match
  const date = new Date(`${seconds}${fraction.slice(0, 4)}Z`)
  if (Number.isNaN(date.getTime())) return null
  return {
    time: `${two(date.getHours())}:${two(date.getMinutes())}:${two(date.getSeconds())}.${String(date.getMilliseconds()).padStart(3, '0')}`,
    stamp: `${seconds}${fraction}Z`,
    level: level as LogParts['level'],
    target,
    text: text.trimStart(),
  }
}
