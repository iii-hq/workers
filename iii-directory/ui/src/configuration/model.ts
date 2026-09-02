export function booleanWithDefault(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback
}

export const FUNCTION_SEARCH_MODE_OPTIONS = [
  {
    value: 'lexical',
    label: 'Lexical',
    description: 'Use BM25 ranking only.',
  },
  {
    value: 'shadow',
    label: 'Shadow',
    description: 'Compute semantic rankings without changing returned results.',
  },
  {
    value: 'hybrid',
    label: 'Hybrid',
    description: 'Fuse BM25 with the configured local semantic model.',
  },
] as const

export type FunctionSearchMode = (typeof FUNCTION_SEARCH_MODE_OPTIONS)[number]['value']

const FUNCTION_SEARCH_MODES = new Set<string>(FUNCTION_SEARCH_MODE_OPTIONS.map((option) => option.value))

export function functionSearchModeWithDefault(value: unknown): FunctionSearchMode {
  return typeof value === 'string' && FUNCTION_SEARCH_MODES.has(value) ? (value as FunctionSearchMode) : 'lexical'
}

export function withFunctionSearchMode<T extends Record<string, unknown>>(
  draft: T,
  mode: FunctionSearchMode,
): T & { function_search_mode: FunctionSearchMode } {
  return { ...draft, function_search_mode: mode }
}

export function semanticModeNeedsModel(mode: FunctionSearchMode, modelPath: unknown): boolean {
  return mode !== 'lexical' && (typeof modelPath !== 'string' || modelPath.trim() === '')
}
