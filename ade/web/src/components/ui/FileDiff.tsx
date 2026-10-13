import type { FileDiffOptions, PostRenderPhase } from '@pierre/diffs'
import type { Editor, EditorOptions } from '@pierre/diffs/edit'
import { EditProvider, MultiFileDiff } from '@pierre/diffs/react'
import {
  type RefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { useTheme } from '@/hooks/use-theme'
import { FILE_DIFF_THEME } from '@/lib/file-diff-theme'
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
  /** Existing review region to focus when an activated context action disappears. */
  contextExpansionFocusFallback?: RefObject<HTMLElement | null>
  className?: string
}

/**
 * Pierre treats two diffs with the same `cacheKey` as one render and defaults
 * the key to the file name, so a read-only diff whose text changes would keep
 * showing its first render. The editor relies on that stable default instead.
 */
function keyedByText(file: FileDiffSide): FileDiffSide & { cacheKey: string } {
  return { ...file, cacheKey: `${file.name}\0${file.contents}` }
}

// Pierre 1.3 renders context expanders as unnamed divs. Its supported render
// hook keeps one native button per hunk action after highlighting or updates.
// The top-painted gutter stays semantic; copies retain their pointer attributes.
function contextExpanderKey(control: HTMLElement): string | HTMLElement {
  const hunk = control
    .closest('[data-expand-index]')
    ?.getAttribute('data-expand-index')
  return hunk == null
    ? control
    : JSON.stringify([
        hunk,
        control.hasAttribute('data-expand-up')
          ? 'up'
          : control.hasAttribute('data-expand-down')
            ? 'down'
            : 'both',
        control.hasAttribute('data-expand-all-button'),
      ])
}

type ContextExpansionFocus = {
  pending: { control: HTMLElement; key: string | HTMLElement } | null
  captureClick(event: Event): void
}
const contextExpansionFocus = new WeakMap<HTMLElement, ContextExpansionFocus>()

function makeContextExpandersAccessible(
  container: HTMLElement,
  phase: PostRenderPhase,
  fallback?: RefObject<HTMLElement | null>,
) {
  const shadow = container.shadowRoot
  if (!shadow) return
  let focus = contextExpansionFocus.get(container)
  if (phase === 'unmount') {
    if (focus) shadow.removeEventListener('click', focus.captureClick, true)
    contextExpansionFocus.delete(container)
    return
  }
  if (!focus) {
    const state: ContextExpansionFocus = {
      pending: null,
      captureClick(event) {
        const control =
          event.target instanceof Element
            ? event.target.closest<HTMLElement>('button[data-expand-button]')
            : null
        state.pending =
          control && shadow.activeElement === control
            ? { control, key: contextExpanderKey(control) }
            : null
      },
    }
    shadow.addEventListener('click', state.captureClick, true)
    contextExpansionFocus.set(container, state)
    focus = state
  }
  const pending = focus.pending
  focus.pending = null
  const groups = new Map<string | HTMLElement, HTMLElement[]>()
  for (const control of shadow.querySelectorAll<HTMLElement>(
    '[data-expand-button]',
  )) {
    const key = contextExpanderKey(control)
    const group = groups.get(key) ?? []
    group.push(control)
    groups.set(key, group)
  }
  let equivalent: HTMLElement | undefined
  for (const [key, controls] of groups) {
    const visible = controls.filter(
      (control) => control.getClientRects().length,
    )
    const primary =
      visible.find((control) => control.closest('[data-gutter]')) ??
      visible[0] ??
      controls.find((control) => control.closest('[data-gutter]')) ??
      controls[0]
    for (const control of controls) {
      const tag = control === primary ? 'button' : 'div'
      const target =
        control.tagName.toLowerCase() === tag
          ? control
          : container.ownerDocument.createElement(tag)
      if (target !== control) {
        for (const { name, value } of control.attributes) {
          target.setAttribute(name, value)
        }
        target.append(...control.childNodes)
        control.replaceWith(target)
      }
      target.removeAttribute('role')
      target.removeAttribute('tabindex')
      if (control !== primary) {
        target.removeAttribute('type')
        target.removeAttribute('aria-label')
        target.setAttribute('aria-hidden', 'true')
        continue
      }
      target.removeAttribute('aria-hidden')
      target.setAttribute('type', 'button')
      if (pending?.key === key) equivalent = target
      target.setAttribute(
        'aria-label',
        target.hasAttribute('data-expand-all-button')
          ? 'Expand all unchanged lines'
          : target.hasAttribute('data-expand-up')
            ? 'Expand unchanged lines above'
            : target.hasAttribute('data-expand-down')
              ? 'Expand unchanged lines below'
              : 'Expand unchanged lines above and below',
      )
    }
  }
  const active = container.ownerDocument.activeElement
  if (
    !pending ||
    pending.control.isConnected ||
    shadow.activeElement ||
    (active && active !== container.ownerDocument.body && active !== container)
  )
    return
  const target = equivalent ?? fallback?.current
  if (target?.isConnected) target.focus({ preventScroll: true })
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
  contextExpansionFocusFallback,
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

  // MultiFileDiff re-runs the whole diff whenever either file object or
  // `options.parseDiffOptions` is a new object (its parse is memoized on
  // their identity), and callers pass inline `{ name, contents }`: keyed
  // on their values, an unrelated render or a style or theme switch
  // reuses the diff it already has. Read-only sides also carry their text
  // as Pierre's cache key (see keyedByText).
  const stableOldFile = useMemo(
    () =>
      editing
        ? { name: oldFile.name, contents: oldFile.contents }
        : keyedByText({ name: oldFile.name, contents: oldFile.contents }),
    [oldFile.name, oldFile.contents, editing],
  )
  const stableNewFile = useMemo(
    () =>
      editing
        ? { name: newFile.name, contents: newFile.contents }
        : keyedByText({ name: newFile.name, contents: newFile.contents }),
    [newFile.name, newFile.contents, editing],
  )
  const parseDiffOptions = useMemo(
    () => ({ ignoreWhitespace }),
    [ignoreWhitespace],
  )
  const options = useMemo<FileDiffOptions<undefined>>(
    () => ({
      diffStyle,
      overflow,
      lineDiffType,
      collapsed,
      expandUnchanged,
      disableFileHeader,
      parseDiffOptions,
      theme: FILE_DIFF_THEME,
      themeType: theme,
      onPostRender(container, _instance, phase) {
        makeContextExpandersAccessible(
          container,
          phase,
          contextExpansionFocusFallback,
        )
      },
      unsafeCSS: `
        /* Keep semantic fills separate from Shiki's ANSI placeholders. */
        :host {
          --diffs-addition-color-override: var(--color-ok);
          --diffs-deletion-color-override: var(--color-alert-strong);
          --diffs-modified-color-override: var(--color-accent);
        }
        [data-gutter] [data-line-number-content] {
          color: var(--diffs-fg-number);
        }
        button[data-expand-button] {
          appearance: none;
          border-block: 0;
          border-left: 0;
          padding: 0;
        }
        button[data-expand-button]:focus-visible {
          outline: 2px solid var(--color-rule-focus);
          outline-offset: -2px;
        }
        [data-separator-wrapper]:has([data-expand-all-button]) {
          display: flex;
        }
        [data-expand-button][data-expand-all-button] {
          display: flex;
          min-width: max-content;
          padding-inline: 12px;
        }
        @media (max-width: 639px) {
          [data-separator="line-info"], [data-separator="line-info-basic"] {
            height: 48px;
          }
          [data-expand-index] [data-separator-wrapper] {
            display: flex;
          }
          [data-expand-button] {
            box-sizing: border-box;
            min-width: 48px;
            min-height: 48px;
          }
        }
      `,
    }),
    [
      diffStyle,
      overflow,
      lineDiffType,
      collapsed,
      expandUnchanged,
      disableFileHeader,
      parseDiffOptions,
      theme,
      contextExpansionFocusFallback,
    ],
  )

  const diff = (
    <MultiFileDiff
      oldFile={stableOldFile}
      newFile={stableNewFile}
      edit={editing}
      editorOptions={editing ? editorOptions : undefined}
      className={cn('[--diffs-font-family:var(--font-code)]', className)}
      // Read-mode diff text takes clicks and selection without input focus;
      // a bare shortcut must not fire from inside it.
      data-keybindings-standdown=""
      options={options}
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
