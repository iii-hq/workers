import { ClearEditorPlugin } from '@lexical/react/LexicalClearEditorPlugin'
import { LexicalComposer } from '@lexical/react/LexicalComposer'
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { ContentEditable } from '@lexical/react/LexicalContentEditable'
import { LexicalErrorBoundary } from '@lexical/react/LexicalErrorBoundary'
import { HistoryPlugin } from '@lexical/react/LexicalHistoryPlugin'
import { OnChangePlugin } from '@lexical/react/LexicalOnChangePlugin'
import { PlainTextPlugin } from '@lexical/react/LexicalPlainTextPlugin'
import {
  $createLineBreakNode,
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  COMMAND_PRIORITY_HIGH,
  KEY_ENTER_COMMAND,
  type LexicalEditor,
} from 'lexical'
import { useEffect, useMemo, useRef } from 'react'
import { cn } from '@/lib/utils'
import {
  $createEnvPlaceholderNode,
  EnvPlaceholderNode,
} from './EnvPlaceholderNode'
import { EnvPlaceholderTransformPlugin } from './EnvPlaceholderTransformPlugin'
import { parseTemplate } from './env-template'

interface EnvLexicalInputProps {
  id?: string
  /** The current template string (e.g. `postgres://${PGUSER:admin}@host`). */
  value: string
  /** Called with the new template string on every edit. */
  onChange: (next: string) => void
  placeholder?: string
  disabled?: boolean
  /**
   * Textarea mode for long-form values (`format: "textarea"`): Enter
   * inserts a line break instead of being swallowed, text wraps, and the
   * surface grows to several rows. Newlines round-trip as `\n` — the
   * single paragraph gains `LineBreakNode`s, whose text content is `\n`.
   */
  multiline?: boolean
  /** Forwarded to the underlying content-editable surface. */
  'aria-label'?: string
}

/**
 * Single-line Lexical input that renders `${VAR}` and `${VAR:default}`
 * tokens as inline pills. The visible text content is exactly what gets
 * saved — pills `getTextContent()` back to their canonical `${…}` form,
 * so the editor's `root.getTextContent()` is a clean round-trip with
 * what an operator would type in a regular `<input>`.
 *
 * Why Lexical instead of a `contenteditable` + regex highlight:
 *
 *   - Pills behave as atomic units (backspace deletes the whole pill;
 *     selection treats it as a single element). Hand-rolled span
 *     highlighting can't replicate that without a lot of imperative
 *     selection juggling.
 *   - The chat composer already uses the same pattern for `@fn(...)`
 *     mentions, so operators see one consistent vocabulary across
 *     console surfaces.
 *
 * Single-line behavior: we intercept Enter at high priority and discard
 * it so the operator can't accidentally split into multiple paragraphs
 * (which would not round-trip to a single line of text). Shift+Enter,
 * Cmd+Enter, etc. are also dropped — there is no use case for newlines
 * in a configuration string value.
 */
export function EnvLexicalInput({
  id,
  value,
  onChange,
  placeholder,
  disabled,
  multiline,
  'aria-label': ariaLabel,
}: EnvLexicalInputProps) {
  // Track the value we last emitted so we can detect external changes
  // (reset, mutation invalidation, parent re-seeding) without bouncing
  // our own onChange back through the editor.
  const lastEmittedRef = useRef<string>(value)

  // LexicalComposer reads `initialConfig` once at mount; capture the
  // current value in a one-shot initializer so the editor surface starts
  // with the right pill/text mix. Later external changes go through a
  // separate sync plugin.
  // biome-ignore lint/correctness/useExhaustiveDependencies: value is intentionally captured once for the initial editor state; subsequent changes flow through ExternalValueSyncPlugin to avoid remounting the editor and losing focus.
  const initialConfig = useMemo(
    () => ({
      namespace: 'iii-env-input',
      theme: {},
      nodes: [EnvPlaceholderNode],
      onError(error: Error) {
        console.error(error)
      },
      editorState: (editor: LexicalEditor) => {
        seedEditorFromTemplate(editor, value)
      },
    }),
    [],
  )

  return (
    <LexicalComposer initialConfig={initialConfig}>
      <div
        className={cn(
          'relative w-full border border-rule bg-bg px-3 text-ink',
          'focus-within:border-ink transition-colors',
          multiline ? 'min-h-36 flex items-start' : 'min-h-9 flex items-center',
          disabled && 'opacity-40 pointer-events-none',
        )}
      >
        <PlainTextPlugin
          contentEditable={
            <ContentEditable
              id={id}
              aria-label={ariaLabel}
              aria-placeholder={placeholder ?? ''}
              placeholder={
                <div
                  className={cn(
                    'env-lexical-placeholder',
                    multiline && 'env-lexical-placeholder--multiline',
                  )}
                >
                  {placeholder ?? ''}
                </div>
              }
              className={cn(
                'env-lexical-editor flex-1 py-[7px] outline-none',
                multiline && 'env-lexical-editor--multiline',
              )}
            />
          }
          ErrorBoundary={LexicalErrorBoundary}
        />
        <HistoryPlugin />
        <ClearEditorPlugin />
        <EnvPlaceholderTransformPlugin />
        {!multiline && <SingleLinePlugin />}
        <EditablePlugin disabled={disabled} />
        <ChangePlugin
          onChange={(text) => {
            lastEmittedRef.current = text
            onChange(text)
          }}
        />
        <ExternalValueSyncPlugin
          value={value}
          lastEmittedRef={lastEmittedRef}
        />
      </div>
    </LexicalComposer>
  )
}

/**
 * Reads the editor's plain-text content on every change and pushes it
 * up. The text comes from `root.getTextContent()`, which concatenates
 * regular text nodes with each decorator's `getTextContent()` — so
 * `${VAR}` pills serialize back to their template form.
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
 * Drops every Enter keystroke so the editor stays a single line. We use
 * HIGH priority to win over the PlainText plugin's default Enter
 * behavior (which would insert a new paragraph).
 */
function SingleLinePlugin() {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    return editor.registerCommand(
      KEY_ENTER_COMMAND,
      (event) => {
        event?.preventDefault()
        return true
      },
      COMMAND_PRIORITY_HIGH,
    )
  }, [editor])
  return null
}

function EditablePlugin({ disabled }: { disabled?: boolean }) {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    editor.setEditable(!disabled)
  }, [editor, disabled])
  return null
}

/**
 * Re-seeds the editor's content whenever the `value` prop changes to a
 * string we haven't just emitted ourselves. Without this, a parent
 * reset (or cache invalidation) wouldn't be reflected in the editor
 * because LexicalComposer only reads its initialConfig once.
 *
 * We compare against `lastEmittedRef` rather than the editor's current
 * text so consecutive keystrokes don't trigger a self-seed loop —
 * `value` always matches what we just emitted in that case.
 */
function ExternalValueSyncPlugin({
  value,
  lastEmittedRef,
}: {
  value: string
  lastEmittedRef: React.RefObject<string>
}) {
  const [editor] = useLexicalComposerContext()
  useEffect(() => {
    if (value === lastEmittedRef.current) return
    lastEmittedRef.current = value
    editor.update(() => {
      seedEditorFromTemplate(editor, value)
    })
  }, [editor, value, lastEmittedRef])
  return null
}

/**
 * One-shot initializer: clear the editor and rebuild a single paragraph
 * with text + env-placeholder pills based on the template string. Used
 * both at mount (via `initialConfig.editorState`) and on external resync.
 * Newlines inside text segments become `LineBreakNode`s so multiline
 * values render as lines and still round-trip to `\n` via getTextContent.
 */
function seedEditorFromTemplate(editor: LexicalEditor, template: string) {
  const root = $getRoot()
  root.clear()
  const paragraph = $createParagraphNode()
  for (const segment of parseTemplate(template)) {
    if (segment.kind === 'text') {
      const lines = segment.text.split('\n')
      lines.forEach((line, i) => {
        if (i > 0) paragraph.append($createLineBreakNode())
        if (line.length > 0) paragraph.append($createTextNode(line))
      })
    } else {
      paragraph.append(
        $createEnvPlaceholderNode(segment.variable, segment.defaultValue),
      )
    }
  }
  root.append(paragraph)
  void editor
}
