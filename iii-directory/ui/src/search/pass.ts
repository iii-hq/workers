/**
 * Parser + headline for the discovery hook's transcript annotation
 * (`origin.directory`, V1): outcome `hint_injected` or `skipped` with one of
 * six stable reasons, plus coarse counts. Anything malformed parses to null
 * and the console falls back to the annotation summary string.
 */

export type DiscoveryOutcome = 'hint_injected' | 'skipped'

export type DiscoveryReason =
  | 'search_unavailable'
  | 'already_searched'
  | 'narrow_surface'
  | 'already_operating'
  | 'task_guided'
  | 'hint_already_sent'

const REASONS: readonly DiscoveryReason[] = [
  'search_unavailable',
  'already_searched',
  'narrow_surface',
  'already_operating',
  'task_guided',
  'hint_already_sent',
]

export interface DiscoveryPass {
  outcome: DiscoveryOutcome
  reason?: DiscoveryReason
  allowedFunctions: number
  functionsGeneration: number
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
}

export function parseDiscoveryPass(
  version: unknown,
  data: unknown,
): DiscoveryPass | null {
  if (version !== 1) return null
  if (typeof data !== 'object' || data === null) return null
  const record = data as Record<string, unknown>
  const outcome = record.outcome
  if (outcome !== 'hint_injected' && outcome !== 'skipped') return null
  if (!isCount(record.allowed_functions)) return null
  if (!isCount(record.functions_generation)) return null
  let reason: DiscoveryReason | undefined
  if (outcome === 'skipped') {
    if (!REASONS.includes(record.reason as DiscoveryReason)) return null
    reason = record.reason as DiscoveryReason
  } else if (record.reason !== undefined && record.reason !== null) {
    return null
  }
  return {
    outcome,
    reason,
    allowedFunctions: record.allowed_functions,
    functionsGeneration: record.functions_generation,
  }
}

export interface Headline {
  text: string
  detail: string
}

export function passHeadline(pass: DiscoveryPass): Headline {
  if (pass.outcome === 'hint_injected') {
    return {
      text: 'directory injected the search hint',
      detail: 'call directory::search_functions once',
    }
  }
  const detail = (() => {
    switch (pass.reason) {
      case 'search_unavailable':
        return 'directory::search_functions not callable'
      case 'already_searched':
        return 'search result already in context'
      case 'narrow_surface':
        return 'surface too narrow for a search hint'
      case 'already_operating':
        return 'session already calling its functions'
      case 'task_guided':
        return 'task already names its functions'
      case 'hint_already_sent':
        return 'hint already sent this session'
      default:
        return 'skipped'
    }
  })()
  return { text: 'directory injected nothing', detail }
}
