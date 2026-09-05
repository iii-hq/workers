import { Blocks, File, Image as ImageIcon, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import {
  ImageThumbnailButton,
  ImageViewer,
  useImageViewer,
} from '@/components/ui/ImageViewer'
import { slashCommandLabel } from '@/lib/slash-commands'
import { cn } from '@/lib/utils'
import type { Attachment } from '@/types/chat'

function chipIcon(type: string) {
  if (type === 'text/x-skill') return Blocks
  return File
}

/* A skill chip reads `/coder/index`: the `skill:` namespace stays hidden
   here as it is in the command pill and the palette. */
function chipName(attachment: Attachment): string {
  if (attachment.type === 'text/x-skill') {
    return `/${slashCommandLabel(attachment.name)}`
  }
  return attachment.name
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
  const viewable =
    attachment.type.startsWith('image/') &&
    (attachment.dataUrl !== undefined || attachment.file !== undefined)
  const Icon = chipIcon(attachment.type)
  const viewer = useImageViewer()
  const src = useImageSource(attachment, viewer.open)
  return (
    <div
      className={cn(
        'inline-flex items-center gap-x-2 rounded-sm bg-surface px-2 py-1 font-mono text-[12px] text-ink max-w-[260px]',
        className,
      )}
    >
      {viewable ? (
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
            src={src}
            alt={attachment.name}
            title={attachment.name}
            description={`${attachment.type.replace('image/', '')} · ${formatSize(attachment.size)}`}
          />
        </>
      ) : (
        <Icon size={16} aria-hidden className="text-ink-faint shrink-0" />
      )}
      <span className="truncate min-w-0">{chipName(attachment)}</span>
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

/** The preview data URL when one was kept, else an object URL over the File while the viewer is open. */
function useImageSource(
  attachment: Attachment,
  open: boolean,
): string | undefined {
  const [objectUrl, setObjectUrl] = useState<string | undefined>(undefined)
  const file = attachment.dataUrl ? undefined : attachment.file
  useEffect(() => {
    if (!open || !file) return
    const url = URL.createObjectURL(file)
    setObjectUrl(url)
    return () => {
      URL.revokeObjectURL(url)
      setObjectUrl(undefined)
    }
  }, [open, file])
  return attachment.dataUrl ?? objectUrl
}
