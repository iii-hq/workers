import type { LexicalEditor } from 'lexical'
import { useCallback, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import type { Attachment, Mode, ModelId, ModelOption } from '@/types/chat'
import type { FunctionEntry } from '@/lib/functions'
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
  model: ModelId
  modelOptions: ModelOption[]
  catalogLoading?: boolean
  onModeChange: (next: Mode) => void
  onModelChange: (next: ModelId) => void
  onSubmit: (payload: ComposerSubmitPayload) => void
  onStop?: () => void
  isStreaming?: boolean
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
  onModeChange,
  onModelChange,
  onSubmit,
  onStop,
  isStreaming,
  initialContent,
  initialAttachments,
  functionEntries,
}: ComposerProps) {
  const [attachments, setAttachments] = useState<Attachment[]>(
    initialAttachments ?? [],
  )
  const [clearToken, setClearToken] = useState(0)
  const textRef = useRef('')

  const handleSubmit = useCallback(() => {
    if (isStreaming) return
    const text = textRef.current.trim()
    if (!text && attachments.length === 0) return
    onSubmit({ text, attachments })
    textRef.current = ''
    setAttachments([])
    setClearToken((t) => t + 1)
  }, [isStreaming, attachments, onSubmit])

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
          placeholder={isStreaming ? 'streaming response…' : 'send a message…'}
          disabled={isStreaming}
          initialContent={initialContent}
          functionEntries={functionEntries}
        />
      </div>

      <div className="flex items-center gap-2 flex-wrap px-3 py-2 border-t border-rule-2">
        <AttachmentButton onAttach={handleAttach} disabled={isStreaming} />
        <ModePicker value={mode} onChange={onModeChange} />
        <div className="flex-1 min-w-0" />
        <ModelPicker
          value={model}
          options={modelOptions}
          onChange={onModelChange}
          disabled={isStreaming}
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
