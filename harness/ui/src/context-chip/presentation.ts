import type {
  ContextSnapshot,
  SnapshotUsage,
} from '../lib/metrics'

export interface PromptUsagePresentation {
  fresh: number
  cached?: number
  cacheCreation?: number
  hitPct?: number
  total: number
}

export interface OutputUsagePresentation {
  output: number
  /** A subset of output, not an additional token charge. */
  reasoning?: number
}

function hasProviderPromptUsage(usage: SnapshotUsage | undefined) {
  return (
    usage?.input != null ||
    usage?.cache_read != null ||
    usage?.cache_write != null
  )
}

export function promptUsagePresentation(
  usage: SnapshotUsage | undefined,
): PromptUsagePresentation | null {
  if (!hasProviderPromptUsage(usage)) return null

  const fresh = usage?.input ?? 0
  const cached = usage?.cache_read
  const cacheCreation = usage?.cache_write
  const total = fresh + (cached ?? 0) + (cacheCreation ?? 0)

  return {
    fresh,
    cached,
    cacheCreation,
    hitPct:
      cached != null && total > 0
        ? Math.round((cached / total) * 100)
        : undefined,
    total,
  }
}

function breakdownSource(estimator: string | undefined) {
  if (!estimator || estimator === 'heuristic') return 'chars/4'
  if (estimator === 'provider') return 'provider tokenizer'
  if (estimator === 'tokenizer' || estimator === 'tiktoken') {
    return 'local tokenizer'
  }
  return estimator
}

export function accountingProvenance(snapshot: ContextSnapshot) {
  const prompt = promptUsagePresentation(snapshot.usage)

  return `prompt total: ${
    prompt != null ? 'provider usage' : 'estimated'
  } · breakdown: ${breakdownSource(snapshot.estimator)}`
}

export function outputUsagePresentation(
  usage: SnapshotUsage | undefined,
): OutputUsagePresentation | null {
  if (usage?.output == null) return null
  return {
    output: usage.output,
    reasoning:
      usage.reasoning != null && usage.reasoning > 0
        ? usage.reasoning
        : undefined,
  }
}
