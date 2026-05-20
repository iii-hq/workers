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
        <span aria-hidden="true" className="text-ink-faint shrink-0">
          {/* tiny "file" glyph: hairline rectangle with a corner fold */}
          <svg
            width="10"
            height="12"
            viewBox="0 0 10 12"
            fill="none"
            stroke="currentColor"
            strokeWidth="1"
            aria-hidden="true"
          >
            <path d="M1 1H6L9 4V11H1V1Z" />
            <path d="M6 1V4H9" />
          </svg>
        </span>
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
          ×
        </button>
      ) : null}
    </div>
  )
}
