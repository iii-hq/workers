import { DEFAULT_THEMES } from '@pierre/diffs'
import { MultiFileDiff } from '@pierre/diffs/react'
import { useTheme } from '@/hooks/use-theme'

/** One side of the diff — a whole file's text, not a patch. */
export interface FileDiffSide {
  /** Display name; also infers the syntax-highlight language. */
  name: string
  contents: string
}

export interface FileDiffProps {
  /** Pass empty `contents` for a created (old) / deleted (new) file. */
  oldFile: FileDiffSide
  newFile: FileDiffSide
  diffStyle?: 'unified' | 'split'
  /** Long lines wrap by default; `'scroll'` preserves strict columns. */
  overflow?: 'scroll' | 'wrap'
  className?: string
}

/**
 * The console's one file-diff surface — `@pierre/diffs`'s `MultiFileDiff`
 * pinned to the console's diff conventions and following the active theme.
 * The diff is computed from the two full file bodies, so callers never
 * parse or ship patch text. Shared with injected worker UI through
 * `@iii-dev/console-ui` for the same reason as `CodeEditor`: the diff
 * renderer (and its highlighter) ships once, inside the console.
 */
export function FileDiff({
  oldFile,
  newFile,
  diffStyle = 'unified',
  overflow = 'wrap',
  className,
}: FileDiffProps) {
  const [theme] = useTheme()
  return (
    <MultiFileDiff
      oldFile={oldFile}
      newFile={newFile}
      className={className}
      options={{
        diffStyle,
        overflow,
        theme: DEFAULT_THEMES,
        themeType: theme,
      }}
    />
  )
}
