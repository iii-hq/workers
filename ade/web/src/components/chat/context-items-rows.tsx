/**
 * The context pane's rows for the common sections (chat-projects §1.3.6):
 * the per-row menu, the image grid with its ↑ sent / ↓ received marks, and an
 * attachment row. They draw both what `harness::context-items` lists and what
 * the local transcript parse finds, through the same shapes.
 */
import uiClasses from '@iii-dev/console-ui/ui-classes'
import {
  ArrowDown,
  ArrowUp,
  Copy,
  Download,
  EllipsisVertical,
  ExternalLink,
  FileCode,
  FileIcon,
  FileSpreadsheet,
  FileText,
  ImageIcon,
  ListTree,
  Share,
} from 'lucide-react'
import type { ReactNode } from 'react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { Hint } from '@/components/ui/Hint'
import {
  ImageThumbnailButton,
  ImageViewer,
  useImageViewer,
} from '@/components/ui/ImageViewer'
import { useLazyAttachment } from '@/hooks/use-lazy-attachment'
import { copyTextToClipboard } from '@/lib/clipboard'
import { cn } from '@/lib/utils'
import type { Attachment } from '@/types/chat'
import {
  downloadStoredAttachment,
  formatSize,
  isLazyImage,
} from './AttachmentChip'

export interface ItemMenuEntry {
  label: string
  icon: ReactNode
  onSelect: () => void
}

/** A row's ⋮ menu: its entries in the order the spec's table gives them. */
export function ItemMenu({
  label,
  entries,
}: {
  /** The accessible name of the trigger (`Actions for …`). */
  label: string
  entries: readonly ItemMenuEntry[]
}) {
  if (entries.length === 0) return null
  return (
    <DropdownMenu>
      <Hint label="More actions">
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className={uiClasses.treeItemAction}
            aria-label={label}
            onClick={(event) => event.stopPropagation()}
          >
            <EllipsisVertical aria-hidden />
          </button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end">
        {entries.map((entry) => (
          <DropdownMenuItem key={entry.label} onSelect={entry.onSelect}>
            {entry.icon}
            {entry.label}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

const ICON = 'size-4 text-ink-faint'

export const menuIcons = {
  open: <ExternalLink aria-hidden className={ICON} />,
  copy: <Copy aria-hidden className={ICON} />,
  show: <ListTree aria-hidden className={ICON} />,
  share: <Share aria-hidden className={ICON} />,
  download: <Download aria-hidden className={ICON} />,
}

/** Whether this browser can hand a file to the system share sheet. */
export function canShareFiles(): boolean {
  return (
    typeof navigator !== 'undefined' &&
    typeof navigator.share === 'function' &&
    typeof navigator.canShare === 'function'
  )
}

/**
 * "Share…" where the browser has the share sheet, else "Download": the stored
 * original either way.
 */
export function shareEntry(
  sessionId: string,
  attachmentId: string | undefined,
  name: string,
): ItemMenuEntry | null {
  if (!attachmentId) return null
  const share = canShareFiles()
  return {
    label: share ? 'Share…' : 'Download',
    icon: share ? menuIcons.share : menuIcons.download,
    onSelect: () =>
      void downloadStoredAttachment(
        sessionId,
        attachmentId,
        name,
        share
          ? (blob, filename) => {
              const file = new File([blob], filename, { type: blob.type })
              if (navigator.canShare({ files: [file] })) {
                void navigator.share({ files: [file], title: filename })
              }
            }
          : undefined,
      ).catch(() => undefined),
  }
}

export function copyLinkEntry(url: string): ItemMenuEntry {
  return {
    label: 'Copy link',
    icon: menuIcons.copy,
    onSelect: () => void copyTextToClipboard(url),
  }
}

const FILE_ICONS: [RegExp, typeof FileIcon][] = [
  [/\.(pdf|md|txt|docx?)$/i, FileText],
  [/\.(csv|tsv|xlsx?)$/i, FileSpreadsheet],
  [/\.(html?|json|jsx?|tsx?|css|ya?ml|toml|sh|py|rs)$/i, FileCode],
]

export function fileIconFor(name: string, mime?: string): typeof FileIcon {
  if (mime?.startsWith('image/')) return ImageIcon
  return FILE_ICONS.find(([pattern]) => pattern.test(name))?.[1] ?? FileIcon
}

/** "48 KB", in mono on the row. */
export function attachmentSize(size: number | null | undefined): string | null {
  return typeof size === 'number' ? formatSize(size) : null
}

/** One picture of the grid: an attachment, maybe still to load, and its source. */
export interface GridImage {
  key: string
  direction: 'sent' | 'received'
  /** Bytes in hand (the local parse). */
  image?: Attachment
  /** A stored original to load (the engine's list). */
  attachmentId?: string
  mime?: string
  name: string
  menu: readonly ItemMenuEntry[]
}

/**
 * "Images 4 · Sent · Received": square thumbnails, three across in the drawer
 * and four in a pane 400px or wider, each with a corner mark (↑ sent, ↓
 * received). A tile opens the viewer; its ⋮ holds the row menu.
 */
export function ImageGrid({
  images,
  sessionId,
  hidden = 0,
  onShowMore,
}: {
  images: readonly GridImage[]
  sessionId: string
  hidden?: number
  onShowMore?: () => void
}) {
  return (
    <div
      data-image-grid=""
      className="grid grid-cols-3 gap-1.5 px-2.5 pt-1 pb-1 @min-[25rem]/context:grid-cols-4"
    >
      {images.map((image) => (
        <ImageGridTile key={image.key} image={image} sessionId={sessionId} />
      ))}
      {hidden > 0 && onShowMore ? (
        <button
          type="button"
          aria-label={`Show ${hidden} more images`}
          onClick={onShowMore}
          className="flex aspect-square items-center justify-center rounded-sm bg-surface font-mono text-xs text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
        >
          +{hidden}
        </button>
      ) : null}
    </div>
  )
}

function ImageGridTile({
  image,
  sessionId,
}: {
  image: GridImage
  sessionId: string
}) {
  const storedId = image.image?.attachmentId ?? image.attachmentId
  const lazy = image.image
    ? isLazyImage(image.image, sessionId)
    : Boolean(image.attachmentId)
  const stored = useLazyAttachment(sessionId, storedId, lazy)
  const viewer = useImageViewer()
  const src =
    image.image?.dataUrl ??
    (stored.state.phase === 'ready' ? stored.state.dataUrl : undefined)
  const Mark = image.direction === 'sent' ? ArrowUp : ArrowDown
  const open = () => (src ? viewer.show() : stored.request())
  const menu: ItemMenuEntry[] = [
    { label: 'Open', icon: menuIcons.open, onSelect: open },
    ...image.menu,
  ]
  return (
    <div
      ref={lazy ? stored.ref : undefined}
      data-direction={image.direction}
      className="group/tile relative aspect-square min-w-0 overflow-hidden rounded-sm bg-surface"
    >
      <ImageThumbnailButton
        title={image.name}
        onClick={open}
        className="flex size-full items-center justify-center"
      >
        {src ? (
          <img
            src={src}
            alt=""
            className="size-full object-cover outline -outline-offset-1 outline-foreground/10"
          />
        ) : (
          <ImageIcon aria-hidden className="size-4 text-ink-faint" />
        )}
      </ImageThumbnailButton>
      <span
        role="img"
        aria-label={image.direction === 'sent' ? 'Sent' : 'Received'}
        className="pointer-events-none absolute bottom-1 left-1 flex size-5 items-center justify-center rounded-xs bg-background/80 text-ink"
      >
        <Mark aria-hidden className="size-4" />
      </span>
      <span
        className={cn(
          'absolute top-0.5 right-0.5 rounded-sm bg-background/80 opacity-0 group-hover/tile:opacity-100 group-focus-within/tile:opacity-100 pointer-coarse:opacity-100',
        )}
      >
        <ItemMenu label={`Actions for ${image.name}`} entries={menu} />
      </span>
      <ImageViewer
        open={viewer.open}
        onOpenChange={viewer.setOpen}
        src={src}
        alt={image.name}
        title={image.name}
      />
    </div>
  )
}

/** "Done · 14s", "Running · 1m 02s", "Failed" (§1.3.6 Subagents). */
export function subagentStatusText(
  status: string,
  startedAtMs: number | undefined,
  endedAtMs: number | undefined,
  now = Date.now(),
): string {
  const running = /running|working|queued|waiting|started/.test(status)
  const failed = /fail|error|cancel/.test(status)
  if (failed) return 'Failed'
  const span =
    startedAtMs && startedAtMs > 0
      ? Math.max(0, (endedAtMs ?? (running ? now : startedAtMs)) - startedAtMs)
      : null
  const word = running ? 'Running' : 'Done'
  return span === null || span === 0 ? word : `${word} · ${shortDuration(span)}`
}

/** `14s`, `1m 02s`, `1h 05m`. */
export function shortDuration(ms: number): string {
  const seconds = Math.round(ms / 1000)
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60)
    return `${minutes}m ${String(seconds % 60).padStart(2, '0')}s`
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, '0')}m`
}
