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
import { useEffect, useMemo, useRef } from 'react'
import { onComposerInsert } from '@/lib/composer-insert'
import type { FunctionEntry } from '@/lib/functions'
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
 * Toggle the editor's editable state when `disabled` flips.
 */
function EditablePlugin({ disabled }: { disabled?: boolean }) {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    editor.setEditable(!disabled)
  }, [editor, disabled])
  return null
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
  return (
    <LexicalComposer initialConfig={initialConfig}>
      <div className="relative">
        <PlainTextPlugin
          contentEditable={
            <ContentEditable
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
