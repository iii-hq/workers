/**
 * Build stub for `@pierre/diffs` (and `/react`), aliased in only by
 * `vite.demo.config.ts`.
 *
 * The package pulls all of shiki's grammars, which lands ~13MB of language
 * chunks in the output directory. The landing demo's scenario dispatches no
 * `coder::*` calls, so the diff views are unreachable — but the renderer
 * registry imports the family statically, so the specifier still has to
 * resolve. This keeps the graph honest and degrades to a plain notice if a
 * future scenario ever does render one.
 */

export const DEFAULT_THEMES = { light: 'github-light', dark: 'github-dark' }

export interface FileContents {
  name: string
  contents: string
}

function DiffUnavailable() {
  return (
    <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
      · diff rendering is not bundled in this demo
    </div>
  )
}

export const File = DiffUnavailable
export const MultiFileDiff = DiffUnavailable

export default { DEFAULT_THEMES, File, MultiFileDiff }
