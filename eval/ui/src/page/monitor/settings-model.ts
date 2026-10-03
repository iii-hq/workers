// Pure logic behind the monitor settings form: the model catalog view, the
// draft, what counts as a change, and the copy that depends on data. No React.
import { formatBytes, formatDuration } from '@iii-dev/console-ui/format'
import type {
  CatalogModel,
  MonitorConfig,
  MonitorLimits,
  MonitorModel,
  ThinkingLevel,
  TriageAvailability,
} from '../../types'
import { count as grouped, investigationCaps, plural } from './detail/present'

export const THINKING_LABEL: Record<ThinkingLevel, string> = {
  minimal: 'Minimal',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra high',
}

/** The `Select` value that means "send no thinking level". */
export const PROVIDER_DEFAULT = 'default'

export const JUDGE = 'judge-typesafe'

export interface ModelEntry {
  key: string
  model: string
  provider: string
  supportsThinking: boolean
  supportsXhigh: boolean
  displayName?: string
  /** False for the saved model when the catalog does not list it (or is down). */
  listed: boolean
}

export interface CatalogOption {
  value: string
  label: string
  description?: string
  keywords: string[]
}

export interface CatalogView {
  groups: Array<{ label: string; options: CatalogOption[] }>
  byKey: Map<string, ModelEntry>
}

/** One unambiguous string per provider + model, for `Selector.value`. */
export function modelKey(provider: string, model: string): string {
  return JSON.stringify([provider, model])
}

export function modelLabel(entry: { model: string; provider: string }): string {
  return `${entry.model} · ${entry.provider}`
}

/** `claude-sonnet-5-5 · medium`: how notices name the model an analysis uses. */
export function modelWithLevel(model: MonitorModel): string {
  return model.thinking_level ? `${model.model} · ${model.thinking_level}` : model.model
}

/**
 * The catalog grouped by provider. The saved model is always present, taken
 * from the saved config: the catalog never decides what is saved.
 */
export function buildCatalog(models: readonly CatalogModel[], saved: MonitorModel | null): CatalogView {
  const byKey = new Map<string, ModelEntry>()
  for (const model of models) {
    const key = modelKey(model.provider, model.id)
    if (byKey.has(key)) continue
    byKey.set(key, {
      key,
      model: model.id,
      provider: model.provider,
      supportsThinking: model.supports_thinking === true,
      supportsXhigh: model.supports_xhigh === true,
      displayName: model.display_name,
      listed: true,
    })
  }
  if (saved) {
    const key = modelKey(saved.provider, saved.model)
    if (!byKey.has(key)) {
      byKey.set(key, {
        key,
        model: saved.model,
        provider: saved.provider,
        supportsThinking: Boolean(saved.thinking_level),
        supportsXhigh: saved.thinking_level === 'xhigh',
        listed: false,
      })
    }
  }

  const byProvider = new Map<string, ModelEntry[]>()
  for (const entry of byKey.values()) {
    const bucket = byProvider.get(entry.provider) ?? []
    bucket.push(entry)
    byProvider.set(entry.provider, bucket)
  }
  const groups = [...byProvider.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([label, entries]) => ({
      label,
      options: entries
        .sort((a, b) => a.model.localeCompare(b.model))
        .map((entry) => ({
          value: entry.key,
          label: modelLabel(entry),
          description: !entry.listed
            ? 'Saved model, not in the catalog'
            : entry.supportsThinking
              ? 'thinking'
              : undefined,
          keywords: [entry.provider, entry.displayName ?? ''].filter(Boolean),
        })),
    }))
  return { groups, byKey }
}

const BASE_LEVELS: ThinkingLevel[] = ['minimal', 'low', 'medium', 'high']

/** Levels the model accepts; empty means the thinking field does not apply. */
export function thinkingChoices(entry: ModelEntry | undefined): ThinkingLevel[] {
  if (!entry?.supportsThinking) return []
  return entry.supportsXhigh ? [...BASE_LEVELS, 'xhigh'] : BASE_LEVELS
}

export interface Draft {
  enabled: boolean
  modelKey: string | null
  thinking: ThinkingLevel | null
  /** The codebase directory as typed; empty means code access off. */
  codeDirectory: string
}

export function draftFromConfig(config: MonitorConfig | null): Draft {
  return {
    enabled: config?.enabled ?? false,
    modelKey: config ? modelKey(config.model.provider, config.model.model) : null,
    thinking: config?.model.thinking_level ?? null,
    codeDirectory: config?.code_repository ?? '',
  }
}

export function isDirty(base: Draft, draft: Draft): boolean {
  return (
    base.enabled !== draft.enabled ||
    base.modelKey !== draft.modelKey ||
    base.thinking !== draft.thinking ||
    base.codeDirectory.trim() !== draft.codeDirectory.trim()
  )
}

/** The directory `eval::configure` gets: trimmed, and `undefined` (access off) when blank. */
export function codeRepository(draft: Draft): string | undefined {
  return draft.codeDirectory.trim() || undefined
}

/**
 * The backend's refusal of a directory, as the message under the field. Any
 * other failure stays a save error: it says nothing about this field.
 */
export function codeDirectoryError(message: string): string | null {
  if (message.includes('code_repository must be an absolute path')) {
    return 'Use an absolute path, for example /home/you/workspaces/workers'
  }
  const notDirectory = /code_repository (.+?) is not a directory/.exec(message)
  if (notDirectory) return `Not a directory: ${notDirectory[1]}`
  // Missing and unreadable alike: the system's own reason says which.
  const unreadable = /code_repository (.+?) is not readable(?:: (.+))?$/.exec(message)
  return unreadable ? `Can't read ${unreadable[1]}${unreadable[2] ? `: ${unreadable[2]}` : ''}` : null
}

/**
 * Under the directory field: the investigation may call any function for now, so
 * what the directory does and does not protect is said next to it.
 */
export const CODE_ACCESS_RISK =
  'For now the investigation may call any function, including ones that change files or start sessions, ' +
  'while it reads untrusted transcripts. It is told to stay read-only, but the directory does not confine absolute paths.'

/** Picking a model keeps the level only when the new model accepts it. */
export function pickModel(draft: Draft, entry: ModelEntry): Draft {
  const keep = draft.thinking !== null && thinkingChoices(entry).includes(draft.thinking)
  return { ...draft, modelKey: entry.key, thinking: keep ? draft.thinking : null }
}

/**
 * The model sent to `eval::configure`. The backend compares it with the saved
 * one and skips the catalog only when they are equal, so an unchanged model
 * keeps its saved provider options and an omitted level stays omitted.
 */
export function toMonitorModel(
  entry: ModelEntry,
  thinking: ThinkingLevel | null,
  saved: MonitorModel | null,
): MonitorModel {
  const same = saved?.provider === entry.provider && saved.model === entry.model
  return {
    model: entry.model,
    provider: entry.provider,
    ...(thinking ? { thinking_level: thinking } : {}),
    ...(same && saved.provider_options ? { provider_options: saved.provider_options } : {}),
  }
}

export function sameModel(a: MonitorModel, b: MonitorModel): boolean {
  return a.model === b.model && a.provider === b.provider && (a.thinking_level ?? null) === (b.thinking_level ?? null)
}

/** The helper under the directory field; the caps wait until the monitor reports them. */
export function codeDirectoryHelp(limits: MonitorLimits | undefined): string {
  const caps = limits
    ? ` With it, an analysis may take up to ${plural(limits.investigation_code_max_turns, 'step')} and ${tokenCap(limits.investigation_code_max_total_tokens)} tokens.`
    : ''
  return `Absolute path on the machine that runs the monitor, normally your clone of the iii workers repository. The investigation runs in this directory like a chat with it selected and reads the current code of every worker, not only the Harness.${caps} Leave it empty to keep code access off.`
}

/** `800k` for round thousands, every other number written out (`16,384`). */
function tokenCap(tokens: number): string {
  return tokens >= 1000 && tokens % 1000 === 0 ? `${tokens / 1000}k` : grouped(tokens)
}

/**
 * The Limits section, as the monitor reports them: `[label, value]` rows.
 * Fixed for this version of the monitor, so the form only states them. The
 * investigation caps follow the saved setting: higher with a code directory.
 */
export function limitRows(limits: MonitorLimits, codeAccess: boolean): Array<[label: string, value: string]> {
  const finished = limits.retention_max_terminal === 1 ? 'analysis' : 'analyses'
  const caps = investigationCaps(limits, codeAccess)
  return [
    ['Deadline', `${formatDuration(limits.analysis_budget_ms)} per analysis, queue included`],
    ['Concurrency', `${limits.queue_concurrency} running · ${limits.max_active_analyses} not finished`],
    ['Context to models', `${formatBytes(limits.model_context_bytes)} of JSON`],
    ['Investigation steps', `${caps.steps}`],
    [
      'Investigation tokens',
      `${tokenCap(caps.totalTokens)} total · ${tokenCap(limits.investigation_max_output_tokens)} output`,
    ],
    ['Audit sample', `${limits.audit_sample_percent}% of sessions with no signal`],
    [
      'Retention',
      `${plural(limits.retention_days, 'day')} · ${grouped(limits.retention_max_terminal)} finished ${finished}`,
    ],
  ]
}

/**
 * True when no analysis can still be running under an older config: only then
 * "running with <model>" names the saved model truthfully. An analysis never
 * outlives its deadline (`analysis_budget_ms`); until that is known nothing is
 * settled.
 */
function settled(config: MonitorConfig, now: number, budgetMs: number | undefined): boolean {
  return budgetMs !== undefined && now - config.updated_at > budgetMs
}

function running(count: number): { subject: string; verb: string } {
  return count === 1
    ? { subject: '1 running analysis', verb: 'keeps' }
    : { subject: `${count} running analyses`, verb: 'keep' }
}

/** The line under the model fields: running analyses keep their model. */
export function runningHint(
  count: number,
  config: MonitorConfig,
  now: number,
  budgetMs: number | undefined,
): string | null {
  if (count <= 0) return null
  const one = count === 1
  const noun = `${count} ${one ? 'analysis is' : 'analyses are'} running`
  const keep = one ? 'It keeps' : 'They keep'
  const tail = 'A change here applies to new analyses only.'
  return settled(config, now, budgetMs)
    ? `${noun} with ${modelWithLevel(config.model)}. ${keep} it. ${tail}`
    : `${noun}. ${keep} the model ${one ? 'it' : 'they'} started with. ${tail}`
}

/** What a save changed and what it did not, in the order an operator asks. */
export function savedNotice(input: {
  before: MonitorConfig | null
  after: MonitorConfig
  runningCount: number
  now: number
  /** `analysis_budget_ms`, when the monitor's state has been read. */
  budgetMs: number | undefined
}): string {
  const { before, after, runningCount, now, budgetMs } = input
  // Clearing the directory is said; a monitor that never had one says nothing.
  const code = after.code_repository
    ? ` and read code in ${after.code_repository}`
    : before?.code_repository
      ? ' and have no code access'
      : ''
  const parts = [`New analyses use ${modelWithLevel(after.model)}${code}.`]
  if (before && runningCount > 0 && !sameModel(before.model, after.model)) {
    const { subject, verb } = running(runningCount)
    parts.push(
      settled(before, now, budgetMs)
        ? `${subject} ${verb} ${modelWithLevel(before.model)}.`
        : `${subject} ${verb} the model ${runningCount === 1 ? 'it' : 'they'} started with.`,
    )
  }
  parts.push(`Observation is ${after.enabled ? 'on' : 'paused'}.`)
  return parts.join(' ')
}

export function triageHint(code: string | undefined): string {
  switch (code) {
    case 'missing_key':
      return 'Add the key in judge-typesafe settings.'
    case 'provider_unavailable':
    case 'unreachable':
      return 'Start the judge and judge-typesafe workers.'
    default:
      return 'Check judge-typesafe.'
  }
}

export function triageWarning(triage: TriageAvailability): string {
  const until = triage.code === 'missing_key' ? 'Until the key is added' : 'Until triage is available'
  return `${until}, new analyses fail at triage. You can still save these settings.`
}
