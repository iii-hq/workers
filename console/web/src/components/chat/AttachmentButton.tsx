import { useRef } from 'react'
import { Button } from '@/components/ui/Button'
import { uid } from '@/hooks/use-conversations'
import type { Attachment } from '@/types/chat'

interface AttachmentButtonProps {
  onAttach: (attachments: Attachment[]) => void
  disabled?: boolean
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
      <Button
        variant="icon"
        size="icon"
        type="button"
        disabled={disabled}
        aria-label="attach files"
        title="attach files"
        onClick={() => inputRef.current?.click()}
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 14 14"
          fill="none"
          stroke="currentColor"
          strokeWidth="1"
          strokeLinecap="square"
          aria-hidden="true"
        >
          {/* paperclip: two parallel diagonals with hairline strokes, no curves */}
          <path d="M9.5 3.5L4.5 8.5L4.5 10L6 10L11 5L11 3.5L9.5 2L8 2L3 7" />
        </svg>
      </Button>
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
