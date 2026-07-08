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
  KEY_ARROW_UP_COMMAND,
  KEY_ENTER_COMMAND,
  type LexicalEditor,
} from 'lexical'
import { useEffect, useMemo, useRef } from 'react'
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
  /* no theme classes — surface inherits Geist from <body> */
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

/**
 * Up-arrow in an EMPTY editor recalls a message for editing (e.g. the last
 * queued message — "press ↑ to edit"). `onRecall` returns the text to load, or
 * null to let Up do its normal caret move. We gate on empty so an in-progress
 * draft is never clobbered, and defer to an open typeahead (which owns Up/Down
 * for option navigation). Loads the returned text and puts the caret at the end.
 */
function RecallOnArrowUpPlugin({
  onRecall,
  menuOpenRef,
}: {
  onRecall?: () => string | null
  menuOpenRef: React.MutableRefObject<boolean>
}) {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    if (!onRecall) return
    return editor.registerCommand(
      KEY_ARROW_UP_COMMAND,
      (event) => {
        if (menuOpenRef.current) return false
        let empty = false
        editor.getEditorState().read(() => {
          empty = $getRoot().getTextContent().length === 0
        })
        if (!empty) return false
        const text = onRecall()
        if (text == null) return false
        event?.preventDefault()
        editor.update(() => {
          const root = $getRoot()
          root.clear()
          const paragraph = $createParagraphNode()
          paragraph.append($createTextNode(text))
          root.append(paragraph)
          paragraph.selectEnd()
        })
        return true
      },
      COMMAND_PRIORITY_LOW,
    )
  }, [editor, onRecall, menuOpenRef])
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
  /** Up-arrow in an empty editor: return text to load, or null. */
  onArrowUpWhenEmpty?: () => string | null
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
  onArrowUpWhenEmpty,
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
      <RecallOnArrowUpPlugin
        onRecall={onArrowUpWhenEmpty}
        menuOpenRef={menuOpenRef}
      />
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
