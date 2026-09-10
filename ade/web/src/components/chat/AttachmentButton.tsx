import { Paperclip } from 'lucide-react'
import { useRef } from 'react'
import { attachmentsFromFiles } from '@/lib/attachments/from-files'
import { cn } from '@/lib/utils'
import type { Attachment } from '@/types/chat'

interface AttachmentButtonProps {
  onAttach: (attachments: Attachment[]) => void
  disabled?: boolean
  className?: string
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
    onAttach(await attachmentsFromFiles(files))
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
          'inline-flex items-center justify-center p-1 text-ink-faint hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-40',
          className,
        )}
      >
        <Paperclip className="size-4 shrink-0" aria-hidden />
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
