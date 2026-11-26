/**
 * Auto-compact on the context chip (chat-projects phase 3, §3.3.3, D170):
 * the snapshot's `autocompact` read tolerantly, where the amber tick sits
 * on the ring and the bar, and the words. The words are the ADE's own
 * (`ade/web/src/lib/autocompact.ts`), sizes included ("160k", "1.2M"), so
 * the console's meter and this chip read the same on either harness.
 */

/** Which level decided: the conversation's, its project's, or the global. */
export interface AutocompactRule {
  scope: 'global' | 'project' | 'session'
  project_id?: string
  project_name?: string
}

/**
 * The snapshot's `autocompact` (contract C): the resolved size, the size
 * the harness compacts at on this model (null at the model's own limit),
 * the model's input budget, and the level that decided.
 */
export interface SnapshotAutocompact {
  setting_tokens: number | null
  effective_tokens: number | null
  cap_tokens: number | null
  rule: AutocompactRule | null
}

function tokensOrNull(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : null
}

function parseRule(raw: unknown): AutocompactRule | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const record = raw as Record<string, unknown>
  const scope = record.scope
  if (scope !== 'global' && scope !== 'project' && scope !== 'session') return null
  const rule: AutocompactRule = { scope }
  if (typeof record.project_id === 'string' && record.project_id) rule.project_id = record.project_id
  if (typeof record.project_name === 'string' && record.project_name) rule.project_name = record.project_name
  return rule
}

/** The snapshot's `autocompact`, or null: an older harness writes none. */
export function parseAutocompact(raw: unknown): SnapshotAutocompact | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const record = raw as Record<string, unknown>
  return {
    setting_tokens: tokensOrNull(record.setting_tokens),
    effective_tokens: tokensOrNull(record.effective_tokens),
    cap_tokens: tokensOrNull(record.cap_tokens),
    rule: parseRule(record.rule),
  }
}

/** What the chip reads off a snapshot: the size it marks, and why. */
export interface AutocompactMark {
  /** The size the meter marks, below `usable`; null for no tick and no label. */
  effective: number | null
  rule: AutocompactRule | null
}

/**
 * The mark for a snapshot. The harness writes `effective_tokens` null at
 * the model's limit; a size at or past `usable` is the same thing here, so
 * no tick ever sits at the bar's end.
 */
export function autocompactMark(snapshot: { usable: number; autocompact?: unknown }): AutocompactMark | null {
  const parsed = parseAutocompact(snapshot.autocompact)
  if (!parsed) return null
  const effective =
    parsed.effective_tokens !== null && snapshot.usable > 0 && parsed.effective_tokens < snapshot.usable
      ? parsed.effective_tokens
      : null
  return { effective, rule: parsed.rule }
}

/** Where the tick sits on a meter of `usable`, 0–1; null without a tick. */
export function tickRatio(effective: number | null | undefined, usable: number): number | null {
  if (effective == null || !(usable > 0)) return null
  return Math.min(1, Math.max(0, effective / usable))
}

/** A size as the ADE writes it: "9.5k", "160k", "1M", "1.2M". */
export function sizeWord(n: number): string {
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

/** The label after the counts: "auto 160k". */
export function meterLabel(effective: number): string {
  return `auto ${sizeWord(effective)}`
}

/** The label while a running turn is past the size (this chip only, D169). */
export const COMPACTS_AFTER_TURN = 'Compacts after this turn'

export const AT_MODEL_LIMIT = "At the model's limit"

/** The breakdown's line: "Auto-compact at 160k · acme-web rule". */
export function meterLine(effective: number | null, rule: AutocompactRule | null): string {
  if (effective == null) return AT_MODEL_LIMIT
  return `Auto-compact at ${sizeWord(effective)}${rule ? ` · ${ruleName(rule)}` : ''}`
}

/**
 * The turn will compact when it ends: it is running and the context is at
 * or past the size. `running` is the console's own signal (`isStreaming`),
 * absent on an older console, where this is never true.
 */
export function compactsAfterTurn(total: number, effective: number | null, running: boolean | undefined): boolean {
  return running === true && effective !== null && total >= effective
}

/** The label after the counts, or null for none. */
export function chipLabel(total: number, mark: AutocompactMark | null, running: boolean | undefined): string | null {
  if (!mark || mark.effective === null) return null
  return compactsAfterTurn(total, mark.effective, running) ? COMPACTS_AFTER_TURN : meterLabel(mark.effective)
}
