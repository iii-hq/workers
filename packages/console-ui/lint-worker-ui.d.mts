export type LintLevel = 'error' | 'warning'

export interface Finding {
  rule: string
  /** Relative to the worker's `ui/` dir. */
  file: string
  line: number
  excerpt: string
  hint: string
}

export interface LintResult {
  errors: Finding[]
  warnings: Finding[]
}

export interface LintWorkerUiOptions {
  /** The worker's `ui/` dir. Default `process.cwd()`. */
  root?: string
  /** The `data-iii-ui` value; default: first one in `styles.css`, else the worker dir name. */
  scope?: string
  /** Promote every warning to an error. Default false. */
  strict?: boolean
  /** Rule ids to skip. */
  disable?: readonly string[]
  /** Per rule, excerpt substrings or regexes to ignore. */
  allow?: Readonly<Record<string, ReadonlyArray<string | RegExp>>>
}

/** rule id → [default level, hint]. */
export const rules: Readonly<Record<string, readonly [level: LintLevel, hint: string]>>

export function inferScope(root: string): string

/** Lint a worker UI's source (`styles.css`, `page.tsx`, `src/**`; never `dist/`). */
export function lintWorkerUi(options?: LintWorkerUiOptions): LintResult

/** Findings grouped by rule (`max` examples each, default 5) plus a summary line, `[worker-ui]`-prefixed. */
export function formatLint(result: LintResult, options?: { strict?: boolean; max?: number }): string
