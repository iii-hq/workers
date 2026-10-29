import type * as monacoNs from 'monaco-editor'
import * as React from 'react'
import { createPortal } from 'react-dom'
import { bindingMatchesEvent } from '@/lib/keybindings/bindings'
import { matchesKeybinding } from '@/lib/keybindings/registry'
import { requestPaletteOpen } from '@/lib/palette/open-request'
import { NEUTRAL_SURFACE } from '@/lib/neutral-surface'
import { centeredScrollTop, clampLine, scrollParentOf } from '@/lib/reveal-line'
import { cn } from '@/lib/utils'
import {
  type CodeEditorQuickEdit,
  type CodeEditorQuickEditScope,
  changedLineSpan,
  insertedEnd,
  QUICK_EDIT_DEFAULT_SHORTCUT,
  QuickEditCard,
  type QuickEditPhase,
  shortcutLabel,
} from './code-editor-quick-edit'

export type {
  CodeEditorQuickEdit,
  CodeEditorQuickEditRequest,
  CodeEditorQuickEditScope,
} from './code-editor-quick-edit'

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

/** An action offered on a selection, in a small floating bar above it. */
export interface CodeEditorSelectionAction {
  id: string
  label: string
  icon?: React.ReactNode
  /** A chord that runs the action while text is selected and the editor
      has focus, in the console's shortcut syntax (`Mod+L`); the bar shows
      it beside the label. */
  shortcut?: string
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
  'aria-invalid'?: React.AriaAttributes['aria-invalid']
  'aria-describedby'?: React.AriaAttributes['aria-describedby']
  'aria-errormessage'?: React.AriaAttributes['aria-errormessage']
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
  /** Code size in px; line height follows at 1.5×. Default 12.5 on 19. */
  fontSize?: number
  /** Actions shown in a discreet floating bar whenever text is selected
      ("Reference in chat", …). Empty or absent: no bar. */
  selectionActions?: readonly CodeEditorSelectionAction[]
  /** Offer Quick Edit on a selection (⌘I by default, and a "Quick Edit"
      button in the selection bar): an inline prompt whose answer, from
      `run`, replaces the selected code. Absent: no Quick Edit. */
  quickEdit?: CodeEditorQuickEdit
  /** The editor's right-click menu, VS Code's: Go to Definition, Rename,
      Format Document, Refactor…, Cut / Copy / Paste, Command Palette, and
      the selection actions and Quick Edit. Off by default (prose fields). */
  contextMenu?: boolean
  /** Where the reader was (scroll, cursor, selection), from an earlier
      `onViewStateChange`: restored once, when the editor mounts. Opaque. */
  initialViewState?: unknown
  /** The reader's place, reported as they scroll or move the cursor and
      once more as the editor goes away, so a remount can come back to it. */
  onViewStateChange?: (state: unknown) => void
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
  fontSize,
}: {
  fill: boolean
  lineNumbers: boolean
  wordWrap: boolean
  minimap: boolean
  fontSize?: number
}): monacoNs.editor.IEditorOptions {
  return {
    fontSize: fontSize ?? 12.5,
    lineHeight: fontSize ? Math.round(fontSize * 1.5) : 19,
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

/** Monaco recognizes shorter, unmoved touches as taps. */
const TOUCH_TAP_MAX_DISTANCE = 30
const TOUCH_TAP_MAX_DURATION_MS = 700

let selectionWidgetSeq = 0
let quickEditSeq = 0

/** The Quick Edit card's widest; a narrow editor gets its content width. */
const QUICK_EDIT_MAX_WIDTH = 640
/** Room between the card and the first selected line. */
const QUICK_EDIT_GAP_PX = 6

/** A remembered view state without its folds: a region folded once (or
    by a stray click on the gutter) must not come back folded on its own and
    leave the line numbers looking like lines went missing. */
function withoutFolds(
  state: monacoNs.editor.ICodeEditorViewState,
): monacoNs.editor.ICodeEditorViewState {
  const contributions = state.contributionsState as
    | Record<string, unknown>
    | undefined
  if (!contributions || !('editor.contrib.folding' in contributions))
    return state
  const { 'editor.contrib.folding': _folds, ...rest } = contributions
  return { ...state, contributionsState: rest }
}

/** `Mod+L` as a Monaco keybinding (letters and digits only), for the label
    the right-click menu shows beside an action. */
function monacoKeybinding(
  monaco: typeof monacoNs,
  binding: string,
): number | null {
  const parts = binding.split('+')
  const key = parts.at(-1) ?? ''
  const code = /^[a-z]$/i.test(key)
    ? (monaco.KeyCode as unknown as Record<string, number>)[`Key${key.toUpperCase()}`]
    : /^\d$/.test(key)
      ? (monaco.KeyCode as unknown as Record<string, number>)[`Digit${key}`]
      : undefined
  if (code === undefined) return null
  let mods = 0
  for (const part of parts.slice(0, -1)) {
    if (part === 'Mod') mods |= monaco.KeyMod.CtrlCmd
    else if (part === 'Shift') mods |= monaco.KeyMod.Shift
    else if (part === 'Alt') mods |= monaco.KeyMod.Alt
    else if (part === 'Ctrl') mods |= monaco.KeyMod.WinCtrl
    else return null
  }
  return mods | code
}

/** The editor's selection as the bar and Quick Edit take it; null when empty. */
function currentSelection(
  editor: monacoNs.editor.IStandaloneCodeEditor,
): CodeEditorSelection | null {
  const model = editor.getModel()
  const sel = editor.getSelection()
  if (!model || !sel || sel.isEmpty()) return null
  return {
    startLine: sel.startLineNumber,
    startColumn: sel.startColumn,
    endLine: sel.endLineNumber,
    endColumn: sel.endColumn,
    text: model.getValueInRange(sel),
  }
}

/** One open Quick Edit card and everything it put into the editor. */
interface QuickEditSession {
  /** The card renders into this (Monaco's content widget node). */
  node: HTMLDivElement
  abort: AbortController | null
  /** Where the picked code is now (edits above it move it). */
  targetRange(): monacoNs.IRange | null
  /** Put `after` in place of `range` (one undo step) and tint `span`. */
  apply(
    range: monacoNs.IRange,
    before: string,
    after: string,
    span: { start: number; end: number } | null,
  ): void
  /** Put the code from before `apply` back. */
  revert(): void
  dispose(): void
}

function createQuickEditSession(
  editor: monacoNs.editor.IStandaloneCodeEditor,
  picked: CodeEditorSelection,
): QuickEditSession {
  const node = document.createElement('div')
  node.className = 'z-10'
  node.style.paddingBottom = `${QUICK_EDIT_GAP_PX}px`
  const zoneNode = document.createElement('div')
  let anchorLine = picked.startLine
  const zone: monacoNs.editor.IViewZone = {
    afterLineNumber: anchorLine - 1,
    heightInPx: 104,
    domNode: zoneNode,
    suppressMouseDown: true,
  }
  let zoneId: string | null = null
  const widgetId = `iii-quick-edit-${++quickEditSeq}`
  // ABOVE the first selected line, i.e. over the zone that line now has.
  const widget: monacoNs.editor.IContentWidget = {
    getId: () => widgetId,
    getDomNode: () => node,
    getPosition: () => ({
      position: { lineNumber: anchorLine, column: 1 },
      preference: [1],
    }),
  }
  const fitWidth = () => {
    const layout = editor.getLayoutInfo()
    const room = layout.contentWidth - layout.verticalScrollbarWidth - 16
    node.style.width = `${Math.max(220, Math.min(QUICK_EDIT_MAX_WIDTH, room))}px`
  }
  const placeZone = () => {
    zone.afterLineNumber = anchorLine - 1
    editor.changeViewZones((accessor) => {
      if (zoneId !== null) accessor.removeZone(zoneId)
      zoneId = accessor.addZone(zone)
    })
    editor.layoutContentWidget(widget)
  }
  fitWidth()
  editor.addContentWidget(widget)
  placeZone()
  // The card grows with its text and its phase: the zone follows, so the
  // code under it moves down instead of hiding behind it.
  const resize = new ResizeObserver(() => {
    const height = Math.ceil(node.getBoundingClientRect().height)
    if (height <= QUICK_EDIT_GAP_PX || height === zone.heightInPx) return
    zone.heightInPx = height
    editor.changeViewZones((accessor) => {
      if (zoneId !== null) accessor.layoutZone(zoneId)
    })
    editor.layoutContentWidget(widget)
  })
  resize.observe(node)
  const layoutListener = editor.onDidLayoutChange(() => {
    fitWidth()
    editor.layoutContentWidget(widget)
  })
  // 1 = TrackedRangeStickiness.AlwaysGrowsWhenTypingAtEdges.
  const target = editor.createDecorationsCollection([
    {
      range: {
        startLineNumber: picked.startLine,
        startColumn: picked.startColumn,
        endLineNumber: picked.endLine,
        endColumn: picked.endColumn,
      },
      options: { stickiness: 1, className: 'iii-quick-edit-target' },
    },
  ])
  let inserted: monacoNs.editor.IEditorDecorationsCollection | null = null
  let tint: monacoNs.editor.IEditorDecorationsCollection | null = null
  let original: string | null = null

  return {
    node,
    abort: null,
    targetRange: () => target.getRange(0),
    apply(range, before, after, span) {
      editor.pushUndoStop()
      editor.executeEdits('quick-edit', [
        { range, text: after, forceMoveMarkers: true },
      ])
      editor.pushUndoStop()
      original = before
      target.clear()
      const end = insertedEnd(range.startLineNumber, range.startColumn, after)
      inserted = editor.createDecorationsCollection([
        {
          range: {
            startLineNumber: range.startLineNumber,
            startColumn: range.startColumn,
            endLineNumber: end.line,
            endColumn: end.column,
          },
          options: { stickiness: 1 },
        },
      ])
      tint = span
        ? editor.createDecorationsCollection([
            {
              range: {
                startLineNumber: range.startLineNumber + span.start,
                startColumn: 1,
                endLineNumber: range.startLineNumber + span.end,
                endColumn: 1,
              },
              options: {
                isWholeLine: true,
                className: 'iii-quick-edit-line',
                linesDecorationsClassName: 'iii-quick-edit-gutter',
              },
            },
          ])
        : null
      // A rewrite can fold the zone into the replaced text; put the card
      // back over a line that still exists.
      anchorLine = Math.min(anchorLine, editor.getModel()?.getLineCount() ?? 1)
      placeZone()
    },
    revert() {
      const range = inserted?.getRange(0)
      if (!range || original === null) return
      editor.pushUndoStop()
      editor.executeEdits('quick-edit', [
        { range, text: original, forceMoveMarkers: true },
      ])
      editor.pushUndoStop()
    },
    dispose() {
      resize.disconnect()
      layoutListener.dispose()
      target.clear()
      inserted?.clear()
      tint?.clear()
      editor.removeContentWidget(widget)
      editor.changeViewZones((accessor) => {
        if (zoneId !== null) accessor.removeZone(zoneId)
      })
    },
  }
}

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
      'aria-invalid': ariaInvalid,
      'aria-describedby': ariaDescribedBy,
      'aria-errormessage': ariaErrorMessage,
      onKeyDown,
      completions,
      fill = false,
      lineNumbers = false,
      wordWrap = true,
      minimap = false,
      fontSize,
      selectionActions: callerActions,
      quickEdit,
      contextMenu = false,
      initialViewState,
      onViewStateChange,
    },
    ref,
  ) => {
    const hostRef = React.useRef<HTMLDivElement>(null)
    const fallbackRef = React.useRef<HTMLTextAreaElement>(null)
    const editorRef = React.useRef<monacoNs.editor.IStandaloneCodeEditor>(null)
    const applyingRef = React.useRef(false)
    // Mirror of the editor's text, kept on every edit and every external
    // replacement, so the prop sync compares against it instead of
    // reading the whole buffer back on each keystroke.
    const editorValueRef = React.useRef<string | null>(null)
    const pendingRevealRef = React.useRef<PendingReveal | null>(null)
    const reportViewStateOnDisposeRef = React.useRef<(() => void) | null>(null)
    const touchStartRef = React.useRef<{
      pointerId: number
      x: number
      y: number
      timeStamp: number
    } | null>(null)
    const browserTouchesRef = React.useRef(new Set<number>())
    const browserPanPointersRef = React.useRef(new Set<number>())
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
        editor.focus()
        scrollOuterTo(editor, lineNumber)
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
        editor.focus()
        scrollOuterTo(editor, first)
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
      fontSize,
    })
    latest.current = {
      value,
      language,
      autoFocus,
      fill,
      lineNumbers,
      wordWrap,
      minimap,
      fontSize,
    }
    const onChangeRef = React.useRef(onChange)
    onChangeRef.current = onChange
    const initialViewStateRef = React.useRef(initialViewState)
    const onViewStateChangeRef = React.useRef(onViewStateChange)
    onViewStateChangeRef.current = onViewStateChange

    // Quick Edit joins the caller's actions in the bar (its plumbing is
    // further down; the bar reaches it through this ref).
    const quickEditRef = React.useRef(quickEdit)
    quickEditRef.current = quickEdit
    const openQuickEditRef = React.useRef<
      (selection: CodeEditorSelection) => void
    >(() => {})
    const hasQuickEdit = Boolean(quickEdit) && !readOnly && !disabled
    const quickEditShortcut = quickEdit?.shortcut ?? QUICK_EDIT_DEFAULT_SHORTCUT
    const selectionActions = React.useMemo<
      readonly CodeEditorSelectionAction[] | undefined
    >(() => {
      if (!hasQuickEdit) return callerActions
      return [
        ...(callerActions ?? []),
        {
          id: 'quick-edit',
          label: 'Quick Edit',
          shortcut: quickEditShortcut,
          run: (selection) => openQuickEditRef.current(selection),
        },
      ]
    }, [callerActions, hasQuickEdit, quickEditShortcut])

    // ── selection actions ──
    // A floating bar above the start of a non-empty selection, offering the
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
        // Above the first selected line, where the eye already is; below it
        // when the selection starts at the top of the viewport.
        const anchor = {
          lineNumber: sel.startLineNumber,
          column: sel.startColumn,
        }
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
              position: anchor,
              // monaco ContentWidgetPositionPreference: 2 = BELOW, 1 = ABOVE
              preference: [1, 2],
            }),
            allowEditorOverflow: true,
          }
        }
        const widget = selectionWidgetRef.current
        // Re-anchor at the new start of the selection.
        widget.getPosition = () => ({
          position: anchor,
          preference: [1, 2],
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

    // ── quick edit ──
    // A card in a view zone above the selection (a content widget laid over
    // the zone, so it scrolls with the code and its text box takes focus),
    // the selection tracked by a decoration while the model works, then the
    // answer applied as one undoable edit with its changed lines tinted.
    const [quickEditState, setQuickEditState] = React.useState<{
      phase: QuickEditPhase
      scope: CodeEditorQuickEditScope
      instruction: string
      error: string | null
      model: string | null
      thinkingLevel: string
    } | null>(null)
    // The model picked in the card outlives one card: the next ⌘I opens on it.
    const quickEditModelRef = React.useRef<{
      model: string | null
      thinkingLevel: string
    }>({ model: null, thinkingLevel: 'default' })
    const quickEditSessionRef = React.useRef<QuickEditSession | null>(null)
    const quickEditInputRef = React.useRef<HTMLTextAreaElement | null>(null)
    const quickEditRootRef = React.useRef<HTMLDivElement | null>(null)
    const quickEditStateRef = React.useRef(quickEditState)
    quickEditStateRef.current = quickEditState

    const closeQuickEdit = React.useCallback((refocus = true) => {
      const session = quickEditSessionRef.current
      if (!session) return
      quickEditSessionRef.current = null
      session.abort?.abort()
      session.dispose()
      setQuickEditState(null)
      if (refocus) editorRef.current?.focus()
    }, [])

    const openQuickEdit = React.useCallback(
      (picked: CodeEditorSelection) => {
        const editor = editorRef.current
        const model = editor?.getModel()
        if (!editor || !model || !quickEditRef.current) return
        // A second ⌘I moves the card to the new selection.
        closeQuickEdit(false)
        hideSelectionBar()
        const session = createQuickEditSession(editor, picked)
        quickEditSessionRef.current = session
        setQuickEditState({
          phase: 'prompt',
          scope: 'selection',
          instruction: '',
          error: null,
          ...quickEditModelRef.current,
        })
        requestAnimationFrame(() => quickEditInputRef.current?.focus())
      },
      [closeQuickEdit, hideSelectionBar],
    )
    openQuickEditRef.current = openQuickEdit

    const submitQuickEdit = React.useCallback(() => {
      const editor = editorRef.current
      const model = editor?.getModel()
      const session = quickEditSessionRef.current
      const state = quickEditStateRef.current
      const run = quickEditRef.current?.run
      if (!editor || !model || !session || !state || !run) return
      const instruction = state.instruction.trim()
      if (instruction === '' || state.phase !== 'prompt') return
      const target = session.targetRange()
      if (!target) {
        closeQuickEdit()
        return
      }
      const abort = new AbortController()
      session.abort = abort
      setQuickEditState({ ...state, phase: 'running', error: null })
      const value = model.getValue()
      const scope = state.scope
      run({
        instruction,
        scope,
        selection: {
          startLine: target.startLineNumber,
          startColumn: target.startColumn,
          endLine: target.endLineNumber,
          endColumn: target.endColumn,
          text: model.getValueInRange(target),
        },
        value,
        language: latest.current.language,
        model: state.model,
        thinkingLevel: state.thinkingLevel,
        signal: abort.signal,
      })
        .then((answer) => {
          if (abort.signal.aborted || quickEditSessionRef.current !== session)
            return
          // The text moved while the model worked: the answer no longer fits.
          if (model.getValue() !== value) {
            setQuickEditState((current) =>
              current && {
                ...current,
                phase: 'prompt',
                error: 'The file changed while the edit was written. Send it again.',
              },
            )
            return
          }
          const range =
            scope === 'file' ? model.getFullModelRange() : target
          const before = model.getValueInRange(range)
          if (answer === before) {
            setQuickEditState((current) =>
              current && {
                ...current,
                phase: 'prompt',
                error: 'The model left the code as it was.',
              },
            )
            return
          }
          session.apply(range, before, answer, changedLineSpan(before, answer))
          setQuickEditState((current) => current && { ...current, phase: 'review' })
          requestAnimationFrame(() => quickEditRootRef.current?.focus())
        })
        .catch((err: unknown) => {
          if (abort.signal.aborted || quickEditSessionRef.current !== session)
            return
          setQuickEditState((current) =>
            current && {
              ...current,
              phase: 'prompt',
              error: err instanceof Error ? err.message : String(err),
            },
          )
          requestAnimationFrame(() => quickEditInputRef.current?.focus())
        })
    }, [closeQuickEdit])

    const stopQuickEdit = React.useCallback(() => {
      quickEditSessionRef.current?.abort?.abort()
      setQuickEditState((current) => current && { ...current, phase: 'prompt' })
      requestAnimationFrame(() => quickEditInputRef.current?.focus())
    }, [])

    const acceptQuickEdit = React.useCallback(() => closeQuickEdit(), [closeQuickEdit])

    const rejectQuickEdit = React.useCallback(() => {
      quickEditSessionRef.current?.revert()
      closeQuickEdit()
    }, [closeQuickEdit])

    // Withdrawn (read-only now, or the caller dropped it) or unmounted: close.
    React.useEffect(() => {
      if (!hasQuickEdit) closeQuickEdit(false)
    }, [hasQuickEdit, closeQuickEdit])
    React.useEffect(() => () => closeQuickEdit(false), [closeQuickEdit])

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
          const fontFamily = codeFontFamily(hostRef.current)
          const editor = monaco.editor.create(hostRef.current, {
            ...MONACO_OPTIONS,
            ...presentationOptions(latest.current),
            value: latest.current.value,
            language: latest.current.language,
            theme: CONSOLE_THEME,
            fontFamily,
          })
          editorRef.current = editor
          // Monaco measures glyphs once, at creation: a webfont that lands
          // later leaves the cursor and selection off by a fraction per
          // column until the editor measures again.
          void document.fonts
            ?.load(`${MONACO_OPTIONS.fontSize}px ${fontFamily}`)
            .then(() => {
              if (!disposed) monaco.editor.remeasureFonts()
            })
            .catch(() => {})
          editorValueRef.current = latest.current.value
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
            const next = editor.getValue()
            editorValueRef.current = next
            onChangeRef.current(next)
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
          // The right-click menu is the selection's surface while it is up.
          editor.onContextMenu?.(() => hideSelectionBar())
          editor.onMouseDown((event) => {
            if (onWidget(event.target)) return
            mouseDownRef.current = true
            hideSelectionBar()
          })
          editor.onMouseUp((event) => {
            if (onWidget(event.target)) return
            mouseDownRef.current = false
            if (actionsRef.current?.length && !quickEditSessionRef.current)
              showSelectionBar(editor)
          })
          editor.onDidChangeCursorSelection((event) => {
            if (!actionsRef.current?.length) return
            // The Quick Edit card is the selection's surface while it is open.
            if (quickEditSessionRef.current) {
              hideSelectionBar()
              return
            }
            // An outside refill collapses the selection, then restores the
            // reader's: neither is a selection the person just made.
            if (applyingRef.current) {
              hideSelectionBar()
              return
            }
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

          // Back where the reader left it; an explicit reveal below wins.
          const restored = initialViewStateRef.current
          if (restored) {
            try {
              editor.restoreViewState(
                withoutFolds(restored as monacoNs.editor.ICodeEditorViewState),
              )
            } catch {
              // A state from another Monaco version: start at the top.
            }
          }
          // Report the place as it moves, at most once a frame.
          let reportFrame = 0
          const reportViewState = () => {
            if (!onViewStateChangeRef.current || reportFrame) return
            reportFrame = requestAnimationFrame(() => {
              reportFrame = 0
              const state = editor.saveViewState()
              if (state) onViewStateChangeRef.current?.(state)
            })
          }
          // Decided at mount, like `fill`: only a caller that keeps the
          // place pays for the listeners.
          if (onViewStateChangeRef.current) {
            editor.onDidScrollChange(reportViewState)
            editor.onDidChangeCursorSelection(reportViewState)
            reportViewStateOnDisposeRef.current = () => {
              if (reportFrame) cancelAnimationFrame(reportFrame)
              const state = editor.saveViewState()
              if (state) onViewStateChangeRef.current?.(state)
            }
          }

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
        reportViewStateOnDisposeRef.current?.()
        reportViewStateOnDisposeRef.current = null
        editorRef.current?.dispose()
        editorRef.current = null
      }
    }, [revealLine, revealLines, hideSelectionBar, showSelectionBar])

    // Actions withdrawn while the bar is up: take it down.
    React.useEffect(() => {
      if (!hasActions) hideSelectionBar()
    }, [hasActions, hideSelectionBar])

    // Apply replacements during the commit: a passive effect can replay an
    // older controlled value after Monaco has already accepted another key.
    React.useLayoutEffect(() => {
      const editor = editorRef.current
      if (!ready || !editor || value === editorValueRef.current) return
      // New text from outside: a card over the old text no longer applies.
      closeQuickEdit(false)
      // The same editor refilled from outside (an agent rewriting the open
      // file) keeps the reader's place: setValue alone drops the cursor and
      // selection and scrolls to the top. Restored positions past the new
      // end clamp to it. A different file is a fresh editor, at the top.
      const viewState = editor.saveViewState()
      applyingRef.current = true
      editor.setValue(value)
      editorValueRef.current = value
      if (viewState) editor.restoreViewState(viewState)
      applyingRef.current = false
    }, [ready, value, closeQuickEdit])

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
        presentationOptions({ fill, lineNumbers, wordWrap, minimap, fontSize }),
      )
    }, [ready, fill, lineNumbers, wordWrap, minimap, fontSize])

    // ── right-click menu ──
    // Monaco's own (VS Code's) menu, with the selection actions and Quick
    // Edit on top. Their keys stay with the capture handler below; the
    // keybinding registered here is what puts "⌘L" beside the row.
    const menuActionsKey = contextMenu
      ? (selectionActions ?? [])
          .map((action) => `${action.id}\u0000${action.label}\u0000${action.shortcut ?? ''}`)
          .join('\n')
      : ''
    React.useEffect(() => {
      const editor = editorRef.current
      if (!ready || !editor) return
      editor.updateOptions({ contextmenu: contextMenu })
      if (!contextMenu || !menuActionsKey) return
      let cancelled = false
      const registered: monacoNs.IDisposable[] = []
      void import('@/lib/monaco').then(({ monaco }) => {
        if (cancelled || editorRef.current !== editor) return
        ;(actionsRef.current ?? []).forEach((action, index) => {
          const keybinding = action.shortcut
            ? monacoKeybinding(monaco, action.shortcut)
            : null
          registered.push(
            editor.addAction({
              id: `iii.selection.${action.id}`,
              label: action.label,
              contextMenuGroupId: '0_iii',
              contextMenuOrder: index,
              precondition: 'editorHasSelection',
              ...(keybinding ? { keybindings: [keybinding] } : {}),
              run: (target) => {
                const picked = currentSelection(
                  target as monacoNs.editor.IStandaloneCodeEditor,
                )
                const current = actionsRef.current?.find(
                  (candidate) => candidate.id === action.id,
                )
                if (picked && current) current.run(picked)
              },
            }),
          )
        })
      })
      return () => {
        cancelled = true
        for (const disposable of registered) disposable.dispose()
      }
    }, [ready, contextMenu, menuActionsKey])

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

    // Monaco owns its editing control and can replace it when switching
    // between a textarea and the browser's native EditContext. Validation
    // belongs on that control so assistive technology reads it on focus.
    React.useEffect(() => {
      const domNode = ready ? editorRef.current?.getDomNode() : null
      if (!domNode) return
      const controlSelector =
        'textarea.inputarea, .native-edit-context[role="textbox"]'
      const syncValidation = () => {
        const attributes = {
          'aria-invalid': ariaInvalid,
          'aria-describedby': ariaDescribedBy,
          'aria-errormessage': ariaErrorMessage,
        }
        for (const control of domNode.querySelectorAll(controlSelector)) {
          for (const [name, value] of Object.entries(attributes)) {
            if (value === undefined) control.removeAttribute(name)
            else control.setAttribute(name, String(value))
          }
        }
      }
      syncValidation()
      const observer = new MutationObserver((records) => {
        const controlAdded = records.some((record) =>
          Array.from(record.addedNodes).some(
            (node) =>
              node instanceof Element &&
              (node.matches(controlSelector) ||
                node.querySelector(controlSelector)),
          ),
        )
        if (controlAdded) syncValidation()
      })
      observer.observe(domNode, { childList: true, subtree: true })
      return () => observer.disconnect()
    }, [ready, ariaInvalid, ariaDescribedBy, ariaErrorMessage])

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

    const quickEditCard =
      quickEditState && quickEditSessionRef.current
        ? createPortal(
            <QuickEditCard
              phase={quickEditState.phase}
              scope={quickEditState.scope}
              onScopeChange={(scope) =>
                setQuickEditState((current) => current && { ...current, scope })
              }
              instruction={quickEditState.instruction}
              onInstructionChange={(instruction) =>
                setQuickEditState(
                  (current) => current && { ...current, instruction, error: null },
                )
              }
              error={quickEditState.error}
              modelLabel={quickEdit?.modelLabel}
              model={quickEditState.model}
              defaultModel={quickEdit?.model}
              thinkingLevel={quickEditState.thinkingLevel}
              onModelChange={(model, thinkingLevel) => {
                quickEditModelRef.current = { model, thinkingLevel }
                setQuickEditState(
                  (current) => current && { ...current, model, thinkingLevel },
                )
              }}
              onSubmit={submitQuickEdit}
              onStop={stopQuickEdit}
              onClose={() => closeQuickEdit()}
              onAccept={acceptQuickEdit}
              onReject={rejectQuickEdit}
              inputRef={quickEditInputRef}
              rootRef={quickEditRootRef}
            />,
            quickEditSessionRef.current.node,
          )
        : null

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

    const isGrowingTextAt = (x: number, y: number) => {
      const editor = editorRef.current
      if (fill || !editor) return false
      const target = editor.getTargetAtClientPoint(x, y)
      // Monaco MouseTargetType: 6 = CONTENT_TEXT, 7 = CONTENT_EMPTY,
      // 3 = GUTTER_LINE_NUMBERS. Widgets keep their own focus behavior.
      return target?.type === 6 || target?.type === 7 || target?.type === 3
    }

    const preparePointerFocus = (event: React.MouseEvent<HTMLDivElement>) => {
      const editor = editorRef.current
      if (
        event.button !== 0 ||
        !editor ||
        editor.hasTextFocus() ||
        !isGrowingTextAt(event.clientX, event.clientY)
      )
        return
      // Native focus can scroll the outer pane to the old caret before
      // Monaco resolves the original pointer event. Prepare focus without
      // scrolling; Monaco still owns the caret, drag and modifier selection.
      editor
        .getDomNode()
        ?.querySelector<HTMLElement>(
          'textarea.inputarea, .native-edit-context[role="textbox"]',
        )
        ?.focus({ preventScroll: true })
    }

    const keepBrowserTouch = (event: React.TouchEvent<HTMLDivElement>) => {
      if (
        Array.from(event.changedTouches).some((touch) =>
          browserTouchesRef.current.has(touch.identifier),
        )
      )
        event.stopPropagation()
    }

    const finishBrowserTouch = (event: React.TouchEvent<HTMLDivElement>) => {
      keepBrowserTouch(event)
      for (const touch of Array.from(event.changedTouches))
        browserTouchesRef.current.delete(touch.identifier)
    }

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
        onKeyDownCapture={(event) => {
          // A selection's shortcuts (⌘L, ⌘I) run before Monaco sees the
          // key: it has bindings of its own on these keys.
          const focused = editorRef.current
          if (
            focused?.hasTextFocus() &&
            !event.nativeEvent.isComposing &&
            selectionActions?.length
          ) {
            const action = selectionActions.find(
              (candidate) =>
                candidate.shortcut &&
                bindingMatchesEvent(candidate.shortcut, event.nativeEvent),
            )
            const picked = action ? currentSelection(focused) : null
            if (action && picked) {
              event.preventDefault()
              event.stopPropagation()
              hideSelectionBar()
              action.run(picked)
              return
            }
          }
          // ⌘K is the ADE's Search everywhere, the editor included: Monaco
          // would take it as the start of one of its chords and swallow it.
          if (
            focused?.hasTextFocus() &&
            !event.nativeEvent.isComposing &&
            matchesKeybinding('palette.toggle', event.nativeEvent)
          ) {
            event.preventDefault()
            event.stopPropagation()
            requestPaletteOpen()
            return
          }
          if (fill || !editorRef.current?.hasTextFocus()) return
          const target = event.target as HTMLElement
          const reveal = () => {
            if (!target.isConnected || document.activeElement !== target) return
            const scroller = scrollParentOf(target)
            if (!scroller) return
            const bounds = target.getBoundingClientRect()
            const viewport = scroller.getBoundingClientRect()
            const inset =
              Number.parseFloat(
                getComputedStyle(target).scrollMarginBlockStart,
              ) || 16
            if (
              bounds.top < viewport.top + inset ||
              bounds.bottom > viewport.top + scroller.clientHeight - 16
            )
              scroller.scrollTo({
                top: centeredScrollTop(
                  scroller.scrollTop,
                  bounds.top - viewport.top,
                  scroller.clientHeight,
                  bounds.height,
                ),
                behavior: 'instant',
              })
          }
          // Monaco moves its native editing control on the next frame.
          // Read its new cursor position after that render has completed.
          requestAnimationFrame(() => requestAnimationFrame(reveal))
        }}
        onMouseDownCapture={preparePointerFocus}
        onTouchStartCapture={(event) => {
          for (const touch of Array.from(event.changedTouches)) {
            if (isGrowingTextAt(touch.clientX, touch.clientY))
              browserTouchesRef.current.add(touch.identifier)
          }
          // Monaco's document gesture handler cancels native touch scrolling
          // and moves its internal viewport. Growing editors have no internal
          // scroll range: leave their text gestures to the browser's pane.
          // Native compatibility mouse events still select the tapped text.
          keepBrowserTouch(event)
        }}
        onTouchMoveCapture={keepBrowserTouch}
        onTouchEndCapture={finishBrowserTouch}
        onTouchCancelCapture={finishBrowserTouch}
        onPointerDownCapture={(event) => {
          if (event.pointerType !== 'touch') return
          if (isGrowingTextAt(event.clientX, event.clientY))
            browserPanPointersRef.current.add(event.pointerId)
          touchStartRef.current = {
            pointerId: event.pointerId,
            x: event.clientX,
            y: event.clientY,
            timeStamp: event.timeStamp,
          }
        }}
        onPointerMoveCapture={(event) => {
          if (
            event.pointerType === 'touch' &&
            browserPanPointersRef.current.has(event.pointerId)
          )
            // A tap's compatibility mousedown can leave Monaco's window
            // drag monitor waiting past pointerup. A following touch pan
            // must scroll the browser pane without extending that selection.
            // Keep pointerup flowing so Monaco can stop its monitor normally.
            event.stopPropagation()
        }}
        onPointerCancelCapture={(event) => {
          if (event.pointerType !== 'touch') return
          browserPanPointersRef.current.delete(event.pointerId)
          if (touchStartRef.current?.pointerId === event.pointerId)
            touchStartRef.current = null
        }}
        onPointerUpCapture={(event) => {
          if (event.pointerType !== 'touch') return
          browserPanPointersRef.current.delete(event.pointerId)
          const start = touchStartRef.current
          if (!start || start.pointerId !== event.pointerId) return
          touchStartRef.current = null
          if (
            event.timeStamp - start.timeStamp >= TOUCH_TAP_MAX_DURATION_MS ||
            Math.abs(event.clientX - start.x) >= TOUCH_TAP_MAX_DISTANCE ||
            Math.abs(event.clientY - start.y) >= TOUCH_TAP_MAX_DISTANCE
          )
            return
          // Touch taps are resolved on touchend, before synthetic mousedown.
          // Pointerup precedes that gesture; waiting until now leaves pans
          // and held touches unfocused while preserving the original tap.
          preparePointerFocus(event)
        }}
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
            aria-invalid={ariaInvalid}
            aria-describedby={ariaDescribedBy}
            aria-errormessage={ariaErrorMessage}
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
        {quickEditCard}
      </div>
    )
  },
)
CodeEditor.displayName = 'CodeEditor'

/**
 * The bar itself: a raised strip of quiet buttons over the selection's first
 * line, each with its shortcut ("Add to Chat ⌘L", "Quick Edit ⌘I").
 * Mouse-down is swallowed so pressing a button never collapses the
 * selection it acts on.
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
      className={cn(
        NEUTRAL_SURFACE,
        'my-1 flex items-center gap-0.5 rounded-lg bg-popover p-1 font-inter text-popover-foreground shadow-md ring-1 ring-foreground/10',
      )}
      onMouseDown={(event) => event.preventDefault()}
    >
      {actions.map((action) => (
        <button
          key={action.id}
          type="button"
          aria-keyshortcuts={action.shortcut ? ariaShortcut(action.shortcut) : undefined}
          onClick={() => {
            action.run(selection)
            onRun()
          }}
          className="inline-flex h-7 items-center gap-2 rounded-md px-2 text-sm text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-ink-faint"
        >
          {action.icon}
          {action.label}
          {action.shortcut ? (
            <kbd className="rounded-md bg-surface-hover px-1.5 font-inter text-xs leading-5 text-ink-faint">
              {shortcutLabel(action.shortcut)}
            </kbd>
          ) : null}
        </button>
      ))}
    </div>
  )
}

/** `Mod+L` in ARIA's spelling: `Meta+L` on a Mac, `Control+L` elsewhere. */
function ariaShortcut(binding: string): string {
  const mac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/.test(navigator.userAgent)
  return binding
    .split('+')
    .map((part) => (part === 'Mod' ? (mac ? 'Meta' : 'Control') : part === 'Ctrl' ? 'Control' : part))
    .join('+')
}
