import type * as monacoNs from 'monaco-editor'
import * as React from 'react'
import { createPortal } from 'react-dom'
import { centeredScrollTop, clampLine, scrollParentOf } from '@/lib/reveal-line'
import { cn } from '@/lib/utils'

export interface CodeEditorHandle {
  focus(): void
  /** Put the cursor on `line` (1-based, clamped), scroll the pane that
      holds the editor so the line sits centered, and focus. Before Monaco
      mounts the request is kept and replayed once it does. */
  revealLine(line: number, column?: number): void
  /** Select whole lines `from`..`to` (1-based, inclusive, clamped), scroll
      them into view and focus — how a `#file(path:from-to)` reference
      opens. Kept and replayed like `revealLine` before Monaco mounts. */
  revealLines(from: number, to: number): void
}

/** A non-empty selection in the editor, 1-based like the gutter. */
export interface CodeEditorSelection {
  startLine: number
  startColumn: number
  endLine: number
  endColumn: number
  text: string
}

/** An action offered on a selection, in a small floating bar by its end. */
export interface CodeEditorSelectionAction {
  id: string
  label: string
  icon?: React.ReactNode
  run(selection: CodeEditorSelection): void
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
  /** Domain identifiers to autocomplete (e.g. SQL table/column names +
      keywords). Non-empty turns on the as-you-type suggest popup (otherwise
      the editor stays prose-quiet) and registers a completion provider for
      the current `language` offering these words. Disposed on unmount. */
  completions?: readonly string[]
  /** Fill the container and own vertical scrolling instead of growing with
      content. The default grows so a short field reads like a textarea; a
      file of thousands of lines needs the editor to virtualize its own
      viewport — with `fill` only visible lines are rendered. Decided at
      mount. */
  fill?: boolean
  /** Show the line-number gutter, folding and current-line highlight — the
      code-file presentation. Off by default (prose fields). */
  lineNumbers?: boolean
  /** Soft-wrap long lines. Default true. */
  wordWrap?: boolean
  /** Render the minimap; only meaningful together with `fill`. */
  minimap?: boolean
  /** Actions shown in a discreet floating bar whenever text is selected
      ("Reference in chat", …). Empty or absent: no bar. */
  selectionActions?: readonly CodeEditorSelectionAction[]
}

/* The pre-Monaco fallback (and permanent degraded mode if the editor chunk
   ever fails to load) renders the same typography the editor is configured
   with, so the swap-in doesn't reflow the text. */
const EDITOR_TYPOGRAPHY =
  'm-0 whitespace-pre-wrap break-words px-3 py-2 text-left font-code text-[12.5px] leading-[19px]'

/** Presentation knobs that map straight onto Monaco options. Applied at
    mount and again whenever the props change. */
function presentationOptions({
  fill,
  lineNumbers,
  wordWrap,
  minimap,
}: {
  fill: boolean
  lineNumbers: boolean
  wordWrap: boolean
  minimap: boolean
}): monacoNs.editor.IEditorOptions {
  return {
    wordWrap: wordWrap ? 'on' : 'off',
    lineNumbers: lineNumbers ? 'on' : 'off',
    folding: lineNumbers,
    lineDecorationsWidth: lineNumbers ? 10 : 12,
    lineNumbersMinChars: lineNumbers ? 3 : 0,
    renderLineHighlight: lineNumbers ? 'line' : 'none',
    minimap: { enabled: fill && minimap },
    scrollbar: fill
      ? {
          vertical: 'auto',
          horizontal: 'auto',
          alwaysConsumeMouseWheel: true,
          useShadows: false,
        }
      : {
          vertical: 'hidden',
          horizontal: 'hidden',
          alwaysConsumeMouseWheel: false,
          useShadows: false,
        },
  }
}

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

/** Keyboard selections settle for this long before the bar shows. */
const SELECTION_BAR_DELAY_MS = 160

let selectionWidgetSeq = 0

type PendingReveal =
  | { kind: 'line'; line: number; column: number }
  | { kind: 'lines'; from: number; to: number }

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
      completions,
      fill = false,
      lineNumbers = false,
      wordWrap = true,
      minimap = false,
      selectionActions,
    },
    ref,
  ) => {
    const hostRef = React.useRef<HTMLDivElement>(null)
    const fallbackRef = React.useRef<HTMLTextAreaElement>(null)
    const editorRef = React.useRef<monacoNs.editor.IStandaloneCodeEditor>(null)
    const applyingRef = React.useRef(false)
    const pendingRevealRef = React.useRef<PendingReveal | null>(null)
    const [ready, setReady] = React.useState(false)

    /** Scroll the OUTER pane so `lineNumber` sits centered (growing mode). */
    const scrollOuterTo = React.useCallback(
      (editor: monacoNs.editor.IStandaloneCodeEditor, lineNumber: number) => {
        const host = hostRef.current
        // A filled editor scrolls itself; the outer pane has nothing to move.
        const scroller = latest.current.fill ? null : scrollParentOf(host)
        if (!host || !scroller) return
        const hostTop =
          host.getBoundingClientRect().top -
          scroller.getBoundingClientRect().top +
          scroller.scrollTop
        const lineTop = editor.getTopForLineNumber(lineNumber)
        const lineHeight = Math.max(
          0,
          editor.getTopForLineNumber(lineNumber + 1) - lineTop,
        )
        scroller.scrollTo({
          top: centeredScrollTop(
            hostTop,
            lineTop,
            scroller.clientHeight,
            lineHeight,
          ),
          behavior: window.matchMedia('(prefers-reduced-motion: reduce)')
            .matches
            ? 'auto'
            : 'smooth',
        })
      },
      [],
    )

    const revealLine = React.useCallback(
      (line: number, column = 1) => {
        const editor = editorRef.current
        if (!editor) {
          pendingRevealRef.current = { kind: 'line', line, column }
          return
        }
        const lineNumber = clampLine(
          line,
          editor.getModel()?.getLineCount() ?? 1,
        )
        editor.setPosition({ lineNumber, column: Math.max(1, column) })
        editor.revealLineInCenter(lineNumber)
        scrollOuterTo(editor, lineNumber)
        editor.focus()
      },
      [scrollOuterTo],
    )

    const revealLines = React.useCallback(
      (from: number, to: number) => {
        const editor = editorRef.current
        if (!editor) {
          pendingRevealRef.current = { kind: 'lines', from, to }
          return
        }
        const model = editor.getModel()
        const lineCount = model?.getLineCount() ?? 1
        const first = clampLine(Math.min(from, to), lineCount)
        const last = clampLine(Math.max(from, to), lineCount)
        // A selection the caller made is not one the person made: no bar.
        programmaticSelectionRef.current = true
        editor.setSelection({
          startLineNumber: first,
          startColumn: 1,
          endLineNumber: last,
          endColumn: model?.getLineMaxColumn(last) ?? 1,
        })
        editor.revealLinesInCenter(first, last)
        scrollOuterTo(editor, first)
        editor.focus()
      },
      [scrollOuterTo],
    )

    // The mount effect runs once; it reads mount-time props through here.
    const latest = React.useRef({
      value,
      language,
      autoFocus,
      fill,
      lineNumbers,
      wordWrap,
      minimap,
    })
    latest.current = {
      value,
      language,
      autoFocus,
      fill,
      lineNumbers,
      wordWrap,
      minimap,
    }
    const onChangeRef = React.useRef(onChange)
    onChangeRef.current = onChange

    // ── selection actions ──
    // A floating bar by the end of a non-empty selection, offering the
    // caller's actions. Monaco positions it as a content widget (so it
    // follows scrolling and escapes the wrapper's clipping); React renders
    // into the widget's node through a portal.
    const actionsRef = React.useRef(selectionActions)
    actionsRef.current = selectionActions
    const hasActions = Boolean(selectionActions && selectionActions.length > 0)
    const [selection, setSelection] =
      React.useState<CodeEditorSelection | null>(null)
    const selectionNodeRef = React.useRef<HTMLDivElement | null>(null)
    const selectionWidgetRef =
      React.useRef<monacoNs.editor.IContentWidget | null>(null)
    const selectionShownRef = React.useRef(false)
    const mouseDownRef = React.useRef(false)
    const programmaticSelectionRef = React.useRef(false)
    const selectionTimerRef = React.useRef<number | null>(null)

    const hideSelectionBar = React.useCallback(() => {
      if (selectionTimerRef.current !== null) {
        window.clearTimeout(selectionTimerRef.current)
        selectionTimerRef.current = null
      }
      const editor = editorRef.current
      const widget = selectionWidgetRef.current
      if (editor && widget && selectionShownRef.current) {
        editor.removeContentWidget(widget)
      }
      selectionShownRef.current = false
      setSelection(null)
    }, [])

    const showSelectionBar = React.useCallback(
      (editor: monacoNs.editor.IStandaloneCodeEditor) => {
        const model = editor.getModel()
        const sel = editor.getSelection()
        if (!model || !sel || sel.isEmpty()) {
          hideSelectionBar()
          return
        }
        const end = sel.getEndPosition()
        if (!selectionNodeRef.current) {
          const node = document.createElement('div')
          node.className = 'z-10'
          selectionNodeRef.current = node
        }
        const node = selectionNodeRef.current
        if (!selectionWidgetRef.current) {
          const widgetId = `iii-selection-actions-${++selectionWidgetSeq}`
          selectionWidgetRef.current = {
            getId: () => widgetId,
            getDomNode: () => node,
            getPosition: () => ({
              position: { lineNumber: end.lineNumber, column: end.column },
              // monaco ContentWidgetPositionPreference: 2 = BELOW, 1 = ABOVE
              preference: [2, 1],
            }),
            allowEditorOverflow: true,
          }
        }
        const widget = selectionWidgetRef.current
        // Re-anchor at the new end of the selection.
        widget.getPosition = () => ({
          position: { lineNumber: end.lineNumber, column: end.column },
          preference: [2, 1],
        })
        if (selectionShownRef.current) editor.layoutContentWidget(widget)
        else editor.addContentWidget(widget)
        selectionShownRef.current = true
        setSelection({
          startLine: sel.startLineNumber,
          startColumn: sel.startColumn,
          endLine: sel.endLineNumber,
          endColumn: sel.endColumn,
          text: model.getValueInRange(sel),
        })
      },
      [hideSelectionBar],
    )

    React.useImperativeHandle(ref, () => ({
      focus: () => {
        if (editorRef.current) editorRef.current.focus()
        else fallbackRef.current?.focus()
      },
      revealLine,
      revealLines,
    }))

    React.useEffect(() => {
      let disposed = false
      void import('@/lib/monaco')
        .then(({ monaco, CONSOLE_THEME, codeFontFamily }) => {
          if (disposed || !hostRef.current) return
          const filled = latest.current.fill
          const editor = monaco.editor.create(hostRef.current, {
            ...MONACO_OPTIONS,
            ...presentationOptions(latest.current),
            value: latest.current.value,
            language: latest.current.language,
            theme: CONSOLE_THEME,
            fontFamily: codeFontFamily(),
          })
          editorRef.current = editor
          editor.getModel()?.updateOptions({ tabSize: 2, insertSpaces: true })

          // Growing mode sizes the host to the content so the OUTER pane
          // scrolls; filled mode leaves the host at the container's height
          // and lets Monaco virtualize the viewport.
          const fitHeight = () => {
            if (hostRef.current && !filled)
              hostRef.current.style.height = `${editor.getContentHeight()}px`
          }
          editor.onDidChangeModelContent(() => {
            // Editing moves the text under the bar; it comes back on the
            // next selection.
            hideSelectionBar()
            if (applyingRef.current) return
            // Read through the ref-captured editor: the latest onChange is
            // re-bound below on every render via this stable dispatcher.
            onChangeRef.current(editor.getValue())
          })
          if (!filled) editor.onDidContentSizeChange(fitHeight)
          fitHeight()

          // The bar waits for the mouse button to come up (a drag would
          // otherwise chase the pointer) and for keyboard selections to
          // settle; a selection made by code (`revealLines`) shows none.
          // A press on the bar itself is not a press in the text: hiding
          // it there would unmount the button before its click fires.
          // (9 = monaco MouseTargetType.CONTENT_WIDGET.)
          const onWidget = (target: { type: number } | null) =>
            target?.type === 9
          editor.onMouseDown((event) => {
            if (onWidget(event.target)) return
            mouseDownRef.current = true
            hideSelectionBar()
          })
          editor.onMouseUp((event) => {
            if (onWidget(event.target)) return
            mouseDownRef.current = false
            if (actionsRef.current?.length) showSelectionBar(editor)
          })
          editor.onDidChangeCursorSelection((event) => {
            if (!actionsRef.current?.length) return
            if (programmaticSelectionRef.current) {
              programmaticSelectionRef.current = false
              hideSelectionBar()
              return
            }
            if (event.selection.isEmpty()) {
              hideSelectionBar()
              return
            }
            if (mouseDownRef.current) return
            if (selectionTimerRef.current !== null) {
              window.clearTimeout(selectionTimerRef.current)
            }
            selectionTimerRef.current = window.setTimeout(() => {
              selectionTimerRef.current = null
              if (editorRef.current === editor) showSelectionBar(editor)
            }, SELECTION_BAR_DELAY_MS)
          })

          if (latest.current.autoFocus) editor.focus()
          setReady(true)
          const pending = pendingRevealRef.current
          if (pending) {
            pendingRevealRef.current = null
            if (pending.kind === 'line') {
              revealLine(pending.line, pending.column)
            } else {
              revealLines(pending.from, pending.to)
            }
          }
        })
        .catch((err) => {
          console.warn(
            '[console] monaco failed to load — staying on the plain fallback editor',
            err,
          )
        })
      return () => {
        disposed = true
        if (selectionTimerRef.current !== null) {
          window.clearTimeout(selectionTimerRef.current)
          selectionTimerRef.current = null
        }
        selectionShownRef.current = false
        selectionWidgetRef.current = null
        editorRef.current?.dispose()
        editorRef.current = null
      }
    }, [revealLine, revealLines, hideSelectionBar, showSelectionBar])

    // Actions withdrawn while the bar is up: take it down.
    React.useEffect(() => {
      if (!hasActions) hideSelectionBar()
    }, [hasActions, hideSelectionBar])

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
      const editor = editorRef.current
      if (!editor) return
      editor.updateOptions(
        presentationOptions({ fill, lineNumbers, wordWrap, minimap }),
      )
    }, [ready, fill, lineNumbers, wordWrap, minimap])

    React.useEffect(() => {
      if (!ready) return
      const editor = editorRef.current
      if (!editor) return
      editor.updateOptions({
        readOnly: !!(readOnly || disabled),
        domReadOnly: !!(readOnly || disabled),
        placeholder,
        ariaLabel,
      })
      // The wrapper's `inert` already blurs in browsers that support it;
      // this is the explicit fallback so a disabled editor never keeps a
      // hidden focused textarea.
      if (disabled && (editor.hasTextFocus() || editor.hasWidgetFocus())) {
        const active = document.activeElement
        if (active instanceof HTMLElement) active.blur()
      }
    }, [ready, readOnly, disabled, placeholder, ariaLabel])

    // Autocomplete: a non-empty `completions` turns on the as-you-type suggest
    // popup (the default stays prose-quiet) and registers a provider for the
    // current language offering those words. The joined key keeps the effect
    // from churning when the parent passes a fresh array of the same words.
    // '\n' as separator: identifiers can't contain it, and unlike the
    // invisible control character it replaced, it can't masquerade as an
    // empty string in an editor or a grep.
    const completionsKey = (completions ?? []).join('\n')
    React.useEffect(() => {
      const editor = editorRef.current
      if (!ready || !editor) return
      const words = completionsKey
        ? completionsKey.split('\n').filter(Boolean)
        : []
      if (words.length === 0) return
      editor.updateOptions({
        quickSuggestions: true,
        wordBasedSuggestions: 'off',
      })
      let disposable: monacoNs.IDisposable | undefined
      let cancelled = false
      void import('@/lib/monaco').then(({ monaco }) => {
        if (cancelled) return
        disposable = monaco.languages.registerCompletionItemProvider(language, {
          triggerCharacters: [' ', '.', '('],
          provideCompletionItems(model, position) {
            // The provider is registered per-language (global), so only answer
            // for this editor's own model — otherwise one editor's words would
            // surface in every other editor of the same language.
            if (model !== editor.getModel()) return { suggestions: [] }
            const w = model.getWordUntilPosition(position)
            const range = {
              startLineNumber: position.lineNumber,
              endLineNumber: position.lineNumber,
              startColumn: w.startColumn,
              endColumn: w.endColumn,
            }
            return {
              suggestions: words.map((label) => ({
                label,
                kind: monaco.languages.CompletionItemKind.Field,
                insertText: label,
                range,
              })),
            }
          },
        })
      })
      return () => {
        cancelled = true
        disposable?.dispose()
        // Restore the component's suggest baseline (prose-quiet) rather than
        // hardcoding — both options return to how the editor was constructed.
        editorRef.current?.updateOptions({
          quickSuggestions: MONACO_OPTIONS.quickSuggestions,
          wordBasedSuggestions: MONACO_OPTIONS.wordBasedSuggestions,
        })
      }
    }, [ready, completionsKey, language])

    const selectionBar =
      selection && selectionNodeRef.current && selectionActions?.length
        ? createPortal(
            <SelectionActionBar
              actions={selectionActions}
              selection={selection}
              onRun={hideSelectionBar}
            />,
            selectionNodeRef.current,
          )
        : null

    return (
      // biome-ignore lint/a11y/noStaticElementInteractions: shortcut relay + gap-click focus around a real editor
      <div
        // The id lives on the wrapper — the only DOM node that survives the
        // fallback-to-Monaco swap — so anchors/deep-links keep their target.
        id={id}
        // `inert` removes the whole editor (including Monaco's hidden
        // textarea) from the tab order while disabled.
        inert={disabled || undefined}
        className={cn(
          'relative bg-bg',
          fill && 'h-full min-h-0 overflow-hidden',
          disabled && 'pointer-events-none opacity-40',
          className,
        )}
        // Monaco's gutters, widgets and dead space are focus targets that are
        // not inputs; every keystroke inside the editor is content.
        data-keybindings-standdown=""
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
          className={cn(
            // Until the swap, the host hides under the in-flow fallback
            // textarea (which is what sizes the wrapper) — `invisible`
            // keeps its box measurable for Monaco's initial layout.
            ready ? undefined : 'invisible absolute inset-0 overflow-hidden',
            ready && fill && 'h-full',
          )}
        />
        {!ready ? (
          <textarea
            ref={fallbackRef}
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
              'relative block w-full resize-none',
              fill ? 'h-full overflow-auto' : 'overflow-hidden',
              'bg-transparent text-ink caret-ink',
              'placeholder:text-ink-ghost focus:outline-none',
            )}
          />
        ) : null}
        {selectionBar}
      </div>
    )
  },
)
CodeEditor.displayName = 'CodeEditor'

/**
 * The bar itself: a raised chip with one quiet button per action, tucked
 * just under the selection's end. Mouse-down is swallowed so pressing a
 * button never collapses the selection it acts on.
 */
function SelectionActionBar({
  actions,
  selection,
  onRun,
}: {
  actions: readonly CodeEditorSelectionAction[]
  selection: CodeEditorSelection
  onRun: () => void
}) {
  return (
    <div
      role="toolbar"
      aria-label="selection actions"
      className="mt-1.5 flex items-center gap-0.5 rounded-md bg-panel-raised p-0.5 shadow-floating"
      onMouseDown={(event) => event.preventDefault()}
    >
      {actions.map((action) => (
        <button
          key={action.id}
          type="button"
          onClick={() => {
            action.run(selection)
            onRun()
          }}
          className="inline-flex h-6 items-center gap-1.5 rounded-sm px-2 font-sans text-[11px] font-semibold text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-ink-faint"
        >
          {action.icon}
          {action.label}
        </button>
      ))}
    </div>
  )
}
