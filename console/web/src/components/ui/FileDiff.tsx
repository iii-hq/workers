import { DEFAULT_THEMES } from '@pierre/diffs'
import type { Editor, EditorOptions } from '@pierre/diffs/edit'
import { EditProvider, MultiFileDiff } from '@pierre/diffs/react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTheme } from '@/hooks/use-theme'
import { cn } from '@/lib/utils'

type PierreEditorConstructor = new (
  options?: EditorOptions<undefined>,
) => Editor<undefined>

export type FileDiffEditState = 'loading' | 'ready' | 'error'

export function resolveFileDiffEditState(
  edit: boolean,
  editorReady: boolean,
  editLoadFailed: boolean,
): FileDiffEditState | null {
  if (!edit) return null
  if (editorReady) return 'ready'
  return editLoadFailed ? 'error' : 'loading'
}

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
  /** Intraline emphasis. `'none'` keeps only whole-line highlighting. */
  lineDiffType?: 'word-alt' | 'word' | 'char' | 'none'
  /** Ignore leading/trailing whitespace when computing changed lines. */
  ignoreWhitespace?: boolean
  /** Render the file body folded; useful for collapse-all review controls. */
  collapsed?: boolean
  /** Expand every unchanged line instead of the compact hunk view. */
  expandUnchanged?: boolean
  /** Hide Pierre's file header when the caller supplies its own review row. */
  disableFileHeader?: boolean
  /** Enable direct editing of the new-file side. The editor loads lazily. */
  edit?: boolean
  /** Receives the complete current new-file body after each edit. */
  onChange?(contents: string): void
  /** Reports whether the lazily loaded inline editor can accept input. */
  onEditStateChange?(state: FileDiffEditState): void
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
  lineDiffType = 'word-alt',
  ignoreWhitespace = false,
  collapsed = false,
  expandUnchanged = false,
  disableFileHeader = false,
  edit = false,
  onChange,
  onEditStateChange,
  className,
}: FileDiffProps) {
  const [theme] = useTheme()
  const [EditorConstructor, setEditorConstructor] =
    useState<PierreEditorConstructor | null>(null)
  const [editLoadFailed, setEditLoadFailed] = useState(false)
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange

  useEffect(() => {
    if (!edit || EditorConstructor) return

    let disposed = false
    setEditLoadFailed(false)
    void import('@pierre/diffs/edit')
      .then(({ Editor }) => {
        if (!disposed) setEditorConstructor(() => Editor)
      })
      .catch((error) => {
        if (disposed) return
        setEditLoadFailed(true)
        console.warn(
          '[console] Pierre editor failed to load; keeping the diff read-only',
          error,
        )
      })

    return () => {
      disposed = true
    }
  }, [edit, EditorConstructor])

  useEffect(() => {
    if (!edit) setEditLoadFailed(false)
  }, [edit])

  const editorOptions = useMemo<EditorOptions<undefined>>(
    () => ({
      onChange(file) {
        onChangeRef.current?.(file.contents)
      },
    }),
    [],
  )
  const createEditor = useCallback(
    (options: EditorOptions<undefined>) => {
      if (!EditorConstructor) {
        throw new Error('Pierre editor is not loaded')
      }
      return new EditorConstructor(options)
    },
    [EditorConstructor],
  )
  const editState = resolveFileDiffEditState(
    edit,
    EditorConstructor !== null,
    editLoadFailed,
  )
  useEffect(() => {
    if (editState !== null) onEditStateChange?.(editState)
  }, [editState, onEditStateChange])
  const editing = editState === 'ready'

  const diff = (
    <MultiFileDiff
      oldFile={oldFile}
      newFile={newFile}
      edit={editing}
      editorOptions={editing ? editorOptions : undefined}
      className={cn('[--diffs-font-family:var(--font-code)]', className)}
      // Read-mode diff text takes clicks and selection without input focus;
      // a bare shortcut must not fire from inside it.
      data-keybindings-standdown=""
      options={{
        diffStyle,
        overflow,
        lineDiffType,
        collapsed,
        expandUnchanged,
        disableFileHeader,
        parseDiffOptions: { ignoreWhitespace },
        theme: DEFAULT_THEMES,
        themeType: theme,
      }}
    />
  )

  if (editing) {
    return <EditProvider createEditor={createEditor}>{diff}</EditProvider>
  }

  return (
    <>
      {diff}
      {edit && (
        <span
          role={editState === 'error' ? 'alert' : 'status'}
          className="sr-only"
        >
          {editState === 'error'
            ? 'Inline editing is unavailable; showing a read-only diff.'
            : 'Loading inline editor.'}
        </span>
      )}
    </>
  )
}
