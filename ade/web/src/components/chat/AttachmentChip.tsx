import {
  Blocks,
  Download,
  File,
  Image as ImageIcon,
  Loader2,
  TriangleAlert,
  X,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import {
  ImageThumbnailButton,
  ImageViewer,
  useImageViewer,
} from '@/components/ui/ImageViewer'
import { useLazyAttachment } from '@/hooks/use-lazy-attachment'
import {
  type CachedAttachment,
  loadAttachment,
} from '@/lib/attachments/attachment-cache'
import { base64ToBlob } from '@/lib/attachments/store'
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
  /**
   * The session whose store holds this attachment's original bytes. With it,
   * a chip carrying an `attachmentId` offers the original for download; the
   * composer's chips (not yet sent, bytes still local) leave it unset.
   */
  sessionId?: string
  className?: string
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}b`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)}kb`
  return `${(bytes / (1024 * 1024)).toFixed(1)}mb`
}

/**
 * `true` for a picture whose bytes stayed in the store: the transcript read
 * left them out (see `fetchTranscript`) and the chip fetches them itself when
 * it scrolls into view. Needs a session to fetch from — a composer chip
 * restored from a parked draft has an id and no bytes too, but ChatView
 * hydrates that one into a `File` in the background and never passes
 * `sessionId` here.
 */
export function isLazyImage(
  attachment: Attachment,
  sessionId: string | undefined,
): boolean {
  return Boolean(
    sessionId &&
      attachment.attachmentId &&
      attachment.type.startsWith('image/') &&
      attachment.dataUrl === undefined &&
      attachment.file === undefined,
  )
}

export function AttachmentChip({
  attachment,
  onRemove,
  sessionId,
  className,
}: AttachmentChipProps) {
  const lazy = isLazyImage(attachment, sessionId)
  const stored = useLazyAttachment(sessionId, attachment.attachmentId, lazy)
  const viewable =
    attachment.type.startsWith('image/') &&
    (attachment.dataUrl !== undefined || attachment.file !== undefined || lazy)
  const Icon = chipIcon(attachment.type)
  const viewer = useImageViewer()
  const thumbnail =
    attachment.dataUrl ??
    (stored.state.phase === 'ready' ? stored.state.dataUrl : undefined)
  const src = useImageSource(attachment, viewer.open) ?? thumbnail
  const failed = lazy && stored.state.phase === 'failed'
  // A placeholder click asks for the bytes (or asks again after a failure)
  // rather than opening a viewer on nothing; once they are here it opens.
  const onThumbnailClick = () => {
    if (thumbnail) viewer.show()
    else stored.request()
  }
  const thumbnailTitle =
    stored.state.phase === 'failed'
      ? `could not load ${attachment.name} — ${stored.state.reason}${stored.state.retryable ? '; click to retry' : ''}`
      : attachment.name
  return (
    <div
      ref={lazy ? stored.ref : undefined}
      className={cn(
        'inline-flex items-center gap-x-2 rounded-sm bg-surface px-2 py-1 font-mono text-[12px] text-ink max-w-[260px]',
        className,
      )}
      {...(lazy ? { 'data-lazy-image': stored.state.phase } : {})}
    >
      {viewable ? (
        <>
          <ImageThumbnailButton
            title={thumbnailTitle}
            onClick={onThumbnailClick}
            className="shrink-0"
          >
            {thumbnail ? (
              <img
                src={thumbnail}
                alt=""
                className="size-6 border border-rule-2 object-cover"
              />
            ) : failed ? (
              <TriangleAlert size={16} aria-hidden className="text-warn" />
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
            unavailableReason={
              stored.state.phase === 'failed'
                ? `could not load the picture: ${stored.state.reason}`
                : undefined
            }
          />
        </>
      ) : (
        <Icon size={16} aria-hidden className="text-ink-faint shrink-0" />
      )}
      <span className="truncate min-w-0">{chipName(attachment)}</span>
      <span className="text-ink-ghost tabular-nums shrink-0">
        {formatSize(attachment.size)}
      </span>
      {sessionId && attachment.attachmentId ? (
        <DownloadButton
          sessionId={sessionId}
          attachmentId={attachment.attachmentId}
          name={attachment.name}
        />
      ) : null}
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

/** How long a failed download keeps its warning before the arrow returns. */
const DOWNLOAD_FAILURE_DWELL_MS = 4000

type DownloadState =
  | { phase: 'idle' }
  | { phase: 'busy' }
  | { phase: 'failed'; reason: string }

/**
 * The bytes to hand back out, from the shared cache. A thumbnail the chip
 * already fetched is the same original, so a download after a scroll costs
 * no request; and the download's fetch, in turn, is the thumbnail's if the
 * chip has not scrolled in yet. `save` is the seam for tests, which have no
 * anchor to click. Rejects as `loadAttachment` does.
 */
export async function downloadStoredAttachment(
  sessionId: string,
  attachmentId: string,
  fallbackName: string,
  save: (blob: Blob, filename: string) => void = saveBlob,
): Promise<CachedAttachment> {
  const stored = await loadAttachment(sessionId, attachmentId)
  const blob = base64ToBlob(stored.data, stored.attachment.mime)
  save(blob, stored.attachment.name || fallbackName)
  return stored
}

/**
 * Hand the original back out. The bytes come from session-manager, not from
 * anything this tab kept: by the time a chip renders in the transcript the
 * `File` is gone (dropped on send, or never here after a reload), and the
 * store is the only copy.
 */
function DownloadButton({
  sessionId,
  attachmentId,
  name,
}: {
  sessionId: string
  attachmentId: string
  name: string
}) {
  const [state, setState] = useState<DownloadState>({ phase: 'idle' })
  const resetTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(resetTimer.current), [])

  const fail = (reason: string) => {
    setState({ phase: 'failed', reason })
    window.clearTimeout(resetTimer.current)
    resetTimer.current = window.setTimeout(
      () => setState({ phase: 'idle' }),
      DOWNLOAD_FAILURE_DWELL_MS,
    )
  }

  const download = async () => {
    if (state.phase === 'busy') return
    setState({ phase: 'busy' })
    try {
      await downloadStoredAttachment(sessionId, attachmentId, name)
      setState({ phase: 'idle' })
    } catch (err) {
      fail(err instanceof Error ? err.message : String(err))
    }
  }

  if (state.phase === 'failed') {
    return (
      <span
        role="status"
        title={`could not download ${name} — ${state.reason}`}
        className="text-warn shrink-0"
      >
        <TriangleAlert size={16} aria-hidden />
        <span className="sr-only">download failed: {state.reason}</span>
      </span>
    )
  }
  return (
    <button
      type="button"
      onClick={() => void download()}
      disabled={state.phase === 'busy'}
      className="text-ink-faint hover:text-accent transition-colors shrink-0 disabled:cursor-progress"
      aria-label={`download ${name}`}
      title={`download ${name}`}
    >
      {state.phase === 'busy' ? (
        <Loader2 size={16} aria-hidden className="animate-spin" />
      ) : (
        <Download size={16} aria-hidden />
      )}
    </button>
  )
}

/**
 * The browser's save-as, driven from a Blob. The object URL is revoked once
 * the click has been dispatched; the download itself has already taken a
 * reference by then, and holding the URL longer would pin the bytes.
 */
function saveBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.rel = 'noopener'
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 0)
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
