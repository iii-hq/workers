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
    value: 'hybrid',
    label: 'Hybrid',
    description: 'Fuse BM25 with the configured local semantic model.',
  },
  {
    value: 'judge',
    label: 'Judge',
    description: 'Rank with the judge worker; falls back to Hybrid while it is not running.',
  },
] as const

export type FunctionSearchMode = (typeof FUNCTION_SEARCH_MODE_OPTIONS)[number]['value']

const FUNCTION_SEARCH_MODES = new Set<string>(FUNCTION_SEARCH_MODE_OPTIONS.map((option) => option.value))

export function functionSearchModeWithDefault(value: unknown): FunctionSearchMode {
  return typeof value === 'string' && FUNCTION_SEARCH_MODES.has(value) ? (value as FunctionSearchMode) : 'judge'
}

export const JUDGE_QUESTION_OPTIONS = [
  {
    value: 'noul',
    label: 'One yes/no per function',
    description: 'Ask about every shortlisted function on its own (up to 16 per capability); admitted by the minimum relevance.',
  },
  {
    value: 'choice',
    label: 'One choice per capability',
    description: 'Let the shortlisted functions compete in a single question; needed by local judges.',
  },
] as const

export type JudgeQuestion = (typeof JUDGE_QUESTION_OPTIONS)[number]['value']

export function judgeQuestionWithDefault(value: unknown): JudgeQuestion {
  return value === 'noul' ? 'noul' : 'choice'
}

/** Keys the worker no longer reads. A stored config can still carry them,
 * including the old TypeSafe key, so every save drops them. */
export function withoutRetiredKeys<T extends Record<string, unknown>>(draft: T): T {
  return Object.fromEntries(Object.entries(draft).filter(([key]) => !key.startsWith('function_search_jev_'))) as T
}

export function withFunctionSearchMode<T extends Record<string, unknown>>(
  draft: T,
  mode: FunctionSearchMode,
): T & { function_search_mode: FunctionSearchMode } {
  return { ...draft, function_search_mode: mode }
}

/** Hybrid is stranded only when the model directory is explicitly
 * `null`: an absent field falls back to the worker's default bundle path (and
 * the first-run download), so it needs no warning. */
export function semanticModeNeedsModel(mode: FunctionSearchMode, modelPath: unknown): boolean {
  return mode === 'hybrid' && modelPath === null
}
