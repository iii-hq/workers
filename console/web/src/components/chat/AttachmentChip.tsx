import { Blocks, File, Image as ImageIcon, SquareSlash, X } from 'lucide-react'
import {
  ImageThumbnailButton,
  ImageViewer,
  useImageViewer,
} from '@/components/ui/ImageViewer'
import { cn } from '@/lib/utils'
import type { Attachment } from '@/types/chat'

/* Same icons as the composer's slash palette sources: a collapsed
 * `<command>` block reads as the command it came from, not as a file. */
function chipIcon(type: string) {
  if (type === 'text/x-slash-command') return SquareSlash
  if (type === 'text/x-skill') return Blocks
  return File
}

interface AttachmentChipProps {
  attachment: Attachment
  onRemove?: (id: string) => void
  className?: string
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}b`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)}kb`
  return `${(bytes / (1024 * 1024)).toFixed(1)}mb`
}

export function AttachmentChip({
  attachment,
  onRemove,
  className,
}: AttachmentChipProps) {
  const isImage = attachment.type.startsWith('image/')
  const Icon = chipIcon(attachment.type)
  const viewer = useImageViewer()
  return (
    <div
      className={cn(
        'inline-flex items-center gap-x-2 rounded-sm bg-surface px-2 py-1 font-mono text-[12px] text-ink max-w-[260px]',
        className,
      )}
    >
      {isImage ? (
        <>
          <ImageThumbnailButton
            title={attachment.name}
            onClick={viewer.show}
            className="shrink-0"
          >
            {attachment.dataUrl ? (
              <img
                src={attachment.dataUrl}
                alt=""
                className="size-6 border border-rule-2 object-cover"
              />
            ) : (
              <ImageIcon size={16} aria-hidden className="text-ink-faint" />
            )}
          </ImageThumbnailButton>
          <ImageViewer
            open={viewer.open}
            onOpenChange={viewer.setOpen}
            src={attachment.dataUrl}
            alt={attachment.name}
            title={attachment.name}
            description={`${attachment.type.replace('image/', '')} · ${formatSize(attachment.size)}`}
          />
        </>
      ) : (
        <Icon size={16} aria-hidden className="text-ink-faint shrink-0" />
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
          <X size={16} aria-hidden />
        </button>
      ) : null}
    </div>
  )
}
