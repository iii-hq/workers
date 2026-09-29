import type { BuildContext, BuildResult, Plugin } from 'esbuild'
import type { LintWorkerUiOptions } from './lint-worker-ui.mjs'

/** The six specifiers the console's import map serves; never bundled. */
export const workerUiExternals: readonly string[]

/** Exact-match externals plugin (`external` would also externalize this package's bundleable subpaths). */
export function workerUiExternalsPlugin(extra?: readonly string[]): Plugin

export interface ScopeOptions {
  /** The `data-iii-ui` value the console wraps the render in — normally the worker name. */
  scope: string
  /** Allowed `@keyframes` name prefixes. Default `[scope, `${scope}-ui`]`. */
  keyframePrefixes?: readonly string[]
  /** Selector prefixes deliberately left global (portal roots). */
  allowUnscopedSelectors?: readonly string[]
}

/** Throws, listing every offending rule, unless the sheet is fully scoped. */
export function assertScoped(css: string, options: ScopeOptions & { file?: string }): void

/** Warns about design-token references unknown to the console and the sheet itself; `strict` throws instead. */
export function checkTokens(
  files: ReadonlyArray<readonly [name: string, text: string]>,
  options?: { strict?: boolean; known?: readonly string[] },
): Array<{ name: string; uses: number }>

export interface BuildWorkerUiOptions extends ScopeOptions {
  /** Default `['page.tsx', 'styles.css']`. */
  entryPoints?: string[]
  /** Default `'dist'`. */
  outdir?: string
  /** Paths resolve against this; default `process.cwd()` (the `ui/` dir). Pass `import.meta.dirname` when invoked from elsewhere. */
  root?: string
  /** Default `process.argv.includes('--watch')`. */
  watch?: boolean
  /** Default `!watch`. */
  minify?: boolean
  /** Fail (instead of warn) on unknown design tokens. Default true. */
  strictTokens?: boolean
  /** Design-rule lint of the source after a non-watch build (see lint-worker-ui). `false` skips it. Default `{}`. */
  lint?: false | Pick<LintWorkerUiOptions, 'strict' | 'disable' | 'allow'>
  plugins?: Plugin[]
  extraExternal?: readonly string[]
  define?: Record<string, string>
}

/** Build the worker's assets (or watch them). Exits 1 when a post-build check fails. */
export function buildWorkerUi(options: BuildWorkerUiOptions): Promise<BuildResult | BuildContext>
