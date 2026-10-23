import { parseDocument } from 'yaml'
import type {
  Checkout,
  Container,
  ContainerEntry,
  DeclaredContainer,
  EditPatch,
} from './compose-api'
import { validateSettingsDraft } from './settings-validation'
import {
  MANAGEMENT_LABEL,
  type WorkerConnectionStatus,
  type WorkerRow,
} from './types'

export { validateSettingsDraft } from './settings-validation'

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
          ? shortPath(d.ref)
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

/**
 * `compose::add` keys a package after its name, so it changes the version of
 * a container in place only when the container carries that name.
 */
export const inPlaceVersion = (
  declared: Pick<DeclaredContainer, 'name' | 'ref' | 'source'>,
) => declared.source === 'package' && basename(declared.ref) === declared.name

/**
 * After an add or a remove Compose brings every declared container up, the
 * ones stopped on purpose included: name them before the operator agrees.
 */
export function alsoStarts(
  idle: readonly string[],
  except: readonly string[] = [],
): string[] {
  const names = idle.filter((name) => !except.includes(name))
  return names.length ? [`Also starts ${names.join(', ')}, stopped now`] : []
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

/** Matches the compose entry reader's literal-credential classification. */
export function isSecretValue(key: string, value: string): boolean {
  const upper = key.replace(/[a-z]/g, (letter) => letter.toUpperCase())
  return (
    ['KEY', 'TOKEN', 'SECRET', 'PASSWORD', 'CREDENTIAL'].some((word) =>
      upper.includes(word),
    ) &&
    value !== '' &&
    !(value.startsWith('${') && value.endsWith('}'))
  )
}

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

/** Explains the new cycle before a dependency can be selected or submitted. */
export function dependencyBlockReason(
  containers: readonly Pick<DeclaredContainer, 'name' | 'start_after'>[],
  name: string,
  candidate: string,
): string | null {
  const graph = new Map(
    containers.map((container) => [container.name, container.start_after]),
  )
  const visited = new Set<string>()
  const findPath = (current: string, path: string[]): string[] | null => {
    if (current === name) return path
    if (visited.has(current)) return null
    visited.add(current)
    for (const dependency of graph.get(current) ?? []) {
      const found = findPath(dependency, [...path, dependency])
      if (found) return found
    }
    return null
  }
  const path = findPath(candidate, [name, candidate])
  return path ? `Would create a startup cycle: ${path.join(' → ')}.` : null
}

/** Stages known previous values; masked credentials always keep their current state. */
export function restoreSettingsDraft(
  current: ContainerEntry,
  previous: ContainerEntry,
): { draft: SettingsDraft; unavailableSecrets: string[] } {
  const unavailable = (variable: ContainerEntry['environment'][number]) =>
    variable.secret ||
    variable.value === MASK ||
    isSecretValue(variable.key, variable.value ?? '')
  const previousEnv = new Map(
    previous.environment.map((variable) => [variable.key, variable]),
  )
  const currentKeys = new Set(
    current.environment.map((variable) => variable.key),
  )
  const draft = draftFrom(current)
  draft.run = previous.run ?? ''
  draft.startAfter = [...previous.start_after]
  draft.config = previous.config_override ?? ''
  draft.env = draft.env.map((row, index) => {
    if (unavailable(current.environment[index])) return row
    const original = previousEnv.get(row.key)
    if (!original) return { ...row, removed: true }
    return unavailable(original) ? row : { ...row, value: original.value ?? '' }
  })
  for (const variable of previous.environment) {
    if (currentKeys.has(variable.key) || unavailable(variable)) continue
    draft.env.push({
      key: variable.key,
      value: variable.value ?? '',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
  }
  return {
    draft,
    unavailableSecrets: previous.environment
      .filter(unavailable)
      .map((variable) => variable.key),
  }
}

const sameList = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && [...a].sort().every((v, i) => v === [...b].sort()[i])

/** Includes unfinished edits that cannot yet be submitted as a patch. */
export function hasSettingsChanges(
  entry: ContainerEntry,
  draft: SettingsDraft,
): boolean {
  return (
    draft.run.trim() !== (entry.run ?? '') ||
    !sameList(draft.startAfter, entry.start_after) ||
    draft.config.trim() !== (entry.config_override ?? '').trim() ||
    draft.env.some((row, index) => {
      if (row.isNew) return !row.removed
      const original = entry.environment[index]
      return (
        row.removed ||
        row.key !== original?.key ||
        (row.secret ? row.replacing : row.value !== (original?.value ?? ''))
      )
    })
  )
}

export function settingsPatch(
  entry: ContainerEntry,
  draft: SettingsDraft,
): { patch: EditPatch; changes: number } {
  const validation = validateSettingsDraft(entry, draft)
  if (!validation.valid) {
    throw new Error(Object.values(validation.env)[0] ?? validation.config ?? '')
  }
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
  const set: Record<string, string> = Object.create(null)
  const unset: string[] = []
  draft.env.forEach((row, i) => {
    const original = entry.environment[i]
    if (row.isNew) {
      if (!row.removed) set[row.key] = row.value
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

/** A confirmed write becomes a masked baseline while its actual readback is unavailable. */
export function submittedSettingsEntry(
  entry: ContainerEntry,
  patch: EditPatch,
): ContainerEntry {
  const safeVariable = (key: string, value: string | null, secret = false) => {
    const masked = secret || value === MASK || isSecretValue(key, value ?? '')
    return { key, value: masked || value === '' ? null : value, secret: masked }
  }
  const environment = new Map(
    entry.environment.map((variable) => [
      variable.key,
      safeVariable(variable.key, variable.value, variable.secret),
    ]),
  )
  if (patch.environment) {
    for (const key of patch.environment.unset) environment.delete(key)
    for (const [key, value] of Object.entries(patch.environment.set))
      environment.set(key, safeVariable(key, value))
  }
  return {
    ...entry,
    worker: patch.worker ?? entry.worker,
    run: patch.run ?? entry.run,
    start_after: [...(patch.start_after ?? entry.start_after)],
    env_file: [...entry.env_file],
    config_override: patch.config_override ?? entry.config_override,
    environment: [...environment.values()],
  }
}

/** Keeps newer edits across readback, with existing environment rows aligned to that read. */
export function rebaseSettingsDraft(
  baseline: ContainerEntry,
  draft: SettingsDraft,
  loaded: ContainerEntry,
): SettingsDraft {
  const next = draftFrom(loaded)
  if (draft.run.trim() !== (baseline.run ?? '')) next.run = draft.run
  if (!sameList(draft.startAfter, baseline.start_after))
    next.startAfter = [...draft.startAfter]
  if (draft.config.trim() !== (baseline.config_override ?? '').trim())
    next.config = draft.config
  const pending = new Map<string, EnvDraft>()
  const added: EnvDraft[] = []
  draft.env.forEach((row, index) => {
    if (row.isNew) {
      added.push({ ...row })
      return
    }
    const original = baseline.environment[index]
    if (
      original &&
      (row.removed ||
        row.key !== original.key ||
        (row.secret ? row.replacing : row.value !== (original.value ?? '')))
    )
      pending.set(original.key, { ...row })
  })
  next.env = next.env.map((row) => {
    const edit = pending.get(row.key)
    if (!edit) return row
    pending.delete(row.key)
    return {
      ...edit,
      secret: row.secret || edit.secret,
      replacing: edit.replacing || (row.secret && !edit.removed),
      isNew: false,
    }
  })
  for (const row of pending.values()) {
    if (!row.removed) next.env.push({ ...row, isNew: true })
  }
  const baselineKeys = new Set(baseline.environment.map((row) => row.key))
  const loadedIndexes = new Map(
    loaded.environment.map((row, index) => [row.key, index]),
  )
  const addedCounts = new Map<string, number>()
  for (const row of added) {
    if (!row.removed)
      addedCounts.set(row.key, (addedCounts.get(row.key) ?? 0) + 1)
  }
  for (const row of added) {
    const index = loadedIndexes.get(row.key)
    if (
      index !== undefined &&
      !row.removed &&
      !baselineKeys.has(row.key) &&
      addedCounts.get(row.key) === 1
    ) {
      const loadedRow = next.env[index]
      next.env[index] = {
        ...loadedRow,
        value: row.value,
        secret: loadedRow.secret || row.secret,
        replacing: loadedRow.secret || row.replacing,
      }
    } else next.env.push(row)
  }
  return next
}

const comparableConfig = (source: string) => {
  const document = parseDocument(source, { uniqueKeys: true, merge: true })
  if (document.errors.length) throw document.errors[0]
  return JSON.stringify(document.toJS(), (_key, value) =>
    value && typeof value === 'object' && !Array.isArray(value)
      ? Object.fromEntries(
          Object.entries(value).sort(([a], [b]) => a.localeCompare(b)),
        )
      : value,
  )
}

/** A masked secret cannot prove a write; observable mismatches always win. */
export function confirmStoredPatch(
  entry: ContainerEntry,
  patch: EditPatch,
): boolean | null {
  if (patch.worker !== undefined && patch.worker !== entry.worker) return false
  if (patch.run !== undefined && patch.run.trim() !== (entry.run ?? '').trim())
    return false
  if (patch.start_after && !sameList(patch.start_after, entry.start_after))
    return false
  if (patch.config_override !== undefined) {
    try {
      if (
        comparableConfig(patch.config_override) !==
        comparableConfig(entry.config_override ?? '')
      )
        return false
    } catch {
      return false
    }
  }
  let masked = false
  if (patch.environment) {
    const { set, unset } = patch.environment
    const environment = new Map(entry.environment.map((row) => [row.key, row]))
    if (unset.some((key) => !Object.hasOwn(set, key) && environment.has(key)))
      return false
    for (const [key, value] of Object.entries(set)) {
      const row = environment.get(key)
      if (!row) return false
      if (row.secret && row.value === null) masked = true
      else if ((row.value ?? '') !== value) return false
    }
  }
  return masked ? null : true
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
          value:
            row.secret || isSecretValue(row.key, row.value)
              ? row.replacing && row.value
                ? `${MASK}  # new value`
                : MASK
              : row.value,
        }))
    : entry.environment.map((v) => ({
        key: v.key,
        value:
          v.secret || isSecretValue(v.key, v.value ?? '')
            ? MASK
            : (v.value ?? ''),
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

/** The start command a local worker needs when its manifest has no
    `scripts.start` (compose fails it with MISSING_START_COMMAND otherwise):
    for a Rust worker, `cargo run` of its binary, as the hand-written entries
    do; '' when there is no default to offer. */
export function defaultRun(manifest: {
  name: string
  language: string | null
  bin: string | null
}): string {
  return manifest.language === 'rust'
    ? `cargo run --locked --bin ${manifest.bin ?? manifest.name}`
    : ''
}
