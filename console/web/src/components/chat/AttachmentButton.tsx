import { Paperclip } from 'lucide-react'
import { useRef } from 'react'
import { uid } from '@/hooks/use-conversations'
import { cn } from '@/lib/utils'
import type { Attachment } from '@/types/chat'

interface AttachmentButtonProps {
  onAttach: (attachments: Attachment[]) => void
  disabled?: boolean
  className?: string
}

const MAX_PREVIEW_BYTES = 1_000_000

function readPreview(file: File): Promise<string | undefined> {
  if (file.size > MAX_PREVIEW_BYTES) return Promise.resolve(undefined)
  if (!/^(image|text)\//.test(file.type)) return Promise.resolve(undefined)
  return new Promise((resolve) => {
    const reader = new FileReader()
    reader.onload = () =>
      resolve(typeof reader.result === 'string' ? reader.result : undefined)
    reader.onerror = () => resolve(undefined)
    reader.readAsDataURL(file)
  })
}

export function AttachmentButton({
  onAttach,
  disabled,
  className,
}: AttachmentButtonProps) {
  const inputRef = useRef<HTMLInputElement>(null)

  const handlePick = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? [])
    if (files.length === 0) return
    const attachments: Attachment[] = await Promise.all(
      files.map(async (f) => ({
        id: uid(),
        name: f.name,
        size: f.size,
        type: f.type || 'application/octet-stream',
        dataUrl: await readPreview(f),
      })),
    )
    onAttach(attachments)
    /* allow re-picking the same file */
    e.target.value = ''
  }

  return (
    <>
      <button
        type="button"
        disabled={disabled}
        aria-label="attach files"
        title="attach files"
        onClick={() => inputRef.current?.click()}
        className={cn(
          'inline-flex items-center justify-center p-1 text-ink-faint hover:text-ink transition-colors disabled:opacity-40 disabled:pointer-events-none',
          className,
        )}
      >
        <Paperclip size={16} aria-hidden />
      </button>
      <input
        ref={inputRef}
        type="file"
        multiple
        className="hidden"
        onChange={handlePick}
      />
    </>
  )
}
