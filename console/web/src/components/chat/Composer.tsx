import {
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  type LexicalEditor,
} from 'lexical'
import { ArrowUp, Loader2, Square } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { PermissionModePicker } from '@/components/permissions/PermissionModePicker'
import type { PermissionMode } from '@/lib/backend/approval-settings'
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
import { BankPicker } from './BankPicker'
import { DirectoryPicker, type WorktreePickerOptions } from './DirectoryPicker'
import { LexicalShell } from './LexicalShell'
import { ModelPicker } from './ModelPicker'
import { ModePicker } from './ModePicker'
import { nextHistoryTarget } from './queue-history'

export interface ComposerSubmitPayload {
  text: string
  attachments: Attachment[]
}

/** Round icon action button (send / queue / stop) at the composer's edge. */
const actionButtonClass = cn(
  'inline-flex size-9 items-center justify-center rounded-full',
  'transition-colors duration-150',
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
  'disabled:pointer-events-none disabled:opacity-40',
)

/** Send/stop is armed: inverted ink fill (light chip in dark mode). */
const actionReadyClass = 'bg-ink text-bg hover:bg-ink/90'

/** Composer is empty: the action recedes into the surface. */
const actionIdleClass = 'bg-surface-active text-ink-ghost'

interface ComposerProps {
  mode: Mode
  model: ModelId | null
  modelOptions: ModelOption[]
  catalogLoading?: boolean
  /**
   * Per-conversation permission mode (manual / auto / full). Owned by
   * the backend `approval_settings` scope; ChatView passes the loaded
   * value here. While loading, the picker disables.
   */
  permissionMode: PermissionMode
  permissionModeLoading?: boolean
  /**
   * Whether to render the manual/auto/full permission-mode picker. Hidden when
   * the optional approval-gate worker is absent (nothing to control). Defaults
   * to `true` so existing callers / Storybook keep the picker.
   */
  showPermissionMode?: boolean
  thinkingLevel: ThinkingLevel
  /** Show the per-session working-directory picker (real backend only). */
  showWorkingDir?: boolean
  workingDir?: string | null
  /** Show the memory bank picker (memory worker present, real backend). */
  showMemoryBank?: boolean
  /** This chat's memory bank; null = the worker's default bank. */
  memoryBank?: string | null
  onMemoryBankChange?: (next: string | null) => void
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
  onPermissionModeChange: (next: PermissionMode) => void
  onSubmit: (payload: ComposerSubmitPayload) => void
  onStop?: () => void
  /** A stop was requested and the server hasn't finalized the turn yet. */
  stopping?: boolean
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
  /**
   * Plain-text sugar for `initialContent` (applied once on mount): seeds the
   * editor AND the internal text state, so a restored draft submits without
   * requiring a keystroke first. Ignored when `initialContent` is given.
   */
  initialText?: string
  /**
   * Live text of the user's draft, fired on every editor change EXCEPT while
   * a queued message is being browsed/edited (that text is not the draft).
   * Powers the per-session draft persistence.
   */
  onTextChange?: (text: string) => void
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
  permissionMode,
  permissionModeLoading,
  showPermissionMode = true,
  thinkingLevel,
  showWorkingDir,
  workingDir,
  showMemoryBank,
  memoryBank,
  onMemoryBankChange,
  workingDirLocked,
  workingDirError,
  defaultWorkingDir,
  worktreePicker,
  onModeChange,
  onModelChange,
  onWorkingDirChange,
  onThinkingLevelChange,
  onPermissionModeChange,
  onSubmit,
  onStop,
  stopping,
  isStreaming,
  queueWhileStreaming,
  blocked,
  blockedPlaceholder = 'chat unavailable…',
  initialContent,
  initialText,
  onTextChange,
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
  const textRef = useRef(initialContent ? '' : (initialText ?? ''))
  /* Boolean mirror of "the editor holds text": the action button swaps on
     the empty↔non-empty transition, and state updates for an unchanged
     boolean bail out — so plain typing still never re-renders the tree. */
  const [hasText, setHasText] = useState(
    () => textRef.current.trim().length > 0,
  )

  // One-shot mount initializer: seed the editor with the restored draft text.
  // Runs inside Lexical's initial-state update, so $-functions apply directly.
  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only initializer, matching LexicalShell's one-shot initialConfig semantics.
  const resolvedInitialContent = useMemo(() => {
    if (initialContent) return initialContent
    const text = initialText
    if (!text) return undefined
    return () => {
      const root = $getRoot()
      root.clear()
      const paragraph = $createParagraphNode()
      paragraph.append($createTextNode(text))
      root.append(paragraph)
    }
  }, [])

  // ↑/↓ browse the queued messages for editing. `browseId` is the message the
  // editor currently holds (null = a live draft). Navigation is non-destructive
  // — the message is removed from the queue only when the edit is submitted.
  // The ref mirror gates `onTextChange` synchronously: `setBrowse` runs before
  // the loaded text echoes back through the editor's change event, so browsed
  // queue text is never reported as the live draft.
  const [browseId, setBrowseId] = useState<string | null>(null)
  const browseIdRef = useRef<string | null>(null)
  const setBrowse = useCallback(
    (id: string | null) => {
      browseIdRef.current = id
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
      setHasText(result.target.text.trim().length > 0)
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
    setHasText(false)
    // The submitted text is no longer a draft; report the clear even if the
    // editor-clear update below is tag-filtered by the change plugin.
    onTextChange?.('')
    setAttachments([])
    setClearToken((t) => t + 1)
  }, [
    inputDisabled,
    attachments,
    onSubmit,
    browseId,
    onEditQueued,
    setBrowse,
    onTextChange,
  ])

  const handleAttach = useCallback((next: Attachment[]) => {
    setAttachments((current) => [...current, ...next])
  }, [])

  const handleRemoveAttachment = useCallback((id: string) => {
    setAttachments((current) => current.filter((a) => a.id !== id))
  }, [])

  return (
    <div className="rounded-xl bg-panel-raised shadow-raised">
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
            setHasText(text.trim().length > 0)
            if (browseIdRef.current === null) onTextChange?.(text)
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
          initialContent={resolvedInitialContent}
          functionEntries={functionEntries}
          workingDir={workingDir}
          onHistoryNav={onEditQueued ? handleHistoryNav : undefined}
        />
      </div>

      <div className="flex min-w-0 items-center gap-2 px-3 py-2">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <ModePicker
            value={mode}
            onChange={onModeChange}
            className="shrink-0"
          />
          {showWorkingDir && onWorkingDirChange ? (
            <DirectoryPicker
              value={workingDir ?? null}
              onChange={onWorkingDirChange}
              locked={workingDirLocked}
              disabled={optionsDisabled}
              externalError={workingDirError}
              defaultDir={defaultWorkingDir}
              worktrees={worktreePicker}
              className="shrink-0"
            />
          ) : null}
          {showMemoryBank && onMemoryBankChange ? (
            <BankPicker
              value={memoryBank ?? null}
              onChange={onMemoryBankChange}
              disabled={optionsDisabled}
            />
          ) : null}
          {showPermissionMode ? (
            <PermissionModePicker
              value={permissionMode}
              onChange={onPermissionModeChange}
              disabled={optionsDisabled || !!permissionModeLoading}
            />
          ) : null}
          <ModelPicker
            value={model}
            options={modelOptions}
            thinkingLevel={thinkingLevel}
            onChange={onModelChange}
            onThinkingLevelChange={onThinkingLevelChange}
            disabled={optionsDisabled}
            loading={catalogLoading}
            className="min-w-0 flex-1"
          />
        </div>

        <div className="flex shrink-0 items-center gap-1">
          <AttachmentButton
            onAttach={handleAttach}
            disabled={inputDisabled}
            className="size-9"
          />
          {/* ONE action button. Mid-stream the slot shows Stop, but the moment
              the composer holds queueable content (text or attachments) it
              flips to send — the editor advertises "queue a message…", and the
              click must queue it, not kill the turn. */}
          {isStreaming &&
          !(queueWhileStreaming && (hasText || attachments.length > 0)) ? (
            <button
              type="button"
              onClick={onStop}
              disabled={stopping}
              aria-label={stopping ? 'stopping' : 'stop generating'}
              className={cn(actionButtonClass, actionReadyClass)}
            >
              {stopping ? (
                <Loader2 size={16} aria-hidden className="animate-spin" />
              ) : (
                <Square size={16} aria-hidden className="fill-current" />
              )}
            </button>
          ) : (
            <button
              type="button"
              onClick={handleSubmit}
              disabled={blocked}
              aria-label={isStreaming ? 'queue message' : 'send message'}
              className={cn(
                actionButtonClass,
                hasText || attachments.length > 0
                  ? actionReadyClass
                  : actionIdleClass,
              )}
            >
              <ArrowUp size={20} aria-hidden />
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
