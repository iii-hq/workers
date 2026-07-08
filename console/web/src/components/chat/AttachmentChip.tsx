import { File, X } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { Attachment } from '@/types/chat'

interface AttachmentChipProps {
  attachment: Attachment
  onRemove?: (id: string) => void
  className?: string
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}b`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)}kb`
  return `${(bytes / (1024 * 1024)).toFixed(1)}mb`
}

export function AttachmentChip({
  attachment,
  onRemove,
  className,
}: AttachmentChipProps) {
  const isImage = attachment.type.startsWith('image/') && attachment.dataUrl
  return (
    <div
      className={cn(
        'inline-flex items-center gap-x-2 border border-rule bg-bg px-2 py-1 font-mono text-[12px] text-ink max-w-[260px]',
        className,
      )}
    >
      {isImage ? (
        <img
          src={attachment.dataUrl}
          alt=""
          className="size-6 object-cover border border-rule-2"
        />
      ) : (
        <File size={12} aria-hidden className="text-ink-faint shrink-0" />
      )}
      <span className="truncate min-w-0">{attachment.name}</span>
      <span className="text-ink-ghost tabular-nums shrink-0">
        {formatSize(attachment.size)}
      </span>
      {onRemove ? (
        <button
          type="button"
          onClick={() => onRemove(attachment.id)}
          className="text-ink-faint hover:text-accent transition-colors shrink-0"
          aria-label={`remove ${attachment.name}`}
        >
          <X size={12} aria-hidden />
        </button>
      ) : null}
    </div>
  )
}
