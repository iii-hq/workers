import type * as monacoNs from 'monaco-editor'
import * as React from 'react'
import { cn } from '@/lib/utils'

export interface CodeEditorHandle {
  focus(): void
}

export interface CodeEditorProps {
  value: string
  onChange: (next: string) => void
  /** Monaco language id (`'markdown'`, `'json'`, `'yaml'`, …). Unknown
      ids degrade to plain text with the same chrome. */
  language: string
  /** Class for the outer wrapper (borders, min-height, width). */
  className?: string
  placeholder?: string
  /** Read-only: content stays selectable/copyable, chrome unchanged. */
  readOnly?: boolean
  /** Inert and dimmed (implies read-only). */
  disabled?: boolean
  autoFocus?: boolean
  id?: string
  'aria-label'?: string
  /** Observes keys bubbling out of the editor (shortcuts like ⌘S) — keys
      Monaco consumes for editing never reach it. */
  onKeyDown?: React.KeyboardEventHandler<HTMLDivElement>
}

/* The pre-Monaco fallback (and permanent degraded mode if the editor chunk
   ever fails to load) renders the same typography the editor is configured
   with, so the swap-in doesn't reflow the text. */
const EDITOR_TYPOGRAPHY =
  'm-0 whitespace-pre-wrap break-words px-3 py-2 text-left font-mono text-[12.5px] leading-[19px]'

const MONACO_OPTIONS: monacoNs.editor.IStandaloneEditorConstructionOptions = {
  automaticLayout: true,
  wordWrap: 'on',
  minimap: { enabled: false },
  lineNumbers: 'off',
  folding: false,
  glyphMargin: false,
  lineDecorationsWidth: 12,
  lineNumbersMinChars: 0,
  renderLineHighlight: 'none',
  scrollBeyondLastLine: false,
  scrollbar: {
    // The wrapper grows with content (see fitHeight) — the OUTER pane owns
    // vertical scrolling, exactly like the old textarea editor.
    vertical: 'hidden',
    horizontal: 'hidden',
    alwaysConsumeMouseWheel: false,
    useShadows: false,
  },
  overviewRulerLanes: 0,
  hideCursorInOverviewRuler: true,
  guides: { indentation: false },
  occurrencesHighlight: 'off',
  selectionHighlight: false,
  fontSize: 12.5,
  lineHeight: 19,
  padding: { top: 8, bottom: 8 },
  // Suggest/hover widgets escape ancestor overflow clipping.
  fixedOverflowWidgets: true,
  // Prose-friendly: no word-based popups while typing; language services
  // (JSON schema completions, …) still fire on trigger characters / ⌃Space.
  quickSuggestions: false,
  wordBasedSuggestions: 'off',
  contextmenu: false,
}

/**
 * The console's code editor — Monaco, themed by the design tokens (the
 * `iii-console` theme in `lib/monaco.ts` follows `html[data-theme]`), shared
 * with injected worker UI through `@iii-dev/console-ui`. Every code/text
 * editing surface goes through this component; nothing else in the console
 * (or in a worker asset) may instantiate its own editor.
 *
 * The wrapper grows with content and does NOT scroll itself — put it inside
 * an `overflow-auto` pane. Monaco loads lazily in its own chunk; until it
 * arrives (or if it never does) a plain textarea with identical typography
 * keeps the surface editable.
 */
export const CodeEditor = React.forwardRef<CodeEditorHandle, CodeEditorProps>(
  (
    {
      value,
      onChange,
      language,
      className,
      placeholder,
      readOnly,
      disabled,
      autoFocus,
      id,
      'aria-label': ariaLabel,
      onKeyDown,
    },
    ref,
  ) => {
    const hostRef = React.useRef<HTMLDivElement>(null)
    const fallbackRef = React.useRef<HTMLTextAreaElement>(null)
    const editorRef = React.useRef<monacoNs.editor.IStandaloneCodeEditor>(null)
    const applyingRef = React.useRef(false)
    const [ready, setReady] = React.useState(false)

    // The mount effect runs once; it reads mount-time props through here.
    const latest = React.useRef({ value, language, autoFocus })
    latest.current = { value, language, autoFocus }
    const onChangeRef = React.useRef(onChange)
    onChangeRef.current = onChange

    React.useImperativeHandle(ref, () => ({
      focus: () => {
        if (editorRef.current) editorRef.current.focus()
        else fallbackRef.current?.focus()
      },
    }))

    React.useEffect(() => {
      let disposed = false
      void import('@/lib/monaco')
        .then(({ monaco, CONSOLE_THEME, monoFontFamily }) => {
          if (disposed || !hostRef.current) return
          const editor = monaco.editor.create(hostRef.current, {
            ...MONACO_OPTIONS,
            value: latest.current.value,
            language: latest.current.language,
            theme: CONSOLE_THEME,
            fontFamily: monoFontFamily(),
          })
          editorRef.current = editor
          editor.getModel()?.updateOptions({ tabSize: 2, insertSpaces: true })

          const fitHeight = () => {
            if (hostRef.current)
              hostRef.current.style.height = `${editor.getContentHeight()}px`
          }
          editor.onDidChangeModelContent(() => {
            if (applyingRef.current) return
            // Read through the ref-captured editor: the latest onChange is
            // re-bound below on every render via this stable dispatcher.
            onChangeRef.current(editor.getValue())
          })
          editor.onDidContentSizeChange(fitHeight)
          fitHeight()
          if (latest.current.autoFocus) editor.focus()
          setReady(true)
        })
        .catch((err) => {
          console.warn(
            '[console] monaco failed to load — staying on the plain fallback editor',
            err,
          )
        })
      return () => {
        disposed = true
        editorRef.current?.dispose()
        editorRef.current = null
      }
    }, [])

    // Prop → editor sync (external value swaps, language, options).
    React.useEffect(() => {
      const editor = editorRef.current
      if (!ready || !editor) return
      if (editor.getValue() !== value) {
        applyingRef.current = true
        editor.setValue(value)
        applyingRef.current = false
      }
    }, [ready, value])

    React.useEffect(() => {
      const editor = editorRef.current
      const model = editor?.getModel()
      if (!ready || !editor || !model) return
      void import('@/lib/monaco').then(({ monaco }) => {
        if (editorRef.current === editor && editor.getModel() === model)
          monaco.editor.setModelLanguage(model, language)
      })
    }, [ready, language])

    React.useEffect(() => {
      if (!ready) return
      editorRef.current?.updateOptions({
        readOnly: !!(readOnly || disabled),
        domReadOnly: !!(readOnly || disabled),
        placeholder,
        ariaLabel,
      })
    }, [ready, readOnly, disabled, placeholder, ariaLabel])

    return (
      // biome-ignore lint/a11y/noStaticElementInteractions: shortcut relay + gap-click focus around a real editor
      <div
        className={cn(
          'relative bg-bg',
          disabled && 'pointer-events-none opacity-40',
          className,
        )}
        onKeyDown={onKeyDown}
        onMouseDown={(e) => {
          // A caller-set min-height can leave dead space under the last
          // line; clicking it lands the caret at the end, like a textarea.
          if (e.target !== e.currentTarget || !editorRef.current) return
          e.preventDefault()
          const editor = editorRef.current
          const model = editor.getModel()
          if (model) {
            const line = model.getLineCount()
            editor.setPosition({
              lineNumber: line,
              column: model.getLineMaxColumn(line),
            })
          }
          editor.focus()
        }}
      >
        <div
          ref={hostRef}
          className={
            // Until the swap, the host hides under the in-flow fallback
            // textarea (which is what sizes the wrapper) — `invisible`
            // keeps its box measurable for Monaco's initial layout.
            ready ? undefined : 'invisible absolute inset-0 overflow-hidden'
          }
        />
        {!ready ? (
          <textarea
            ref={fallbackRef}
            id={id}
            value={value}
            onChange={(e) => onChange(e.currentTarget.value)}
            placeholder={placeholder}
            readOnly={readOnly || disabled}
            disabled={disabled}
            // biome-ignore lint/a11y/noAutofocus: mirrors the editor's autoFocus contract
            autoFocus={autoFocus}
            aria-label={ariaLabel}
            spellCheck={false}
            autoCapitalize="off"
            autoComplete="off"
            autoCorrect="off"
            rows={Math.max(1, value.split('\n').length)}
            className={cn(
              EDITOR_TYPOGRAPHY,
              'relative block w-full resize-none overflow-hidden',
              'bg-transparent text-ink caret-ink',
              'placeholder:text-ink-ghost focus:outline-none',
            )}
          />
        ) : null}
      </div>
    )
  },
)
CodeEditor.displayName = 'CodeEditor'
