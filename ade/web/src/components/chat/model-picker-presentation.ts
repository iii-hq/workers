import {
  CATALOG_MODEL_KEY_SEP,
  type ModelId,
  type ModelOption,
} from '@/types/chat'

export function providerForModel(modelId: ModelId | undefined): string | null {
  return modelId?.split(CATALOG_MODEL_KEY_SEP)[0] || null
}

export function formatModelLabel(label: string): string {
  return label
    .replace(/\s+\([^)]+\)\s*$/, '')
    .replace(/-/g, ' ')
    .replace(/\b[a-z]/g, (character) => character.toUpperCase())
    .replace(/\bGpt\b/g, 'GPT')
    .replace(/^GPT (?=\d)/, 'GPT-')
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
