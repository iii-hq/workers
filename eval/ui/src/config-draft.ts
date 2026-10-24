// The settings form's draft of `LoopConfigV1`: every field a string while it
// is typed, converted only on save. Nothing has a default; an empty field
// stays empty and is refused by `validateConfig`.
import type { CandidateConfig, JudgeConfig, LoopConfig, PromotionConfig, ValidationConfig, Where } from './types'

export interface ConfigDraft {
  provider: string
  model: string
  agent: string
  where: Where | ''
  stackName: string
  stackYaml: string
  runs_per_side: string
  technical_retries: string
  max_scenarios: string
  estimated_usd_per_run: string
  min_effect_tokens: string
  min_effect_pass_rate_pp: string
  max_spend_usd: string
  /** The budget's time limit, typed in minutes. */
  max_duration_min: string
  max_calls: string
  max_candidates: string
  /** Control scenario ids, comma or newline separated. */
  controls: string
  /** Top-level in `LoopConfigV1`, edited with the promotion section and saved only while it is on. */
  release_build_estimate_usd: string
  /** Each optional section: `null` when off, else its fields as typed. */
  sections: Record<SectionId, Record<string, string> | null>
}

export type SectionId = 'judge' | 'candidate' | 'validation' | 'promotion'

export interface FieldSpec {
  /** Dotted path inside the section, e.g. `target.engine_url`. */
  key: string
  label: string
  kind: 'text' | 'int' | 'number' | 'multiline' | 'where' | 'thinking'
  optional?: boolean
  hint?: string
}

export interface SectionSpec {
  id: SectionId
  title: string
  /** What turning it off means, as the contract states it. */
  off: string
  fields: FieldSpec[]
}

/** The four optional sections of `LoopConfigV1`, field for field. */
export const SECTIONS: SectionSpec[] = [
  {
    id: 'judge',
    title: 'Judge',
    off: 'Off: an ambiguous finding stays inconclusive.',
    fields: [
      { key: 'provider', label: 'Judge provider', kind: 'text', hint: 'Routes judge::evaluate, e.g. typesafe.' },
      {
        key: 'estimated_usd',
        label: 'Cost per question (USD)',
        kind: 'number',
        hint: 'Reserved before the judge is asked.',
      },
    ],
  },
  {
    id: 'candidate',
    title: 'Improve',
    off: 'Off: a cycle measures and stops before proposing any change.',
    fields: [
      {
        key: 'repository',
        label: 'Repository',
        kind: 'text',
        hint: "The deployment's trusted Workers checkout, an absolute path.",
      },
      {
        key: 'base_sha',
        label: 'Base commit',
        kind: 'text',
        hint: 'Every candidate starts here (40-hex) and must fast-forward main from it.',
      },
      {
        key: 'worktrees_dir',
        label: 'Worktrees directory',
        kind: 'text',
        hint: 'Where each candidate gets its own worktree, an absolute path.',
      },
      { key: 'provider', label: 'Actor provider', kind: 'text' },
      {
        key: 'model',
        label: 'Actor model',
        kind: 'text',
        hint: 'Writes the change; the read-only proposal uses it too.',
      },
      { key: 'thinking_level', label: 'Thinking level', kind: 'thinking', optional: true },
      { key: 'max_turns', label: 'Max turns', kind: 'int', hint: 'Per candidate.' },
      { key: 'max_output_tokens', label: 'Max output tokens', kind: 'int' },
      { key: 'max_total_tokens', label: 'Max total tokens', kind: 'int' },
      { key: 'max_cost_usd', label: 'Max cost (USD)', kind: 'number', hint: 'Reserved before the actor starts.' },
      {
        key: 'publish_remote',
        label: 'Publish branch to',
        kind: 'text',
        optional: true,
        hint: 'A configured git remote, pushed without force so the validation run can pin the commit; empty publishes nothing.',
      },
    ],
  },
  {
    id: 'validation',
    title: 'Prove',
    off: 'Off: a written change cannot be proved, so the cycle stops before any decision.',
    fields: [
      { key: 'where', label: 'Runs on', kind: 'where', hint: 'The Harness pin needs a stack the E2E builds.' },
      { key: 'stack_name', label: 'Stack name', kind: 'text' },
      {
        key: 'source_stack_template',
        label: 'Candidate stack',
        kind: 'multiline',
        hint: '{commit} becomes the candidate commit.',
      },
      {
        key: 'artifact_stack_template',
        label: 'Release candidate stack',
        kind: 'multiline',
        hint: '{version} becomes the Registry version.',
      },
    ],
  },
  {
    id: 'promotion',
    title: 'Ship',
    off: 'Off: accepted changes stop at the source decision.',
    fields: [
      {
        key: 'artifact_target',
        label: 'Artifact target',
        kind: 'text',
        hint: 'Registry binaries key, e.g. x86_64-unknown-linux-gnu.',
      },
      {
        key: 'target.engine_url',
        label: 'Activation Engine URL',
        kind: 'text',
        hint: 'Explicit local Engine, e.g. ws://127.0.0.1:49134.',
      },
      { key: 'target.namespace', label: 'Activation namespace', kind: 'text' },
      { key: 'target.compose_file', label: 'Activation compose file', kind: 'text' },
    ],
  },
]

/** Every top-level `LoopConfigV1` field. */
const TOP_LEVEL = new Set([
  'subject',
  'where',
  'stack',
  'runs_per_side',
  'technical_retries',
  'max_scenarios',
  'estimated_usd_per_run',
  'min_effect',
  'budget',
  'controls',
  'judge',
  'candidate',
  'validation',
  'promotion',
  'release_build_estimate_usd',
])

const NO_SECTIONS: ConfigDraft['sections'] = { judge: null, candidate: null, validation: null, promotion: null }

/** A section switched on with every field empty: nothing defaulted. */
export function emptySection(id: SectionId): Record<string, string> {
  const spec = SECTIONS.find((section) => section.id === id)
  return Object.fromEntries((spec?.fields ?? []).map((field) => [field.key, '']))
}

function read(value: unknown, path: string): unknown {
  return path
    .split('.')
    .reduce<unknown>(
      (at, key) => (at && typeof at === 'object' ? (at as Record<string, unknown>)[key] : undefined),
      value,
    )
}

function sectionDraft(id: SectionId, value: unknown): Record<string, string> | null {
  if (!value || typeof value !== 'object') return null
  const spec = SECTIONS.find((section) => section.id === id)
  return Object.fromEntries(
    (spec?.fields ?? []).map((field) => {
      const raw = read(value, field.key)
      return [field.key, raw === undefined || raw === null ? '' : String(raw)]
    }),
  )
}

/** The typed section, or `undefined` when it is off. Empty optional fields are omitted. */
function sectionValue(id: SectionId, draft: Record<string, string> | null): Record<string, unknown> | undefined {
  if (!draft) return undefined
  const spec = SECTIONS.find((section) => section.id === id)
  const out: Record<string, unknown> = {}
  for (const field of spec?.fields ?? []) {
    const text = draft[field.key] ?? ''
    if (field.optional && !text.trim()) continue
    const value =
      field.kind === 'int' || field.kind === 'number' ? num(text) : field.kind === 'multiline' ? text : text.trim()
    const keys = field.key.split('.')
    let at = out
    for (const key of keys.slice(0, -1)) {
      at[key] = (at[key] as Record<string, unknown> | undefined) ?? {}
      at = at[key] as Record<string, unknown>
    }
    at[keys[keys.length - 1]] = value
  }
  return out
}

export const EMPTY_DRAFT: ConfigDraft = {
  provider: '',
  model: '',
  agent: '',
  where: '',
  stackName: '',
  stackYaml: '',
  runs_per_side: '',
  technical_retries: '',
  max_scenarios: '',
  estimated_usd_per_run: '',
  min_effect_tokens: '',
  min_effect_pass_rate_pp: '',
  max_spend_usd: '',
  max_duration_min: '',
  max_calls: '',
  max_candidates: '',
  controls: '',
  release_build_estimate_usd: '',
  sections: NO_SECTIONS,
}

export function draftFromConfig(config: LoopConfig): ConfigDraft {
  return {
    provider: config.subject.provider,
    model: config.subject.model,
    agent: config.subject.agent ?? '',
    where: config.where,
    stackName: config.stack?.name ?? '',
    stackYaml: config.stack?.yaml ?? '',
    runs_per_side: String(config.runs_per_side),
    technical_retries: String(config.technical_retries),
    max_scenarios: String(config.max_scenarios),
    estimated_usd_per_run: String(config.estimated_usd_per_run),
    min_effect_tokens: String(config.min_effect.tokens),
    min_effect_pass_rate_pp: String(config.min_effect.pass_rate_pp),
    max_spend_usd: String(config.budget.max_spend_usd),
    max_duration_min: String(config.budget.max_duration_ms / 60_000),
    max_calls: String(config.budget.max_calls),
    max_candidates: String(config.budget.max_candidates),
    controls: (config.controls ?? []).join(', '),
    release_build_estimate_usd:
      config.release_build_estimate_usd === undefined ? '' : String(config.release_build_estimate_usd),
    sections: {
      judge: sectionDraft('judge', config.judge),
      candidate: sectionDraft('candidate', config.candidate),
      validation: sectionDraft('validation', config.validation),
      promotion: sectionDraft('promotion', config.promotion),
    },
  }
}

const optional = <K extends string, V>(key: K, value: V | undefined) => (value === undefined ? {} : { [key]: value })

/** An empty or unparsable number becomes NaN, which validation refuses. */
const num = (text: string) => (text.trim() === '' ? Number.NaN : Number(text))

/** The draft as the config `eval::configure` takes; `null` until a place to run is chosen. */
export function configFromDraft(draft: ConfigDraft): LoopConfig | null {
  if (!draft.where) return null
  const agent = draft.agent.trim()
  const stack = draft.where !== 'harness' || draft.stackName.trim() || draft.stackYaml.trim()
  return {
    subject: { provider: draft.provider.trim(), model: draft.model.trim(), ...(agent ? { agent } : {}) },
    where: draft.where,
    ...(stack ? { stack: { name: draft.stackName.trim(), yaml: draft.stackYaml } } : {}),
    runs_per_side: num(draft.runs_per_side),
    technical_retries: num(draft.technical_retries),
    max_scenarios: num(draft.max_scenarios),
    estimated_usd_per_run: num(draft.estimated_usd_per_run),
    min_effect: { tokens: num(draft.min_effect_tokens), pass_rate_pp: num(draft.min_effect_pass_rate_pp) },
    budget: {
      max_spend_usd: num(draft.max_spend_usd),
      max_duration_ms: num(draft.max_duration_min) * 60_000,
      max_calls: num(draft.max_calls),
      max_candidates: num(draft.max_candidates),
    },
    controls: draft.controls
      .split(/[,\n]/)
      .map((control) => control.trim())
      .filter(Boolean),
    ...optional('judge', sectionValue('judge', draft.sections.judge) as JudgeConfig | undefined),
    ...optional('candidate', sectionValue('candidate', draft.sections.candidate) as CandidateConfig | undefined),
    ...optional('validation', sectionValue('validation', draft.sections.validation) as ValidationConfig | undefined),
    ...optional('promotion', sectionValue('promotion', draft.sections.promotion) as PromotionConfig | undefined),
    ...(draft.sections.promotion ? { release_build_estimate_usd: num(draft.release_build_estimate_usd) } : {}),
  }
}

/** The advanced editor's JSON, read back into a draft; throws a readable message. */
export function draftFromJson(text: string): ConfigDraft {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch (error) {
    throw new Error(`Not valid JSON: ${(error as Error).message}`)
  }
  const config = value as Partial<LoopConfig> | null
  if (!config || typeof config !== 'object' || !config.subject || !config.min_effect || !config.budget) {
    throw new Error('Expected a LoopConfig object with subject, where, min_effect and budget.')
  }
  if (config.where !== 'harness' && config.where !== 'docker' && config.where !== 'github') {
    throw new Error('where must be harness, docker or github.')
  }
  // The backend refuses unknown fields (`deny_unknown_fields`): say so here
  // instead of silently dropping a key the form cannot show.
  const unknown = Object.keys(config).filter((key) => !TOP_LEVEL.has(key))
  for (const section of SECTIONS) {
    const value = (config as Record<string, unknown>)[section.id]
    if (!value || typeof value !== 'object') continue
    const known = new Set(section.fields.map((field) => field.key.split('.')[0]))
    unknown.push(
      ...Object.keys(value)
        .filter((key) => !known.has(key))
        .map((key) => `${section.id}.${key}`),
    )
  }
  if (unknown.length) throw new Error(`Unknown LoopConfig fields: ${unknown.join(', ')}.`)
  return draftFromConfig(config as LoopConfig)
}
