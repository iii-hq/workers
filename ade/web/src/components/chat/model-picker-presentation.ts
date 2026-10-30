import {
  CATALOG_MODEL_KEY_SEP,
  type ModelId,
  type ModelOption,
} from '@/types/chat'

export function providerForModel(modelId: ModelId | undefined): string | null {
  return modelId?.split(CATALOG_MODEL_KEY_SEP)[0] || null
}

/** The family mark beside the name already says Claude: `Fable 5`. */
export function formatModelLabel(label: string): string {
  return label
    .replace(/\s+\([^)]+\)\s*$/, '')
    .replace(/^claude[\s-]+(?=\S)/i, '')
    .replace(/-/g, ' ')
    .replace(/\b[a-z]/g, (character) => character.toUpperCase())
    .replace(/\bGpt\b/g, 'GPT')
    .replace(/^GPT (?=\d)/, 'GPT-')
}

export type ModelFamily = 'claude' | 'gpt'

/**
 * The family a model belongs to, read off its id and label, so the picker
 * can show the family's mark (the Claude Code mascot, the Codex cloud)
 * whichever provider serves it. `null` falls back to the provider's mark.
 */
export function modelFamily(
  modelId: ModelId | null | undefined,
  label?: string | null,
): ModelFamily | null {
  // The model half of `provider::model`: a `claude-code` provider serving
  // a GPT model is still a GPT.
  const model = modelId?.split(CATALOG_MODEL_KEY_SEP).slice(1).join('') ?? ''
  const hay = `${model} ${label ?? ''}`.toLowerCase()
  if (/\b(claude|fable|opus|sonnet|haiku)\b/.test(hay)) return 'claude'
  if (/\b(gpt|codex|o[134](-mini|-pro)?)\b/.test(hay)) return 'gpt'
  return null
}

export function formatProviderLabel(provider: string | null): string | null {
  if (!provider) return null
  const knownProviders: Record<string, string> = {
    anthropic: 'Anthropic',
    codex: 'Codex',
    google: 'Google',
    openai: 'OpenAI',
    'openai-codex': 'Codex',
  }
  return (
    knownProviders[provider] ??
    provider
      .replace(/-/g, ' ')
      .replace(/\b[a-z]/g, (character) => character.toUpperCase())
  )
}

export function getModelPresentation(
  value: ModelId | null,
  options: readonly ModelOption[],
): { label: string; provider: string | null } {
  const selected = options.find((option) => option.id === value)
  return {
    label: selected ? formatModelLabel(selected.label) : 'No model',
    provider: formatProviderLabel(providerForModel(selected?.id)),
  }
}

/** The effort as a label: `high` → `High`. `default` is the quiet absence. */
export function formatEffortLabel(effort: string): string {
  if (!effort || effort === 'default') return ''
  return effort.charAt(0).toUpperCase() + effort.slice(1)
}

export interface ProviderPresentation {
  id: string | null
  /** The router's `display_name` when the provider is present, else the id spelled out. */
  label: string | null
  /** The mark the worker declared, when it did. */
  iconSvg?: string
}

/**
 * Who serves the selected model, for the composer's trigger: the provider
 * worker's own name (`Anthropic` vs `Claude Code (subscription)`) and its
 * mark, so two providers of the same model family tell apart at a glance.
 */
export function providerPresentationFor(
  modelId: ModelId | null | undefined,
  providers: readonly { id: string; display_name: string; icon_svg?: string }[],
): ProviderPresentation {
  const id = providerForModel(modelId ?? undefined)
  if (!id) return { id: null, label: null }
  const entry = providers.find((provider) => provider.id === id)
  return {
    id,
    label: entry?.display_name ?? formatProviderLabel(id),
    iconSvg: entry?.icon_svg,
  }
}
