import type { LexicalEditor } from 'lexical'
import { useCallback, useRef, useState } from 'react'
import { PermissionModePicker } from '@/components/permissions/PermissionModePicker'
import { Button } from '@/components/ui/Button'
import { Select } from '@/components/ui/Select'
import type { PermissionMode } from '@/lib/backend/approval-settings'
import type { FunctionEntry } from '@/lib/functions'
import {
  type Attachment,
  type Mode,
  type ModelId,
  type ModelOption,
  THINKING_LEVELS,
  type ThinkingLevel,
} from '@/types/chat'
import { AttachmentButton } from './AttachmentButton'
import { AttachmentChip } from './AttachmentChip'
import { DirectoryPicker, type WorktreePickerOptions } from './DirectoryPicker'
import { LexicalShell } from './LexicalShell'
import { ModelPicker } from './ModelPicker'
import { ModePicker } from './ModePicker'

export interface ComposerSubmitPayload {
  text: string
  attachments: Attachment[]
}

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
  /**
   * When true, the picker renders read-only. NOT set after the first send —
   * the working dir stays re-scopable mid-conversation (ChatView always
   * passes `false`); this exists for callers that want a genuinely locked
   * picker (e.g. an embedded/read-only surface).
   */
  workingDirLocked?: boolean
  /** Worktrees tab in the picker (real backend + worktree worker only). */
  worktreePicker?: WorktreePickerOptions
  onModeChange: (next: Mode) => void
  onModelChange: (next: ModelId) => void
  onWorkingDirChange?: (next: string) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  onPermissionModeChange: (next: PermissionMode) => void
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
  workingDirLocked,
  worktreePicker,
  onModeChange,
  onModelChange,
  onWorkingDirChange,
  onThinkingLevelChange,
  onPermissionModeChange,
  onSubmit,
  onStop,
  isStreaming,
  queueWhileStreaming,
  blocked,
  blockedPlaceholder = 'chat unavailable…',
  initialContent,
  initialAttachments,
  functionEntries,
}: ComposerProps) {
  const [attachments, setAttachments] = useState<Attachment[]>(
    initialAttachments ?? [],
  )
  const [clearToken, setClearToken] = useState(0)
  const textRef = useRef('')

  const inputDisabled = blocked || (isStreaming && !queueWhileStreaming)
  // Turn options are frozen on the running turn; changing them mid-stream
  // would silently not apply, so the pickers stay locked while streaming.
  const optionsDisabled = isStreaming || blocked

  const handleSubmit = useCallback(() => {
    if (inputDisabled) return
    const text = textRef.current.trim()
    if (!text && attachments.length === 0) return
    onSubmit({ text, attachments })
    textRef.current = ''
    setAttachments([])
    setClearToken((t) => t + 1)
  }, [inputDisabled, attachments, onSubmit])

  const handleAttach = useCallback((next: Attachment[]) => {
    setAttachments((current) => [...current, ...next])
  }, [])

  const handleRemoveAttachment = useCallback((id: string) => {
    setAttachments((current) => current.filter((a) => a.id !== id))
  }, [])

  return (
    <div className="border border-rule bg-bg">
      {attachments.length > 0 ? (
        <div className="flex flex-wrap gap-2 px-3 pt-3 pb-1 border-b border-rule-2">
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
        />
      </div>

      <div className="flex items-center gap-2 flex-wrap px-3 py-2 border-t border-rule-2">
        <AttachmentButton onAttach={handleAttach} disabled={inputDisabled} />
        <ModePicker value={mode} onChange={onModeChange} />
        {showWorkingDir && onWorkingDirChange ? (
          <DirectoryPicker
            value={workingDir ?? null}
            onChange={onWorkingDirChange}
            locked={workingDirLocked}
            disabled={optionsDisabled}
            worktrees={worktreePicker}
          />
        ) : null}
        {showPermissionMode ? (
          <PermissionModePicker
            value={permissionMode}
            onChange={onPermissionModeChange}
            disabled={optionsDisabled || !!permissionModeLoading}
          />
        ) : null}
        <div className="flex-1 min-w-0" />
        <Select<ThinkingLevel>
          value={thinkingLevel}
          options={THINKING_LEVELS.map((l) => ({
            value: l,
            label: l === 'off' ? 'thinking off' : `thinking ${l}`,
          }))}
          onChange={onThinkingLevelChange}
          disabled={optionsDisabled}
          aria-label="thinking level"
        />
        <ModelPicker
          value={model}
          options={modelOptions}
          onChange={onModelChange}
          disabled={optionsDisabled}
          loading={catalogLoading}
        />
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
          <Button
            type="button"
            variant="pill"
            size="sm"
            onClick={onStop}
            aria-label="stop generating"
          >
            stop
          </Button>
        ) : (
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={handleSubmit}
            disabled={blocked}
            aria-label="send message"
          >
            send
            <span aria-hidden>→</span>
          </Button>
        )}
      </div>
    </div>
  )
}
