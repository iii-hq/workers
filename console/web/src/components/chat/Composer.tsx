import type { LexicalEditor } from 'lexical'
import { useCallback, useRef, useState } from 'react'
import { PermissionModePicker } from '@/components/permissions/PermissionModePicker'
import { Button } from '@/components/ui/Button'
import type { PermissionMode } from '@/lib/backend/approval-settings'
import type { FunctionEntry } from '@/lib/functions'
import { Select } from '@/components/ui/Select'
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
  onModeChange: (next: Mode) => void
  onModelChange: (next: ModelId) => void
  onThinkingLevelChange: (next: ThinkingLevel) => void
  onPermissionModeChange: (next: PermissionMode) => void
  onSubmit: (payload: ComposerSubmitPayload) => void
  onStop?: () => void
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
  onModeChange,
  onModelChange,
  onThinkingLevelChange,
  onPermissionModeChange,
  onSubmit,
  onStop,
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

  const inputDisabled = isStreaming || blocked

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
            isStreaming
              ? 'streaming response…'
              : blocked
                ? blockedPlaceholder
                : 'send a message…'
          }
          disabled={inputDisabled}
          initialContent={initialContent}
          functionEntries={functionEntries}
        />
      </div>

      <div className="flex items-center gap-2 flex-wrap px-3 py-2 border-t border-rule-2">
        <AttachmentButton onAttach={handleAttach} disabled={inputDisabled} />
        <ModePicker value={mode} onChange={onModeChange} />
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
