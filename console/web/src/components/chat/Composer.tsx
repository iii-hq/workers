import type { LexicalEditor } from 'lexical'
import { ArrowUp, Square } from 'lucide-react'
import { useCallback, useRef, useState } from 'react'
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
  /** Locked after the first send — picker renders read-only. */
  workingDirLocked?: boolean
  onModeChange: (next: Mode) => void
  onModelChange: (next: ModelId) => void
  onWorkingDirChange?: (next: string) => void
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
        <ModelPicker
          value={model}
          options={modelOptions}
          thinkingLevel={thinkingLevel}
          onChange={onModelChange}
          onThinkingLevelChange={onThinkingLevelChange}
          disabled={inputDisabled}
          loading={catalogLoading}
        />
        <div className="flex-1 min-w-0" />
        <AttachmentButton onAttach={handleAttach} disabled={inputDisabled} />
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
