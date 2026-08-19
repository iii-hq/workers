import { AutoFocusPlugin } from '@lexical/react/LexicalAutoFocusPlugin'
import { ClearEditorPlugin } from '@lexical/react/LexicalClearEditorPlugin'
import { LexicalComposer } from '@lexical/react/LexicalComposer'
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { ContentEditable } from '@lexical/react/LexicalContentEditable'
import { LexicalErrorBoundary } from '@lexical/react/LexicalErrorBoundary'
import { HistoryPlugin } from '@lexical/react/LexicalHistoryPlugin'
import { OnChangePlugin } from '@lexical/react/LexicalOnChangePlugin'
import { PlainTextPlugin } from '@lexical/react/LexicalPlainTextPlugin'
import {
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  CLEAR_EDITOR_COMMAND,
  COMMAND_PRIORITY_LOW,
  KEY_ARROW_DOWN_COMMAND,
  KEY_ARROW_UP_COMMAND,
  KEY_ENTER_COMMAND,
  type LexicalEditor,
} from 'lexical'
import { useEffect, useLayoutEffect, useMemo, useRef } from 'react'
import { onComposerFocusRequest, onComposerInsert } from '@/lib/composer-insert'
import type { FunctionEntry } from '@/lib/functions'
import {
  type ComposerEditorSize,
  classifyComposerResize,
} from './composer-resize'
import { FileMentionNode } from './lexical/FileMentionNode'
import { FileMentionsPlugin } from './lexical/FileMentionsPlugin'
import { FileMentionTransformPlugin } from './lexical/FileMentionTransformPlugin'
import { FunctionMentionNode } from './lexical/FunctionMentionNode'
import { FunctionMentionTransformPlugin } from './lexical/FunctionMentionTransformPlugin'
import { MentionsPlugin } from './lexical/MentionsPlugin'
import { SlashCommandsPlugin } from './lexical/SlashCommandsPlugin'

interface LexicalShellProps {
  onChange: (text: string) => void
  onSubmit: () => void
  placeholder?: string
  disabled?: boolean
  /** Put the caret in the editor on mount. Off by default. */
  autoFocus?: boolean
}

const baseConfig = {
  namespace: 'iii-chat',
  /* no theme classes — surface inherits Inter from <body> */
  theme: {},
  /* Decorator nodes must be registered up-front so importJSON/restore work. */
  nodes: [FunctionMentionNode, FileMentionNode],
  onError(error: Error) {
    console.error(error)
  },
}

/**
 * Lifts the editor text out on every change. The text is whatever
 * `root.getTextContent()` returns — plain text, no marks.
 */
function ChangePlugin({ onChange }: { onChange: (text: string) => void }) {
  return (
    <OnChangePlugin
      ignoreHistoryMergeTagChange
      onChange={(state) => {
        state.read(() => {
          onChange($getRoot().getTextContent())
        })
      }}
    />
  )
}

/**
 * Enter submits, Shift+Enter inserts a newline (Lexical's default).
 * We listen at LOW priority. While a typeahead menu is open we swallow Enter
 * here (return true) so it can't fall through to PlainTextPlugin's
 * KEY_ENTER_COMMAND at EDITOR priority (which would insert a newline). The
 * typeahead runs at NORMAL and gets first shot at consuming Enter for option
 * selection; this branch is the safety net for the brief window where the
 * menu is open but the typeahead's Enter handler isn't (yet) consuming.
 */
function SubmitOnEnterPlugin({
  onSubmit,
  menuOpenRef,
}: {
  onSubmit: () => void
  menuOpenRef: React.MutableRefObject<boolean>
}) {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    return editor.registerCommand(
      KEY_ENTER_COMMAND,
      (event) => {
        if (menuOpenRef.current) {
          event?.preventDefault()
          return true
        }
        if (event && (event.shiftKey || event.metaKey || event.ctrlKey)) {
          return false
        }
        event?.preventDefault()
        onSubmit()
        return true
      },
      COMMAND_PRIORITY_LOW,
    )
  }, [editor, onSubmit, menuOpenRef])
  return null
}

/** Replace the whole editor with `text` (empty string clears it), caret at end. */
function loadEditorText(editor: LexicalEditor, text: string) {
  editor.update(() => {
    const root = $getRoot()
    root.clear()
    const paragraph = $createParagraphNode()
    if (text.length > 0) paragraph.append($createTextNode(text))
    root.append(paragraph)
    paragraph.selectEnd()
  })
}

/**
 * Up / Down browse a message history (the queued messages — "↑ to edit,
 * ↓ to cycle"). `onNav(direction)` owns the cursor and the pristine-gate
 * (it only navigates when the editor hasn't been edited, so in-progress text
 * and caret moves within a real edit are never clobbered); it returns the
 * text to load ('' clears back to a live draft) or null to let the arrow do
 * its normal caret move. Defers to an open typeahead (which owns Up/Down for
 * option navigation).
 */
function HistoryNavPlugin({
  onNav,
  menuOpenRef,
}: {
  onNav?: (direction: 'up' | 'down') => string | null
  menuOpenRef: React.MutableRefObject<boolean>
}) {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    if (!onNav) return
    const handler = (direction: 'up' | 'down') => (event: KeyboardEvent) => {
      if (menuOpenRef.current) return false
      const text = onNav(direction)
      if (text === null) return false
      event?.preventDefault()
      loadEditorText(editor, text)
      return true
    }
    const offUp = editor.registerCommand(
      KEY_ARROW_UP_COMMAND,
      handler('up'),
      COMMAND_PRIORITY_LOW,
    )
    const offDown = editor.registerCommand(
      KEY_ARROW_DOWN_COMMAND,
      handler('down'),
      COMMAND_PRIORITY_LOW,
    )
    return () => {
      offUp()
      offDown()
    }
  }, [editor, onNav, menuOpenRef])
  return null
}

/**
 * Imperatively expose a "clear" so the parent can wipe the editor after submit.
 * We use Lexical's CLEAR_EDITOR_COMMAND, which the ClearEditorPlugin handles.
 */
function ClearOnDemandPlugin({ token }: { token: number }) {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    if (token === 0) return
    editor.dispatchCommand(CLEAR_EDITOR_COMMAND, undefined)
  }, [editor, token])
  return null
}

/**
 * Drains the composer-insert bus (see `lib/composer-insert`) into the
 * editor: appended as its own paragraph with the caret at the end, so a
 * picked browser element lands ready to send or annotate. Replaces the
 * content when the editor holds only whitespace to avoid a leading blank
 * line.
 */
function ExternalInsertPlugin() {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    return onComposerInsert((text) => {
      editor.update(() => {
        const root = $getRoot()
        if (root.getTextContent().trim().length === 0) root.clear()
        const paragraph = $createParagraphNode()
        paragraph.append($createTextNode(text))
        root.append(paragraph)
        paragraph.selectEnd()
      })
      editor.focus()
    })
  }, [editor])
  return null
}

/**
 * Take the caret when a surface asks for it, e.g. a "new chat" that reused
 * the untouched one already open, where nothing remounts to focus itself.
 */
function FocusOnRequestPlugin({ enabled }: { enabled: boolean }) {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    if (!enabled) return
    return onComposerFocusRequest(() => editor.focus())
  }, [editor, enabled])
  return null
}

/**
 * Toggle the editor's editable state when `disabled` flips.
 */
function EditablePlugin({ disabled }: { disabled?: boolean }) {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    editor.setEditable(!disabled)
  }, [editor, disabled])
  return null
}

/**
 * Transition Lexical's editor between measured pixel heights without routing
 * keystrokes through React state. Measuring briefly restores intrinsic height;
 * the visible transition stays on the same contenteditable node, preserving
 * selection, IME composition and its capped internal scroll.
 */
function useAnimatedComposerHeight() {
  const frameRef = useRef<HTMLDivElement>(null)
  const editorRef = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    const frame = frameRef.current
    const editor = editorRef.current
    if (
      !frame ||
      !editor ||
      typeof MutationObserver === 'undefined' ||
      typeof ResizeObserver === 'undefined'
    ) {
      return
    }

    let previousFrameSize: ComposerEditorSize | null = null
    let resizeFrame: number | null = null
    let instantResetFrame: number | null = null
    let pendingResize: 'content' | 'container' | null = null

    const clearInstantMode = () => {
      editor.removeAttribute('data-composer-height-mode')
      instantResetFrame = null
    }

    const resetInstantModeAfterFrame = () => {
      if (instantResetFrame !== null) {
        window.cancelAnimationFrame(instantResetFrame)
      }
      instantResetFrame = window.requestAnimationFrame(clearInstantMode)
    }

    const retargetHeight = (animate: boolean) => {
      const currentHeight = editor.getBoundingClientRect().height
      if (currentHeight <= 0) return
      const scrollTop = editor.scrollTop

      editor.dataset.composerHeightMode = 'instant'
      editor.dataset.composerHeightMeasuring = ''
      const targetHeight = editor.getBoundingClientRect().height
      if (targetHeight <= 0) {
        editor.removeAttribute('data-composer-height-measuring')
        clearInstantMode()
        return
      }

      if (!editor.hasAttribute('data-composer-height-ready')) {
        editor.style.setProperty(
          '--composer-editor-height',
          `${targetHeight}px`,
        )
        editor.dataset.composerHeightReady = ''
        editor.removeAttribute('data-composer-height-measuring')
        editor.scrollTop = scrollTop
        resetInstantModeAfterFrame()
        return
      }

      editor.style.setProperty('--composer-editor-height', `${currentHeight}px`)
      editor.removeAttribute('data-composer-height-measuring')
      // Commit the current visual height as the transition's new baseline.
      void editor.offsetHeight
      editor.scrollTop = scrollTop

      if (animate && Math.abs(targetHeight - currentHeight) > 0.5) {
        if (instantResetFrame !== null) {
          window.cancelAnimationFrame(instantResetFrame)
          instantResetFrame = null
        }
        clearInstantMode()
        editor.style.setProperty(
          '--composer-editor-height',
          `${targetHeight}px`,
        )
        return
      }

      editor.style.setProperty('--composer-editor-height', `${targetHeight}px`)
      resetInstantModeAfterFrame()
    }

    const scheduleResize = (kind: 'content' | 'container') => {
      // Direct/container manipulation wins when both happen in one frame.
      if (pendingResize !== 'container') pendingResize = kind
      if (resizeFrame !== null) return
      resizeFrame = window.requestAnimationFrame(() => {
        const nextResize = pendingResize
        pendingResize = null
        resizeFrame = null
        retargetHeight(nextResize === 'content')
      })
    }

    const initialRect = editor.getBoundingClientRect()
    if (initialRect.width > 0 && initialRect.height > 0) {
      editor.dataset.composerHeightMode = 'instant'
      editor.style.setProperty(
        '--composer-editor-height',
        `${initialRect.height}px`,
      )
      editor.dataset.composerHeightReady = ''
      resetInstantModeAfterFrame()
    }

    const mutationObserver = new MutationObserver(() => {
      scheduleResize('content')
    })
    mutationObserver.observe(editor, {
      characterData: true,
      childList: true,
      subtree: true,
    })

    const frameObserver = new ResizeObserver(() => {
      const rect = frame.getBoundingClientRect()
      if (rect.width <= 0 || rect.height <= 0) return
      const nextSize = { width: rect.width, height: rect.height }
      const resizeKind = classifyComposerResize(previousFrameSize, nextSize)
      previousFrameSize = nextSize
      if (
        resizeKind === 'container' ||
        (resizeKind === 'initial' &&
          !editor.hasAttribute('data-composer-height-ready'))
      ) {
        scheduleResize('container')
      }
    })
    frameObserver.observe(frame)

    return () => {
      mutationObserver.disconnect()
      frameObserver.disconnect()
      if (resizeFrame !== null) window.cancelAnimationFrame(resizeFrame)
      if (instantResetFrame !== null) {
        window.cancelAnimationFrame(instantResetFrame)
      }
      editor.removeAttribute('data-composer-height-measuring')
      editor.removeAttribute('data-composer-height-mode')
      editor.removeAttribute('data-composer-height-ready')
      editor.style.removeProperty('--composer-editor-height')
    }
  }, [])

  return { editorRef, frameRef }
}

export interface LexicalShellHandle {
  clear: () => void
}

interface LexicalShellExtendedProps extends LexicalShellProps {
  clearToken: number
  /** Optional one-shot initializer that runs once on mount inside the editor. */
  initialContent?: (editor: LexicalEditor) => void
  functionEntries?: FunctionEntry[]
  /** Enables the `#` file-mention typeahead, scoped to this directory. */
  workingDir?: string | null
  /** Up/Down browse a message history: return text to load ('' clears), or null. */
  onHistoryNav?: (direction: 'up' | 'down') => string | null
}

export function LexicalShell({
  onChange,
  onSubmit,
  placeholder = 'send a message…',
  disabled,
  autoFocus,
  clearToken,
  initialContent,
  functionEntries,
  workingDir,
  onHistoryNav,
}: LexicalShellExtendedProps) {
  /* LexicalComposer reads initialConfig once on mount; lock it behind useMemo
     so the initializer callback identity doesn't trigger a remount on re-render. */
  // biome-ignore lint/correctness/useExhaustiveDependencies: initialContent is a one-shot mount initializer; capturing later changes would force a remount and lose editor state.
  const initialConfig = useMemo(
    () => ({
      ...baseConfig,
      editorState: initialContent ?? null,
    }),
    [],
  )
  /* Shared between the mentions plugin (the producer) and SubmitOnEnter
     (the consumer) so we can suppress submit when the typeahead is up. */
  const menuOpenRef = useRef(false)
  const { editorRef, frameRef } = useAnimatedComposerHeight()
  return (
    <LexicalComposer initialConfig={initialConfig}>
      <div ref={frameRef} className="relative">
        <PlainTextPlugin
          contentEditable={
            <ContentEditable
              ref={editorRef}
              aria-label="message composer"
              aria-placeholder={placeholder}
              placeholder={
                <div className="composer-placeholder px-3 py-2">
                  {placeholder}
                </div>
              }
              className="composer-editor px-3 py-2"
            />
          }
          ErrorBoundary={LexicalErrorBoundary}
        />
      </div>
      <HistoryPlugin />
      <ClearEditorPlugin />
      <ClearOnDemandPlugin token={clearToken} />
      <ChangePlugin onChange={onChange} />
      <SubmitOnEnterPlugin onSubmit={onSubmit} menuOpenRef={menuOpenRef} />
      <HistoryNavPlugin onNav={onHistoryNav} menuOpenRef={menuOpenRef} />
      <ExternalInsertPlugin />
      {/* Opening a session is a request to write in it, so the first
          keystroke should land in the message rather than be spent aiming.
          Lexical's own plugin waits for the editable node, which a bare
          focus() call on mount does not. */}
      {autoFocus === true && disabled !== true ? <AutoFocusPlugin /> : null}
      <FocusOnRequestPlugin enabled={disabled !== true} />
      <EditablePlugin disabled={disabled} />
      <MentionsPlugin
        menuOpenRef={menuOpenRef}
        functionEntries={functionEntries}
      />
      {workingDir ? (
        <FileMentionsPlugin menuOpenRef={menuOpenRef} workingDir={workingDir} />
      ) : null}
      <SlashCommandsPlugin menuOpenRef={menuOpenRef} />
      <FunctionMentionTransformPlugin />
      <FileMentionTransformPlugin />
    </LexicalComposer>
  )
}
