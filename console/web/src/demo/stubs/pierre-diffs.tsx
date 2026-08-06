/**
 * Build stub for `@pierre/diffs` (and `/react`), aliased in only by
 * `vite.demo.config.ts`.
 *
 * The package pulls all of shiki's grammars, which lands ~13MB of language
 * chunks in the output directory. The demo's sub-agents do write files, so
 * the coder card is reachable — it just renders the body unhighlighted here.
 * Everything around it (batch chips, paths, byte counts) is the real view.
 */

export const DEFAULT_THEMES = { light: 'github-light', dark: 'github-dark' }

export interface FileContents {
  name: string
  contents: string
}

/** The body, monospaced and scrollable, with no tokenizer behind it. */
function PlainFile({ contents }: { contents: string }) {
  return (
    <pre className="max-h-[280px] overflow-auto whitespace-pre px-3 py-2 font-mono text-[12px] leading-[1.55] text-ink">
      {contents}
    </pre>
  )
}

export function File({ file }: { file: FileContents }) {
  return <PlainFile contents={file.contents} />
}

/** New-file diffs are all the demo produces: show the new side. */
export function MultiFileDiff({ newFile }: { newFile: FileContents }) {
  return <PlainFile contents={newFile.contents} />
}

export default { DEFAULT_THEMES, File, MultiFileDiff }
