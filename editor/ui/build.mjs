/**
 * Build the worker's two console assets:
 *
 *   page.tsx   → dist/page.js    (injected over `console:script`)
 *   styles.css → dist/styles.css (injected over `console:style`)
 *
 * The five shared specifiers stay EXTERNAL — they resolve at runtime through
 * the console's import map. A bundled second React copy surfaces as a cryptic
 * "Invalid hook call" with nothing pointing at the cause, and a bundled editor
 * would ship megabytes to duplicate the Monaco the console already runs.
 * `--watch` pairs with the worker's III_EDITOR_UI_WATCH poller.
 *
 * ---------------------------------------------------------------------------
 * Why this file has plugins: narrowing @pierre/diffs to fit the asset cap
 * ---------------------------------------------------------------------------
 * The page renders files and diffs with `@pierre/diffs`. Bundled as-is that
 * costs 10,808,316 bytes — over the console's 8 MiB per-asset cap
 * (`console/src/ui_assets.rs`, MAX_ASSET_BYTES). Almost none of that is
 * pierre: its own code is 350 KB. The 10 MB is shiki's *full* bundle, which
 * pierre reaches because `shiki/bundle/full` builds its `createHighlighter`
 * from three statically-referenced catalogs:
 *
 *   @shikijs/langs             7,579,720 B  (347 TextMate grammars)
 *   @shikijs/themes            1,330,336 B  (65 themes)
 *   @shikijs/engine-oniguruma    638,722 B  (base64 wasm)
 *
 * esbuild cannot tree-shake any of it: the catalogs are indexed by runtime
 * string, and with `splitting: false` every `() => import('@shikijs/langs/x')`
 * is inlined into the single output file. Code splitting is not an option
 * either — the console's loader requires every registered `console:script` to
 * export a `default` setup function, so extra chunks cannot be served.
 *
 * The plugins below narrow those three catalogs. Measured, cumulative:
 *
 *   baseline                                     10,808,316 B
 *   + slimShiki (allowlist, no shiki themes)      5,138,416 B
 *   + oniguruma/wasm stubs                        4,515,533 B
 *   + drop @pierre/theming's shiki collection     3,170,356 B
 *   + only pierre-dark / pierre-light             2,897,352 B
 *
 * Both guards at the bottom of this file exist because every plugin here
 * reaches into a dependency's internals. `assertSizeBudget` catches a shiki
 * bump that silently restores the full catalogs; `assertHighlighting` catches
 * one that silently leaves us rendering unhighlighted plain text.
 */

import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

import esbuild from 'esbuild'

/**
 * Grammars the editor bundles. Everything else raises shiki's own
 * "Language `x` not found" — loud, and caught by `assertHighlighting`.
 *
 * Chosen for what this repo actually opens: Rust workers, their TypeScript
 * injected UI, and the config and doc files around them. `cpp` is deliberately
 * absent — cpp.mjs plus cpp-macro.mjs are 626 KB between them, a fifth of the
 * whole asset, for a language no iii worker is written in. Adding one back is
 * a one-line change; re-run the build and the budget line prints what it cost.
 */
const HIGHLIGHT_LANGUAGES = [
  ['typescript', ['ts', 'cts', 'mts']],
  ['tsx', []],
  ['javascript', ['js', 'cjs', 'mjs']],
  ['jsx', []],
  ['json', []],
  ['jsonc', []],
  ['yaml', ['yml']],
  ['toml', []],
  ['rust', ['rs']],
  ['python', ['py']],
  ['go', []],
  ['sql', []],
  ['shellscript', ['bash', 'sh', 'zsh', 'shell']],
  ['css', []],
  ['html', []],
  ['markdown', ['md']],
  ['dockerfile', []],
  ['ini', ['properties']],
  ['xml', []],
  ['diff', ['patch']],
]

/** Themes the page can ask for. `DEFAULT_THEMES` is exactly these two. */
const KEPT_PIERRE_THEMES = new Set(['pierre-dark', 'pierre-light'])

/**
 * Replace the `shiki` root entry with a fine-grained equivalent.
 *
 * `shiki` resolves to `shiki/bundle/full`, whose `createHighlighter` closes
 * over `bundledLanguages`, `bundledThemes`, and an oniguruma default engine.
 * This shim keeps that module's exact export surface — @pierre/diffs imports
 * `bundledLanguages`, `createHighlighter`, `createJavaScriptRegexEngine`,
 * `createOnigurumaEngine`, `codeToHtml`, `createCssVariablesTheme`,
 * `getTokenStyleObject` and `stringifyTokenStyle` from it — while swapping the
 * catalogs for the allowlist above. It is shiki's own documented fine-grained
 * bundle pattern; pierre imports the full bundle internally, so the only place
 * a consumer can opt into the narrow one is here.
 *
 * `bundledThemes` is empty on purpose: pierre resolves theme names through
 * @pierre/theming (pierre-dark and pierre-light ship as @pierre/theme
 * modules), never through shiki's catalog.
 *
 * The shim's own imports resolve from @pierre/diffs' directory, not this
 * package's. That is what guarantees one physical `@shikijs/core` in the
 * graph: resolving from here could pick a different copy than pierre's own
 * `shiki/core` import, and two `@shikijs/core` module registries would mean
 * the highlighter this shim constructs is not the one pierre's renderers
 * expect.
 */
function slimShiki() {
  const NAMESPACE = 'pierre-slim-shiki'
  const ONIG_NAMESPACE = 'pierre-slim-shiki-onig'
  const langEntries = HIGHLIGHT_LANGUAGES.map(
    ([id, aliases]) =>
      `{id:${JSON.stringify(id)},name:${JSON.stringify(id)},aliases:${JSON.stringify(aliases)},` +
      `import:()=>import(${JSON.stringify(`@shikijs/langs/${id}`)})}`,
  ).join(',')

  const CONTENTS = `
export * from '@shikijs/core'
import { createBundledHighlighter, createSingletonShorthands, guessEmbeddedLanguages } from '@shikijs/core'
import { createJavaScriptRegexEngine, defaultJavaScriptRegexConstructor } from '@shikijs/engine-javascript'

export const bundledLanguagesInfo = [${langEntries}]
export const bundledLanguagesBase = Object.fromEntries(bundledLanguagesInfo.map((l) => [l.id, l.import]))
export const bundledLanguagesAlias = Object.fromEntries(
  bundledLanguagesInfo.flatMap((l) => l.aliases.map((a) => [a, l.import])),
)
export const bundledLanguages = { ...bundledLanguagesBase, ...bundledLanguagesAlias }

export const bundledThemesInfo = []
export const bundledThemes = {}

export const createHighlighter = createBundledHighlighter({
  langs: bundledLanguages,
  themes: bundledThemes,
  engine: () => createJavaScriptRegexEngine(),
})

export const {
  codeToHtml, codeToHast, codeToTokens, codeToTokensBase,
  codeToTokensWithThemes, getSingletonHighlighter, getLastGrammarState,
} = createSingletonShorthands(createHighlighter, { guessEmbeddedLanguages })

export { createJavaScriptRegexEngine, defaultJavaScriptRegexConstructor }
export function createOnigurumaEngine() {
  throw new Error('@pierre/diffs: the oniguruma engine is not bundled — use preferredHighlighter "shiki-js"')
}
export function loadWasm() {
  throw new Error('@pierre/diffs: the oniguruma wasm is not bundled')
}
`

  return {
    name: 'pierre-slim-shiki',
    setup(build) {
      let pierreDir = null

      // @pierre/diffs is the only bare-`shiki` importer in the graph (its
      // sibling @pierre/theming reaches for `shiki/core`, which is left
      // alone), so the first importer's directory IS pierre's. One fixed
      // path keeps a single module identity for the shim however many pierre
      // files import `shiki`, which matters because the shim owns a
      // highlighter singleton.
      build.onResolve({ filter: /^shiki$/ }, (args) => {
        pierreDir ??= args.resolveDir
        return { path: 'root', namespace: NAMESPACE }
      })

      build.onLoad({ filter: /.*/, namespace: NAMESPACE }, () => ({
        contents: CONTENTS,
        loader: 'js',
        resolveDir: pierreDir,
      }))

      // Nothing here selects the wasm engine: pierre defaults to
      // `preferredHighlighter: 'shiki-js'` and the shim's engine factory is
      // the JS one. Both specifiers are still statically reachable, so
      // esbuild bundles the 638 KB wasm unless they are stubbed. The stubs
      // throw rather than no-op so a future opt-in to 'shiki-wasm' fails
      // where it is switched on instead of silently rendering plain text.
      build.onResolve({ filter: /^shiki\/wasm$/ }, () => ({ path: 'wasm', namespace: ONIG_NAMESPACE }))
      build.onResolve({ filter: /^shiki\/engine\/oniguruma$/ }, () => ({
        path: 'engine',
        namespace: ONIG_NAMESPACE,
      }))
      build.onLoad({ filter: /.*/, namespace: ONIG_NAMESPACE }, (args) =>
        args.path === 'wasm'
          ? { contents: 'export default undefined\n', loader: 'js' }
          : {
              contents:
                'const no = () => { throw new Error("@pierre/diffs: the oniguruma engine is not bundled") }\n' +
                'export const createOnigurumaEngine = no\nexport const loadWasm = no\n' +
                'export default { createOnigurumaEngine, loadWasm }\n',
              loader: 'js',
            },
      )
    },
  }
}

/**
 * Narrow the two theme catalogs @pierre/theming registers.
 *
 * `collections/shiki.js` maps 65 theme names to `@shikijs/themes/*` dynamic
 * imports (1,330,336 B) and `shared_highlighter.js` registers every one of
 * them at module scope, so the whole set is reachable even though the page
 * only ever asks for pierre's own two. Replacing that collection with an
 * empty one drops all of it. `collections/pierre.js` keeps its ten names
 * registered, but the eight the page cannot select resolve to a throwing stub
 * instead of a theme module each.
 *
 * Both interceptors reach past @pierre/theming's `exports` map into private
 * files. `dist/collections/shiki.js` can move or be renamed on any patch
 * release, at which point the filter stops matching — silently, because the
 * build still succeeds and merely gets bigger. That is precisely what
 * `assertSizeBudget` is for; `assertHighlighting` covers the mirror case
 * where the shape changes and themes stop resolving at all.
 */
function slimPierreThemes() {
  const NAMESPACE = 'pierre-slim-themes'
  return {
    name: 'pierre-slim-themes',
    setup(build) {
      build.onLoad({ filter: /[\\/]@pierre[\\/]theming[\\/]dist[\\/]collections[\\/]shiki\.js$/ }, () => ({
        contents:
          'import { createThemeCollection } from "../modules/createThemeCollection.js"\n' +
          'export const SHIKI_COLLECTION = "shiki"\n' +
          'export const SHIKI_THEMES = []\n' +
          'export const LIGHT_SHIKI_THEMES = []\n' +
          'export const DARK_SHIKI_THEMES = []\n' +
          'export const shikiThemes = createThemeCollection({ themes: [] })\n',
        loader: 'js',
      }))

      build.onResolve({ filter: /^@pierre\/theme\/[a-z-]+$/ }, (args) => {
        const name = args.path.slice('@pierre/theme/'.length)
        if (KEPT_PIERRE_THEMES.has(name)) return null
        return { path: name, namespace: NAMESPACE }
      })

      build.onLoad({ filter: /.*/, namespace: NAMESPACE }, (args) => ({
        contents:
          `throw new Error(${JSON.stringify(
            `@pierre/theme/${args.path} is not bundled — the editor asset ships pierre-dark and pierre-light only`,
          )})\n` + 'export default undefined\n',
        loader: 'js',
      }))
    },
  }
}

/** Rebuilt per invocation: an esbuild plugin closes over per-build state. */
const plugins = () => [slimShiki(), slimPierreThemes()]

const options = {
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  external: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', '@iii-dev/console-ui'],
  plugins: plugins(),
  logLevel: 'info',
}

/**
 * The console refuses an asset over 8 MiB outright, so the point of a budget
 * here is not to stay under the cap — it is to trip when an interceptor above
 * silently stops matching. Each one was measured failing on its own, from a
 * 2,092,035-byte baseline:
 *
 *   pierre theme allowlist widened     2,364,628 B   +272,593
 *   oniguruma/wasm stubs bypassed      2,714,945 B   +622,910
 *   @pierre/theming collection moved   3,438,029 B  +1,345,994
 *   slimShiki bypassed entirely       10,580,197 B  +8,488,162
 *
 * Half the cap (4 MiB) would only catch the last of those, which makes it a
 * cap check rather than a regression check. 2.5 MiB catches the bottom three
 * and leaves 529,405 bytes — 25% — of headroom.
 *
 * Only the first is missed, and it cannot break anything: it is unreachable
 * theme data, not a rendering path. Catching it too would mean a budget within
 * 160 KB of the current size, which no page could grow inside.
 *
 * A grammar added to HIGHLIGHT_LANGUAGES can trip this. That is deliberate: a
 * 600 KB language should have to raise the number in the same commit, where
 * the cost is visible, instead of quietly spending headroom.
 *
 * Over budget is a build failure, not a warning — the alternative is a worker
 * that registers and is then rejected by the console, which surfaces as a
 * missing page rather than as a bundling problem.
 */
const SIZE_BUDGET_BYTES = 2.5 * 1024 * 1024

async function assertSizeBudget() {
  const file = path.join('dist', 'page.js')
  const bytes = (await readFile(file)).byteLength
  const mib = (bytes / 1048576).toFixed(3)
  if (bytes > SIZE_BUDGET_BYTES) {
    throw new Error(
      `${file} is ${bytes} bytes (${mib} MiB) — over the ${SIZE_BUDGET_BYTES}-byte budget.\n` +
        'Usually this means one of the shiki or theme interceptors in build.mjs stopped\n' +
        'matching: they reach into dependency internals, so a patch bump can move a path.\n' +
        'Run `node build.mjs --analyze` to see which package came back.\n' +
        'If the growth is a language you meant to add, raise SIZE_BUDGET_BYTES here.',
    )
  }
  console.log(`dist/page.js ${bytes} bytes (${mib} MiB) — within the ${SIZE_BUDGET_BYTES}-byte budget`)
}

/**
 * Prove the narrowed bundle still highlights.
 *
 * The size budget catches the catalogs coming back; this catches them going
 * away. Every failure mode of the interceptors above is silent in the browser
 * — pierre catches a rejected grammar load and renders the file as plain
 * text, which reads as a styling bug rather than a build one. So the build
 * loads the real bundled highlighter path and asserts a TypeScript snippet
 * comes back as coloured spans under a pierre theme.
 *
 * Built separately for node: the page bundle is a browser ESM asset with React
 * external, which cannot be imported here. It goes to a temp file rather than
 * a `data:` URL because a throw inside the probe puts the module's URL in the
 * stack trace, and a 2 MB base64 blob in the build log buries the one line
 * that says what broke.
 */
async function assertHighlighting() {
  const probe = await esbuild.build({
    stdin: {
      contents:
        "import { getSharedHighlighter, DEFAULT_THEMES } from '@pierre/diffs'\n" +
        'const highlighter = await getSharedHighlighter({\n' +
        '  themes: [DEFAULT_THEMES.dark, DEFAULT_THEMES.light],\n' +
        "  langs: ['typescript'],\n" +
        '})\n' +
        "export const html = highlighter.codeToHtml('export const n: number = 1\\n', {\n" +
        "  lang: 'typescript',\n" +
        '  theme: DEFAULT_THEMES.dark,\n' +
        '})\n' +
        'export const themes = highlighter.getLoadedThemes()\n',
      resolveDir: process.cwd(),
      loader: 'ts',
    },
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
    plugins: plugins(),
  })

  const dir = await mkdtemp(path.join(os.tmpdir(), 'editor-ui-smoke-'))
  try {
    const file = path.join(dir, 'probe.mjs')
    await writeFile(file, probe.outputFiles[0].text)

    let html
    let themes
    try {
      ;({ html, themes } = await import(pathToFileURL(file).href))
    } catch (cause) {
      // A grammar or theme the interceptors dropped fails here, inside pierre,
      // with a message that already names it. Keep that message and say where
      // it came from, so it does not read as an unrelated crash.
      throw new Error(`highlighting smoke test failed: ${cause instanceof Error ? cause.message : cause}`)
    }

    // A theme that failed to resolve and a grammar that never loaded both end
    // the same way: one span, no inline colour.
    const coloured = /<span style="color:#[0-9a-fA-F]{3,8}/.test(html)
    if (!coloured || themes.length !== 2) {
      throw new Error(
        'highlighting smoke test failed: expected coloured spans under two pierre themes, got ' +
          `${themes.length} theme(s) and:\n${html.slice(0, 400)}`,
      )
    }
    console.log(`highlighting ok — ${themes.join(' + ')}, ${html.length} bytes of coloured markup`)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
}

/** `--analyze` prints per-package output bytes, for when the budget trips. */
async function analyze() {
  const result = await esbuild.build({
    ...options,
    plugins: plugins(),
    logLevel: 'silent',
    metafile: true,
    write: false,
  })
  const output = Object.values(result.metafile.outputs).find((o) => o.entryPoint === 'page.tsx')
  const totals = new Map()
  for (const [file, info] of Object.entries(output.inputs)) {
    const match =
      /node_modules\/\.pnpm\/[^/]+\/node_modules\/((?:@[^/]+\/)?[^/]+)\//.exec(file) ??
      /node_modules\/((?:@[^/]+\/)?[^/]+)\//.exec(file)
    const key = match?.[1] ?? '(this package)'
    totals.set(key, (totals.get(key) ?? 0) + info.bytesInOutput)
  }
  for (const [pkg, bytes] of [...totals].sort((a, b) => b[1] - a[1])) {
    console.log(`${String(bytes).padStart(9)}  ${pkg}`)
  }
}

if (process.argv.includes('--watch')) {
  const ctx = await esbuild.context(options)
  await ctx.watch()
} else if (process.argv.includes('--analyze')) {
  await analyze()
} else {
  await esbuild.build(options)
  await assertSizeBudget()
  await assertHighlighting()
}
