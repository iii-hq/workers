import type { Plugin } from 'esbuild'

/** A worker namespace (`browser`), as the app accepts it. */
export declare const NS_PATTERN: RegExp

export interface ReadyInfo {
  sdk: string
  catalog: number
  features: string[]
  pages: { name: string; modal: boolean; fullScreen: boolean }[]
}

export interface BuildResult {
  /** Absolute path of the bundle written. */
  outfile: string
  /** Its size in UTF-8 bytes. */
  bytes: number
  /** sha256 of its UTF-8 bytes, lower-case hex (the descriptor's `bundle.hash`). */
  sha256: string
  /** Size warnings (above limits.bundleWarnBytes). */
  warnings: string[]
  /** The `ready` the bundle sent in the smoke run (null with `smoke: false`). */
  ready: ReadyInfo | null
}

export interface BuildOptions {
  /** The app's entry (TSX/TS), which calls `defineApp`. */
  entry: string
  /** Where the bundle goes, e.g. `dist/native.js`. */
  outfile: string
  /** The worker's function namespace (`browser`): calls and triggers are limited to it. */
  ns: string
  /** Default true. */
  minify?: boolean
  /** Run the bundle in the test harness before writing it (default true). */
  smoke?: boolean
  /** Keep rebuilding on changes; pairs with the worker's III_<NS>_NATIVE_UI_WATCH poller. */
  watch?: boolean
  log?: Pick<Console, 'info' | 'warn' | 'error'>
}

export declare function buildNativeUi(options: BuildOptions & { watch?: false }): Promise<BuildResult>
export declare function buildNativeUi(options: BuildOptions & { watch: true }): Promise<{ stop(): Promise<void> }>
export declare function buildNativeUi(options: BuildOptions): Promise<BuildResult | { stop(): Promise<void> }>

/** The size limits and the smoke run on a built bundle; throws when it must not ship. */
export declare function checkBundle(
  source: string,
  options?: { ns?: string; smoke?: boolean },
): Promise<{ bytes: number; warnings: string[]; ready: ReadyInfo | null }>

/** The esbuild plugin of the build (JSX through babel-preset-solid, universal). */
export declare function nativeUiPlugin(): Plugin
