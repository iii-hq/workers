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
import { DirectoryPicker } from './DirectoryPicker'
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
  onModeChange: (next: Mode) => void
  onModelChange: (next: ModelId) => void
  onWorkingDirChange?: (next: string) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  onPermissionModeChange: (next: PermissionMode) => void
  onSubmit: (payload: ComposerSubmitPayload) => void
  onStop?: () => void
  /** Resume a turn parked at max_turns (the ask-to-continue pause). */
  onContinue?: () => void
  /** The turn is parked awaiting the user's continue/stop decision. */
  waiting?: boolean
  isStreaming?: boolean
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
  onModeChange,
  onModelChange,
  onWorkingDirChange,
  onThinkingLevelChange,
  onPermissionModeChange,
  onSubmit,
  onStop,
  onContinue,
  waiting,
  isStreaming,
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

  // A turn parked at max_turns is non-terminal but NOT actively streaming, so
  // the input stays usable: the user can press Continue or type to steer.
  const inputDisabled = blocked || (!!isStreaming && !waiting)

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
            waiting
              ? 'press continue, or type to steer…'
              : isStreaming
                ? 'streaming response…'
                : blocked
                  ? blockedPlaceholder
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
            disabled={inputDisabled}
          />
        ) : null}
        {showPermissionMode ? (
          <PermissionModePicker
            value={permissionMode}
            onChange={onPermissionModeChange}
            disabled={inputDisabled || !!permissionModeLoading}
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
          disabled={inputDisabled}
          aria-label="thinking level"
        />
        <ModelPicker
          value={model}
          options={modelOptions}
          onChange={onModelChange}
          disabled={inputDisabled}
          loading={catalogLoading}
        />
        {waiting ? (
          <>
            <Button
              type="button"
              variant="pill"
              size="sm"
              onClick={onStop}
              aria-label="stop turn"
            >
              stop
            </Button>
            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={onContinue}
              aria-label="continue turn"
            >
              continue
              <span aria-hidden>→</span>
            </Button>
          </>
        ) : isStreaming ? (
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
