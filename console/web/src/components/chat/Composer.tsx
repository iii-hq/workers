import type { LexicalEditor } from 'lexical'
import { ArrowUp, Square } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { ConsoleExtensionSlot } from '@/extensions/ConsoleExtensions'
import type { FunctionEntry } from '@/lib/functions'
import { cn } from '@/lib/utils'
import type {
  Attachment,
  Mode,
  ModelId,
  ModelOption,
  ThinkingLevel,
} from '@/types/chat'
import { AttachmentButton } from './AttachmentButton'
import { AttachmentChip } from './AttachmentChip'
import { DirectoryPicker, type WorktreePickerOptions } from './DirectoryPicker'
import { LexicalShell } from './LexicalShell'
import { ModelPicker } from './ModelPicker'
import { ModePicker } from './ModePicker'
import { nextHistoryTarget } from './queue-history'

export interface ComposerSubmitPayload {
  text: string
  attachments: Attachment[]
}

interface ComposerProps {
  mode: Mode
  model: ModelId | null
  modelOptions: ModelOption[]
  catalogLoading?: boolean
  /** Context exposed to worker-owned controls mounted beside the composer. */
  extensionContext?: Record<string, unknown>
  thinkingLevel: ThinkingLevel
  /** Show the per-session working-directory picker (real backend only). */
  showWorkingDir?: boolean
  workingDir?: string | null
  /**
   * When true, the picker renders read-only. NOT set after the first send —
   * the working dir stays re-scopable mid-conversation (ChatView always
   * passes `false`); this exists for callers that want a genuinely locked
   * picker (e.g. an embedded/read-only surface).
   */
  workingDirLocked?: boolean
  /** Stale-dir validation failure; auto-opens the picker with the message. */
  workingDirError?: string | null
  /** Stack default folder — pinned "default" row in the picker. */
  defaultWorkingDir?: string | null
  /** Worktrees tab in the picker (real backend + worktree worker only). */
  worktreePicker?: WorktreePickerOptions
  onModeChange: (next: Mode) => void
  onModelChange: (next: ModelId) => void
  onWorkingDirChange?: (next: string) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  onSubmit: (payload: ComposerSubmitPayload) => void
  onStop?: () => void
  isStreaming?: boolean
  /**
   * When true, the editor stays unlocked while streaming: a submit queues the
   * message on the running turn (delivered when the stream ends) and the stop
   * button stays available. When false (mock backends), streaming locks the
   * editor as before.
   */
  queueWhileStreaming?: boolean
  /** External lock (e.g. harness not installed). Editor + send disabled. */
  blocked?: boolean
  /** Placeholder while `blocked` is true. */
  blockedPlaceholder?: string
  /** Initial editor content (applied once on mount). */
  initialContent?: (editor: LexicalEditor) => void
  /** Initial attachment chips (applied once on mount). */
  initialAttachments?: Attachment[]
  functionEntries?: FunctionEntry[]
  /**
   * Queued messages the composer can browse+edit with ↑/↓, oldest→newest.
   * Non-destructive: browsing just loads a message; the change is committed on
   * submit (see `onEditQueued`). When set alongside `onEditQueued`, ↑/↓ cycle.
   */
  queuedForEdit?: Array<{ id: string; text: string; attachments: Attachment[] }>
  /**
   * Submit while browsing a queued message: save the edit in place (preserving
   * its queue position) with the new text + attachments, or remove it when the
   * payload is `null` (submitting an emptied composer). Given its id.
   */
  onEditQueued?: (
    id: string,
    payload: { text: string; attachments: Attachment[] } | null,
  ) => void
  /** Which queued message is being browsed (`null` = live draft), for highlight. */
  onBrowseChange?: (id: string | null) => void
}

export function Composer({
  mode,
  model,
  modelOptions,
  catalogLoading,
  extensionContext,
  thinkingLevel,
  showWorkingDir,
  workingDir,
  workingDirLocked,
  workingDirError,
  defaultWorkingDir,
  worktreePicker,
  onModeChange,
  onModelChange,
  onWorkingDirChange,
  onThinkingLevelChange,
  onSubmit,
  onStop,
  isStreaming,
  queueWhileStreaming,
  blocked,
  blockedPlaceholder = 'chat unavailable…',
  initialContent,
  initialAttachments,
  functionEntries,
  queuedForEdit,
  onEditQueued,
  onBrowseChange,
}: ComposerProps) {
  const [attachments, setAttachments] = useState<Attachment[]>(
    initialAttachments ?? [],
  )
  const [clearToken, setClearToken] = useState(0)
  const textRef = useRef('')

  // ↑/↓ browse the queued messages for editing. `browseId` is the message the
  // editor currently holds (null = a live draft). Navigation is non-destructive
  // — the message is removed from the queue only when the edit is submitted.
  const [browseId, setBrowseId] = useState<string | null>(null)
  const setBrowse = useCallback(
    (id: string | null) => {
      setBrowseId(id)
      onBrowseChange?.(id)
    },
    [onBrowseChange],
  )

  // Drop the browse cursor if the message it pointed at left the queue.
  useEffect(() => {
    if (browseId !== null && !queuedForEdit?.some((m) => m.id === browseId)) {
      setBrowse(null)
    }
  }, [queuedForEdit, browseId, setBrowse])

  // Apply the pure ↑/↓ decision (see queue-history): load the chosen message
  // (returning its text for LexicalShell to insert) or return null to let the
  // arrow move the caret.
  const handleHistoryNav = useCallback(
    (direction: 'up' | 'down'): string | null => {
      const result = nextHistoryTarget(
        queuedForEdit ?? [],
        browseId,
        textRef.current,
        direction,
      )
      if (result.kind === 'noop') return null
      setBrowse(result.target.id)
      setAttachments(result.target.attachments)
      textRef.current = result.target.text
      return result.target.text
    },
    [queuedForEdit, browseId, setBrowse],
  )

  const inputDisabled = blocked || (isStreaming && !queueWhileStreaming)
  // Turn options are frozen on the running turn; changing them mid-stream
  // would silently not apply, so the pickers stay locked while streaming.
  const optionsDisabled = isStreaming || blocked

  const handleSubmit = useCallback(() => {
    if (inputDisabled) return
    const text = textRef.current.trim()
    const empty = !text && attachments.length === 0
    // Editing a queued message: save it in place (or remove it when emptied)
    // instead of sending a new message. A blank live composer is a no-op.
    if (browseId !== null) {
      onEditQueued?.(browseId, empty ? null : { text, attachments })
      setBrowse(null)
    } else {
      if (empty) return
      onSubmit({ text, attachments })
    }
    textRef.current = ''
    setAttachments([])
    setClearToken((t) => t + 1)
  }, [inputDisabled, attachments, onSubmit, browseId, onEditQueued, setBrowse])

  const handleAttach = useCallback((next: Attachment[]) => {
    setAttachments((current) => [...current, ...next])
  }, [])

  const handleRemoveAttachment = useCallback((id: string) => {
    setAttachments((current) => current.filter((a) => a.id !== id))
  }, [])

  return (
    <div className="border border-rule bg-panel">
      {attachments.length > 0 ? (
        <div className="flex flex-wrap gap-2 p-3 border-b border-rule-2">
          {attachments.map((a) => (
            <AttachmentChip
              key={a.id}
              attachment={a}
              onRemove={handleRemoveAttachment}
            />
          ))}
        </div>
      ) : null}

      <div className="px-1 pt-1">
        <LexicalShell
          onChange={(text) => {
            textRef.current = text
          }}
          onSubmit={handleSubmit}
          clearToken={clearToken}
          placeholder={
            blocked
              ? blockedPlaceholder
              : isStreaming
                ? queueWhileStreaming
                  ? 'queue a message…'
                  : 'streaming response…'
                : 'send a message…'
          }
          disabled={inputDisabled}
          initialContent={initialContent}
          functionEntries={functionEntries}
          workingDir={workingDir}
          onHistoryNav={onEditQueued ? handleHistoryNav : undefined}
        />
      </div>

      <div className="flex items-center gap-2 flex-wrap px-3 py-2 border-t border-rule-2">
        <ModePicker value={mode} onChange={onModeChange} />
        {showWorkingDir && onWorkingDirChange ? (
          <DirectoryPicker
            value={workingDir ?? null}
            onChange={onWorkingDirChange}
            locked={workingDirLocked}
            disabled={optionsDisabled}
            externalError={workingDirError}
            defaultDir={defaultWorkingDir}
            worktrees={worktreePicker}
          />
        ) : null}
        <ConsoleExtensionSlot
          name="chat.composer.controls"
          context={{
            ...extensionContext,
            disabled: optionsDisabled,
          }}
        />
        <ModelPicker
          value={model}
          options={modelOptions}
          thinkingLevel={thinkingLevel}
          onChange={onModelChange}
          onThinkingLevelChange={onThinkingLevelChange}
          disabled={optionsDisabled}
          loading={catalogLoading}
        />
        <div className="flex-1 min-w-0" />
        <AttachmentButton onAttach={handleAttach} disabled={inputDisabled} />
        {isStreaming && queueWhileStreaming ? (
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={handleSubmit}
            disabled={blocked}
            aria-label="queue message"
          >
            send
            <span aria-hidden>→</span>
          </Button>
        ) : null}
        {isStreaming ? (
          <button
            type="button"
            onClick={onStop}
            aria-label="stop generating"
            className={cn(
              'inline-flex items-center justify-center size-8 rounded-full bg-bg text-ink',
              '[html[data-theme=dark]_&]:bg-white [html[data-theme=dark]_&]:text-[#0a0a0a]',
              'hover:opacity-80 transition-opacity duration-150',
              'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
              'disabled:pointer-events-none disabled:opacity-40',
            )}
          >
            <Square size={16} aria-hidden className="fill-black/90" />
          </button>
        ) : (
          <button
            type="button"
            onClick={handleSubmit}
            disabled={blocked}
            aria-label="send message"
            className={cn(
              'inline-flex items-center justify-center size-8 rounded-full bg-bg text-ink',
              '[html[data-theme=dark]_&]:bg-white [html[data-theme=dark]_&]:text-[#0a0a0a]',
              'hover:opacity-80 transition-opacity duration-150',
              'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
              'disabled:pointer-events-none disabled:opacity-40',
            )}
          >
            <ArrowUp size={20} aria-hidden />
          </button>
        )}
      </div>
    </div>
  )
}
