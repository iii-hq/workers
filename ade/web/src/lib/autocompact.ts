/**
 * Auto-compact (chat-projects phase 3, spec §3 with D158–D172), pure: the
 * setting at each level, the one resolution every surface reads, the token
 * field's parser, and the words of the meter, the chip, the marker and the
 * settings. The views only draw them; the apps port the same functions.
 *
 * - **Global** (`harness::chat-settings`): `{mode: "off" | "tokens", tokens?,
 *   instructions?}`, off by default.
 * - **Project** (`harness::projects`, the record's `autocompact`) and
 *   **conversation** (`metadata.autocompact`): `{mode: "inherit" | "tokens" |
 *   "off", tokens?}`; absent or malformed is `inherit`.
 * - **Resolution**: conversation, then project, then global; the first that
 *   is not `inherit` decides. A side chat reads its conversation's value (the
 *   harness reads the parent's metadata through `side_chat_of`, not the
 *   frozen copy). A sub-agent copies only `project_id`, so its own metadata,
 *   its project and the global decide.
 * - **Effective** (D4, D171): the resolved size when it is below the model's
 *   `usable` (the input budget at which the context-manager compacts today),
 *   else null: the model's own limit, as today.
 */
import type { Conversation } from '@/types/chat'

/*
 * No runtime imports: the entry mapper, the backend and the chat all read
 * this module, and two keys are all it needs from them.
 */
/** `metadata.project_id` of a conversation out of every project (`NO_PROJECT`). */
const NO_PROJECT = 'none'
/** `metadata.side_chat_of`: the conversation a side chat belongs to (`SIDE_CHAT_KEY`). */
const SIDE_CHAT_KEY = 'side_chat_of'

/* ── the setting ────────────────────────────────────────────────────── */

/** The smallest and largest size a setting may hold (`bad_tokens` outside). */
export const AUTOCOMPACT_MIN_TOKENS = 20_000
export const AUTOCOMPACT_MAX_TOKENS = 2_000_000
/** The slider stops at 1M; the field goes to 2M. */
export const AUTOCOMPACT_SLIDER_MAX = 1_000_000
/** The slider's step. */
export const AUTOCOMPACT_SLIDER_STEP = 10_000
/** The size a switch turned on starts at when none was ever set. */
export const AUTOCOMPACT_DEFAULT_TOKENS = 400_000
/** The project dialog's preset chips (§3.3.2). */
export const AUTOCOMPACT_PRESETS: readonly number[] = [
  100_000, 160_000, 200_000, 400_000, 800_000,
]
/** The summary instructions' limit (`bad_instructions` over it). */
export const AUTOCOMPACT_INSTRUCTIONS_MAX = 1_000

/** A project's or a conversation's setting. */
export type AutocompactSetting =
  | { mode: 'inherit' }
  | { mode: 'off' }
  | { mode: 'tokens'; tokens: number }

/** The global setting (harness configuration). */
export interface GlobalAutocompact {
  mode: 'off' | 'tokens'
  tokens?: number
  instructions?: string
}

export const INHERIT: AutocompactSetting = { mode: 'inherit' }
export const GLOBAL_OFF: GlobalAutocompact = { mode: 'off' }

function validTokens(value: unknown): value is number {
  return (
    typeof value === 'number' &&
    Number.isFinite(value) &&
    value >= AUTOCOMPACT_MIN_TOKENS &&
    value <= AUTOCOMPACT_MAX_TOKENS
  )
}

/** A project's or a conversation's `autocompact`; anything malformed is `inherit`. */
export function parseAutocompactSetting(raw: unknown): AutocompactSetting {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return INHERIT
  const record = raw as Record<string, unknown>
  if (record.mode === 'off') return { mode: 'off' }
  if (record.mode === 'tokens' && validTokens(record.tokens)) {
    return { mode: 'tokens', tokens: Math.round(record.tokens) }
  }
  return INHERIT
}

/** The global `autocompact` out of `harness::chat-settings`; malformed reads off. */
export function parseGlobalAutocompact(raw: unknown): GlobalAutocompact {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return GLOBAL_OFF
  const record = raw as Record<string, unknown>
  const tokens = validTokens(record.tokens)
    ? Math.round(record.tokens)
    : undefined
  const instructions =
    typeof record.instructions === 'string' && record.instructions.length > 0
      ? record.instructions
      : undefined
  const mode =
    record.mode === 'tokens' && tokens !== undefined ? 'tokens' : 'off'
  return {
    mode,
    ...(tokens !== undefined ? { tokens } : {}),
    ...(instructions !== undefined ? { instructions } : {}),
  }
}

/** The size a setting compacts at, or null (off, or inherit with nothing below). */
export function globalTokens(global: GlobalAutocompact | null): number | null {
  return global?.mode === 'tokens' && global.tokens !== undefined
    ? global.tokens
    : null
}

/** Two settings say the same. */
export function sameSetting(a: AutocompactSetting, b: AutocompactSetting) {
  if (a.mode !== b.mode) return false
  return a.mode !== 'tokens' || a.tokens === (b as { tokens: number }).tokens
}

/* ── the token field ────────────────────────────────────────────────── */

/** The field's one error, out of range or not a size. */
export const TOKENS_RANGE_ERROR = 'Between 20k and 2M tokens'

export type TokensParse =
  | { ok: true; tokens: number }
  | { ok: false; error: string }

/**
 * A comma read one way only, or null when it could mean two sizes:
 * thousands groups on a bare number (`400,000`, `1,200,000`), or one
 * decimal comma with one or two digits before a unit (`20,5k`, `1,25M`).
 * `20,5`, `1,234k` and `400,000k` are not sizes: never scale a guess.
 */
function withoutCommas(text: string): string | null {
  if (!text.includes(',')) return text
  if (/^\d{1,3}(?:,\d{3})+$/.test(text)) return text.replace(/,/g, '')
  const decimal = /^(\d+),(\d{1,2})([km])$/.exec(text)
  return decimal ? `${decimal[1]}.${decimal[2]}${decimal[3]}` : null
}

/**
 * The size typed in a token field: `400000`, `400,000`, `400k`, `1.2M`,
 * `1,5M` (case and spaces don't matter). Outside 20k–2M, not a size, or a
 * comma that could mean two sizes, it is "Between 20k and 2M tokens".
 */
export function parseTokens(text: string): TokensParse {
  const cleaned = withoutCommas(text.trim().toLowerCase().replace(/[\s_]/g, ''))
  if (cleaned === null) return { ok: false, error: TOKENS_RANGE_ERROR }
  const match = /^(\d+(?:\.\d+)?|\.\d+)([km]?)$/.exec(cleaned)
  if (!match) return { ok: false, error: TOKENS_RANGE_ERROR }
  const scale = match[2] === 'm' ? 1_000_000 : match[2] === 'k' ? 1_000 : 1
  const tokens = Math.round(Number(match[1]) * scale)
  if (!validTokens(tokens)) return { ok: false, error: TOKENS_RANGE_ERROR }
  return { ok: true, tokens }
}

/**
 * The size a token field holds. The field shows a stored size rounded
 * (`formatTokens`: 123456 reads `123k`), so while its text still reads as the
 * stored size, it is that exact size, never the rounded one: a save that
 * didn't touch the size keeps it. Anything else is what was typed
 * (`parseTokens`). The apps use the same rule.
 */
export function fieldTokens(
  text: string,
  stored: number | null | undefined,
): TokensParse {
  if (stored != null && validTokens(stored) && text === formatTokens(stored)) {
    return { ok: true, tokens: stored }
  }
  return parseTokens(text)
}

/**
 * A size the way every auto-compact word writes it: `160k`, `9.5k`, `1M`,
 * `1.2M`. Under 10k one decimal, then whole thousands, then millions with
 * one decimal; a trailing `.0` goes.
 */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0'
  const trim = (value: number) => {
    const fixed = value.toFixed(1)
    return fixed.endsWith('.0') ? fixed.slice(0, -2) : fixed
  }
  if (n >= 999_500) return `${trim(n / 1_000_000)}M`
  if (n >= 10_000) return `${Math.round(n / 1_000)}k`
  if (n >= 1_000) return `${trim(n / 1_000)}k`
  return String(Math.round(n))
}

/* ── resolution ─────────────────────────────────────────────────────── */

/** Which level decided (the snapshot's and the record's `rule`). */
export interface AutocompactRule {
  scope: 'global' | 'project' | 'session'
  project_id?: string
  project_name?: string
}

export interface AutocompactResolution {
  /** The resolved size (`setting_tokens`), or null for off. */
  tokens: number | null
  rule: AutocompactRule
}

/** The rule out of untrusted wire JSON (a snapshot, a record, the log). */
export function parseRule(raw: unknown): AutocompactRule | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const record = raw as Record<string, unknown>
  const scope = record.scope
  if (scope !== 'global' && scope !== 'project' && scope !== 'session') {
    return null
  }
  const rule: AutocompactRule = { scope }
  if (typeof record.project_id === 'string' && record.project_id) {
    rule.project_id = record.project_id
  }
  if (typeof record.project_name === 'string' && record.project_name) {
    rule.project_name = record.project_name
  }
  return rule
}

/** The project a conversation sits in, as the resolution needs it. */
export interface AutocompactProject {
  id: string
  name: string
  /** The record's `autocompact`, raw (read with `parseAutocompactSetting`). */
  autocompact?: unknown
}

/**
 * Conversation, then project, then global: the first that is not
 * `inherit` decides (§3.2.1, D164).
 */
export function resolveAutocompact(input: {
  session: AutocompactSetting
  project: AutocompactProject | null
  global: GlobalAutocompact | null
}): AutocompactResolution {
  const { session, project, global } = input
  if (session.mode !== 'inherit') {
    return {
      tokens: session.mode === 'tokens' ? session.tokens : null,
      rule: { scope: 'session' },
    }
  }
  if (project) {
    const own = parseAutocompactSetting(project.autocompact)
    if (own.mode !== 'inherit') {
      return {
        tokens: own.mode === 'tokens' ? own.tokens : null,
        rule: {
          scope: 'project',
          project_id: project.id,
          project_name: project.name,
        },
      }
    }
  }
  return { tokens: globalTokens(global), rule: { scope: 'global' } }
}

/** What the resolution reads of a conversation (a side chat's parent too). */
export type ResolvedConversation = Pick<
  Conversation,
  'id' | 'sessionMetadata' | 'projectId'
>

/** A conversation's own `metadata.autocompact`. */
export function autocompactOf(
  conversation: Pick<Conversation, 'sessionMetadata'>,
): AutocompactSetting {
  return parseAutocompactSetting(conversation.sessionMetadata?.autocompact)
}

/** The conversation a side chat belongs to (`metadata.side_chat_of`), or null. */
export function sideChatParentId(
  conversation: Pick<Conversation, 'sessionMetadata'>,
): string | null {
  const parentId = conversation.sessionMetadata?.[SIDE_CHAT_KEY]
  return typeof parentId === 'string' && parentId ? parentId : null
}

/**
 * The resolution for a conversation. A side chat reads its conversation's
 * metadata and project, from the conversations loaded here or from
 * `parent` (read with `session::get` when it isn't loaded). Null while
 * neither has it: the side chat's own `autocompact` is the copy frozen at
 * the fork, never what the harness reads (D164).
 */
export function conversationAutocompact(input: {
  conversation: Pick<Conversation, 'sessionMetadata' | 'projectId'>
  projects: readonly AutocompactProject[]
  global: GlobalAutocompact | null
  /** The conversations loaded here, where a side chat's parent is looked for. */
  conversations?: readonly ResolvedConversation[]
  /** A side chat's parent read from the engine, when it isn't loaded here. */
  parent?: ResolvedConversation | null
  /**
   * The global setting has been read (default true). While it hasn't, a
   * resolution that reaches the global is unknown (null), never "off".
   */
  globalRead?: boolean
  /**
   * The project list has answered (default true). While it hasn't, a
   * conversation in a project the list lacks is unknown (null).
   */
  projectsRead?: boolean
}): AutocompactResolution | null {
  let conversation: Pick<Conversation, 'sessionMetadata' | 'projectId'> =
    input.conversation
  const parentId = sideChatParentId(conversation)
  if (parentId) {
    const parent =
      input.conversations?.find((c) => c.id === parentId) ??
      (input.parent?.id === parentId ? input.parent : null)
    if (!parent) return null
    conversation = parent
  }
  const session = autocompactOf(conversation)
  const projectId = conversation.projectId
  const inProject = Boolean(projectId && projectId !== NO_PROJECT)
  const project = inProject
    ? (input.projects.find((p) => p.id === projectId) ?? null)
    : null
  if (
    session.mode === 'inherit' &&
    inProject &&
    !project &&
    input.projectsRead === false
  ) {
    return null
  }
  const resolved = resolveAutocompact({
    session,
    project,
    global: input.global,
  })
  if (resolved.rule.scope === 'global' && input.globalRead === false) {
    return null
  }
  return resolved
}

/**
 * The size the meter marks (D4): the resolved size when it is below the
 * model's `usable`, else null (the model's own limit, as today). Unknown
 * without a `usable`.
 */
export function effectiveTokens(
  tokens: number | null | undefined,
  usable: number | null | undefined,
): number | null {
  if (tokens == null || usable == null || !(usable > 0)) return null
  return tokens < usable ? tokens : null
}

/** Where the tick sits on a meter of `usable`, 0–1; null without a tick. */
export function tickRatio(
  effective: number | null | undefined,
  usable: number | null | undefined,
): number | null {
  if (effective == null || usable == null || !(usable > 0)) return null
  return Math.min(1, Math.max(0, effective / usable))
}

/** The snapshot's `autocompact` (contract C), or null from an older harness. */
export interface SnapshotAutocompact {
  setting_tokens: number | null
  effective_tokens: number | null
  cap_tokens: number | null
  rule: AutocompactRule | null
}

function tokensOrNull(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : null
}

export function parseSnapshotAutocompact(
  raw: unknown,
): SnapshotAutocompact | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const record = raw as Record<string, unknown>
  return {
    setting_tokens: tokensOrNull(record.setting_tokens),
    effective_tokens: tokensOrNull(record.effective_tokens),
    cap_tokens: tokensOrNull(record.cap_tokens),
    rule: parseRule(record.rule),
  }
}

/** What a meter marks: the effective size (null: the model's limit) and its rule. */
export interface AutocompactMark {
  effective: number | null
  rule: AutocompactRule | null
}

/** The mark a running turn froze, from its snapshot (§3.2.3). */
export function snapshotMark(snapshot: SnapshotAutocompact): AutocompactMark {
  return { effective: snapshot.effective_tokens, rule: snapshot.rule }
}

/* ── words: the meter, the chip, the notice ─────────────────────────── */

/** "global rule", "acme-web rule", "this conversation's rule". */
export function ruleName(rule: AutocompactRule): string {
  switch (rule.scope) {
    case 'session':
      return "this conversation's rule"
    case 'project':
      return `${rule.project_name ?? 'project'} rule`
    default:
      return 'global rule'
  }
}

/** The meter's label on wide surfaces: "auto 160k". */
export function meterLabel(effective: number): string {
  return `auto ${formatTokens(effective)}`
}

/** The label while a turn is past the threshold (the harness's chip only, D169). */
export const COMPACTS_AFTER_TURN = 'Compacts after this turn'

/** The context popover's line (§3.3.3). */
export function meterLine(
  effective: number | null,
  rule: AutocompactRule | null,
): string {
  if (effective == null) return AT_MODEL_LIMIT
  return `Auto-compact at ${formatTokens(effective)}${rule ? ` · ${ruleName(rule)}` : ''}`
}

export const AT_MODEL_LIMIT = "At the model's limit"

/** The header chip (§3.3.3): "Auto-compact 160k". */
export function chipText(effective: number): string {
  return `Auto-compact ${formatTokens(effective)}`
}

/**
 * The start notice's words. The harness writes the notice's text and the
 * transcript shows it verbatim; this is the same sentence for fixtures and
 * the apps' tests.
 */
export function autocompactingNotice(threshold: number): string {
  return `Auto-compacting at ${formatTokens(threshold)}…`
}

/* ── words: the transcript's marker (contract C) ────────────────────── */

export type CompactionTrigger = 'command' | 'auto' | 'inline' | 'limit'

/** What a `compaction` record says about itself; absent from older records. */
export interface CompactionFacts {
  trigger: CompactionTrigger
  thresholdTokens?: number
  rule?: AutocompactRule
  contextBefore?: number
  contextAfter?: number
}

const TRIGGERS: readonly CompactionTrigger[] = [
  'command',
  'auto',
  'inline',
  'limit',
]

/** The record's phase-3 fields; null for an older record (no `trigger`). */
export function parseCompactionFacts(raw: unknown): CompactionFacts | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const record = raw as Record<string, unknown>
  const trigger = TRIGGERS.find((t) => t === record.trigger)
  if (!trigger) return null
  const facts: CompactionFacts = { trigger }
  const threshold = tokensOrNull(record.threshold_tokens)
  if (threshold !== null) facts.thresholdTokens = threshold
  const rule = parseRule(record.rule)
  if (rule) facts.rule = rule
  const before = tokensOrNull(record.context_before)
  if (before !== null) facts.contextBefore = before
  const after = tokensOrNull(record.context_after)
  if (after !== null) facts.contextAfter = after
  return facts
}

/** The facts the marker reads its sentence from: auto, inline and limit (§3.3.5). */
export function markedCompaction(
  facts: CompactionFacts | null,
): CompactionFacts | null {
  return facts && facts.trigger !== 'command' ? facts : null
}

/** "161k → 34k tokens", or null without both numbers. */
export function contextChange(
  before: number | undefined,
  after: number | undefined,
): string | null {
  if (before === undefined || after === undefined) return null
  return `${formatTokens(before)} → ${formatTokens(after)} tokens`
}

/**
 * The marker (contract C):
 * - auto: "Auto-compacted at 160k · 161k → 34k tokens · acme-web rule";
 * - inline: "Auto-compacted during the turn at 160k · …";
 * - limit: "Compacted at the model's limit · 116k → 30k tokens".
 * A part the record lacks is left out. `command` reads as today (§3.3.5):
 * the transcript keeps its eyebrow, so `markedCompaction` drops it.
 */
export function compactionMarkerText(facts: CompactionFacts): string {
  const change = contextChange(facts.contextBefore, facts.contextAfter)
  const at =
    facts.thresholdTokens !== undefined
      ? ` at ${formatTokens(facts.thresholdTokens)}`
      : ''
  let head: string
  switch (facts.trigger) {
    case 'auto':
      head = `Auto-compacted${at}`
      break
    case 'inline':
      head = `Auto-compacted during the turn${at}`
      break
    case 'limit':
      head = "Compacted at the model's limit"
      break
    default:
      head = 'Compacted'
  }
  const rule =
    (facts.trigger === 'auto' || facts.trigger === 'inline') && facts.rule
      ? ruleName(facts.rule)
      : null
  return [head, change, rule].filter(Boolean).join(' · ')
}

/* ── words: Settings › Chat › Auto-compact (§3.3.1) ──────────────────── */

export const SETTINGS_TITLE = 'Auto-compact'
export const SETTINGS_INTRO =
  "When a conversation's context passes this size at the end of a turn, the harness compacts it, as /compact does, and tells every device. Applies to every provider on this engine."
export const SWITCH_LABEL = 'Compact conversations by themselves'
export const VALUE_LEAD = 'Compact at'
export const VALUE_TRAIL = 'tokens'
/** D171: the ticks sit at each model's `usable`, its own limit. */
export const SETTINGS_HINT =
  "Whichever comes first: this size or the model's own limit. A model's limit is its input budget, less than its window: about 116k on a 200k Claude model. The grey ticks are the limits of the models used here in the last 7 days."
export const MODELS_HEADING = 'Models used here in the last 7 days'
export const MODELS_EMPTY =
  'No model has run a turn on this engine in the last 7 days.'
export const OWN_LIMIT = 'At its own limit, as today'
export const SUMMARY_LABEL = 'What the summary keeps (optional)'
export const SUMMARY_PLACEHOLDER =
  'Keep file paths, open TODOs and the PR numbers.'
export const SUMMARY_HINT =
  'Sent with the compaction, like the text after /compact.'
export const OVERRIDES_HEADING = 'Projects and sessions'
export const OVERRIDES_HINT =
  "A project's value wins over the global; a session's over its project's."
export const COLUMN_WHERE = 'Where'
export const COLUMN_LIMIT = 'Limit'
export const COLUMN_EFFECTIVE = 'Effective'
export const HISTORY_HEADING = 'Last auto-compactions'
export const HISTORY_EMPTY = 'No auto-compactions yet.'

/**
 * The Settings › Chat row's description: what applies now. Null while the
 * settings aren't read (`undefined`): the row claims nothing it doesn't know.
 */
export function settingsRowText(
  global: GlobalAutocompact | null | undefined,
): string | null {
  if (global === undefined) return null
  const tokens = globalTokens(global)
  return tokens === null
    ? "Off: conversations compact at the model's limit, as today."
    : `At ${formatTokens(tokens)}, or at the model's limit when it comes first.`
}

/** A model row's numbers, in mono: "200k window · 116k input budget". */
export function modelNumbers(
  contextWindow: number | null,
  usable: number,
): string {
  const budget = `${formatTokens(usable)} input budget`
  return contextWindow
    ? `${formatTokens(contextWindow)} window · ${budget}`
    : budget
}

/** A model row's outcome under the global size: "At 400k" or its own limit. */
export function modelOutcome(tokens: number | null, usable: number): string {
  const effective = effectiveTokens(tokens, usable)
  return effective === null ? OWN_LIMIT : `At ${formatTokens(effective)}`
}

/** The overrides table's Limit cell. */
export function limitCell(
  setting: AutocompactSetting,
  global: GlobalAutocompact | null,
): string {
  if (setting.mode === 'tokens') return formatTokens(setting.tokens)
  if (setting.mode === 'off') return 'Off'
  const tokens = globalTokens(global)
  return tokens === null ? 'Global (off)' : `Global (${formatTokens(tokens)})`
}

/** A model in use, as the Effective cell names it. */
export interface ModelUse {
  name: string
  usable: number
}

function namesList(names: readonly string[]): string {
  const unique = [...new Set(names)]
  if (unique.length <= 1) return unique.join('')
  return `${unique.slice(0, -1).join(', ')} and ${unique.at(-1)}`
}

/** "The model's limit only": the Effective cell of off. */
export const MODEL_LIMIT_ONLY = "The model's limit only"

/**
 * The overrides table's Effective cell, per model of
 * `harness::autocompact::models` (§3.3.1, D173): "160k on Sonnet 5.5 · Opus
 * 5.5 at its own limit", "600k on Sonnet 5.5", "The model's limit only".
 * With no model known: "400k, or the model's limit".
 */
export function effectiveCell(
  tokens: number | null,
  models: readonly ModelUse[],
): string {
  if (tokens === null) return MODEL_LIMIT_ONLY
  const size = formatTokens(tokens)
  if (models.length === 0) return `${size}, or the model's limit`
  const below = models.filter((m) => tokens < m.usable).map((m) => m.name)
  const above = models.filter((m) => tokens >= m.usable).map((m) => m.name)
  const parts: string[] = []
  if (below.length > 0) parts.push(`${size} on ${namesList(below)}`)
  if (above.length > 0) {
    const many = new Set(above).size > 1
    parts.push(
      `${namesList(above)} at ${many ? 'their own limits' : 'its own limit'}`,
    )
  }
  return parts.join(' · ')
}

/** A session row's Where line: "Ledger reconciliation · session in billing-api". */
export function sessionWhere(title: string, projectName: string | null) {
  return `${title} · ${projectName ? `session in ${projectName}` : 'session, no project'}`
}

/**
 * How many sessions set their own value: "2 sessions", "12+ sessions". Null
 * when no loaded session does (older pages or not, never "0+").
 */
export function sessionsCount(count: number, more: boolean): string | null {
  if (count <= 0) return null
  const n = `${count}${more ? '+' : ''}`
  return `${n} ${count === 1 && !more ? 'session sets' : 'sessions set'} their own value`
}

/* ── words: the project dialog (§3.3.2) and Details (§3.3.4) ─────────── */

export const SECTION_LABEL = 'Auto-compact'
export const USE_GLOBAL = 'Use the global setting'
export const ONLY_MODEL_LIMIT = "Only at the model's limit"
export const COMPACT_AT_CHOICE = 'Compact at…'
export const PROJECT_HINT =
  "Whichever comes first: this size or the model's own limit. A 200k Claude model's limit is about 116k, so above that it still compacts at its own limit, as it does today."
export const DETAILS_HINT =
  "Whichever comes first: this size or the model's own limit."

/** "at 400k" or "at the model's limit", after "Use the global setting". */
export function globalNow(global: GlobalAutocompact | null): string {
  const tokens = globalTokens(global)
  return tokens === null ? "at the model's limit" : `at ${formatTokens(tokens)}`
}

/** A value in a choice's parentheses: "160k" or "model's limit". */
function valueWord(tokens: number | null): string {
  return tokens === null ? "model's limit" : formatTokens(tokens)
}

/**
 * Details' first choice (§3.3.4): "Project's (160k)", or "Global (400k)" for
 * a conversation without a project. While what it inherits isn't read (the
 * global `undefined`, the project `undefined`), it names the level with no
 * value: "Project's", "Global".
 */
export function inheritChoiceLabel(input: {
  /** Null: no project; undefined: in a project the list hasn't given yet. */
  project: AutocompactProject | null | undefined
  /** Undefined while the settings aren't read. */
  global: GlobalAutocompact | null | undefined
}): string {
  if (input.project === undefined) return "Project's"
  if (!input.project) {
    return input.global === undefined
      ? 'Global'
      : `Global (${valueWord(globalTokens(input.global))})`
  }
  const own = parseAutocompactSetting(input.project.autocompact)
  if (own.mode === 'inherit' && input.global === undefined) return "Project's"
  const resolved = resolveAutocompact({
    session: INHERIT,
    project: input.project,
    global: input.global ?? null,
  })
  return `Project's (${valueWord(resolved.tokens)})`
}

/**
 * Details' line in a side chat, which follows its conversation (§3.3.4,
 * D164): "Compacts at 160k, as its conversation does." or "Compacts at the
 * model's limit, as its conversation does."
 */
export function sideChatLine(tokens: number | null | undefined): string {
  if (tokens === undefined) return SIDE_CHAT_UNREAD
  const at = tokens === null ? "the model's limit" : formatTokens(tokens)
  return `Compacts at ${at}, as its conversation does.`
}

/** A side chat whose conversation can't be read here: no size, no guess. */
export const SIDE_CHAT_UNREAD = 'Compacts as its conversation does.'

/* ── the log ─────────────────────────────────────────────────────────── */

/** History rows: "161k → 34k". */
export function historyChange(before?: number, after?: number): string {
  if (before === undefined || after === undefined) return ''
  return `${formatTokens(before)} → ${formatTokens(after)}`
}
