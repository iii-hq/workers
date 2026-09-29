import {
  Dialog,
  DialogContent,
  DialogTitle,
  EmptyState,
  type Host,
  IconButton,
  ImageViewer,
  List,
  ListItem,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage, formatBytes, formatRelative } from '@iii-dev/console-ui/format'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { Film, Image, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { MEDIA_CHANGED, type Media, type SimApi } from '../lib/api'

/**
 * Screenshots and recordings of one simulator, newest first, live on
 * `ios-simulator::media-changed`. Opening an item reads the file from the
 * worker in slices and shows it in the shared image viewer or a video
 * dialog; the blob URL is released when it closes.
 */
export function MediaPanel({
  host,
  api,
  udid,
  onError,
}: {
  host: Host
  api: SimApi
  udid: string
  onError: (message: string) => void
}) {
  const { data, loading, error } = useWorkerLive({
    iii: host.iii,
    triggers: [{ type: MEDIA_CHANGED, config: { tenant: api.tenant, udid } }],
    fetch: () => api.media(udid),
    handlerId: 'iii::ios-simulator-ui::media',
  })
  const [open, setOpen] = useState<{ item: Media; url: string | null } | null>(null)

  useEffect(() => {
    const url = open?.url
    return () => {
      if (url) URL.revokeObjectURL(url)
    }
  }, [open?.url])

  const show = async (item: Media) => {
    setOpen({ item, url: null })
    try {
      const url = await api.mediaUrl(item)
      setOpen((current) => {
        if (current?.item.name === item.name) return { item, url }
        URL.revokeObjectURL(url)
        return current
      })
    } catch (err) {
      setOpen(null)
      onError(errorMessage(err))
    }
  }

  const items = useMemo(() => data ?? [], [data])

  if (error) return <StatusPanel variant="alert" role="alert" headline="Could not list media" detail={error} />
  if (loading && items.length === 0) {
    return (
      <div className="ios-ui-skeletons" aria-busy="true">
        <Skeleton className="ios-ui-skeleton-row" />
        <Skeleton className="ios-ui-skeleton-row" />
      </div>
    )
  }
  if (items.length === 0) {
    return (
      <EmptyState
        icon={Image}
        title="No screenshots or recordings"
        description="Use the camera and record buttons above; files land in this tenant's data folder."
      />
    )
  }

  return (
    <div className="ios-ui-media">
      <List>
        {items.map((item) => (
          <ListItem
            key={item.name}
            as="div"
            role="button"
            tabIndex={0}
            leading={item.kind === 'screenshot' ? <Image size={16} aria-hidden /> : <Film size={16} aria-hidden />}
            label={item.kind === 'screenshot' ? 'Screenshot' : 'Recording'}
            description={`${formatRelative(item.created_ms)} ago · ${formatBytes(item.bytes)}`}
            onClick={() => void show(item)}
            onKeyDown={(e) => {
              if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
                e.preventDefault()
                void show(item)
              }
            }}
            trailing={
              <IconButton
                label="Delete"
                onClick={(e) => {
                  e.stopPropagation()
                  void api.deleteMedia(item.name).catch((err) => onError(errorMessage(err)))
                }}
              >
                <Trash2 size={16} aria-hidden />
              </IconButton>
            }
          />
        ))}
      </List>

      {open?.item.kind === 'screenshot' ? (
        <ImageViewer
          open
          onOpenChange={(next) => !next && setOpen(null)}
          src={open.url}
          alt="Simulator screenshot"
          title={open.item.name}
          description={formatBytes(open.item.bytes)}
          unavailableReason={open.url ? undefined : 'Loading…'}
        />
      ) : null}
      {open?.item.kind === 'recording' ? (
        <Dialog open onOpenChange={(next) => !next && setOpen(null)}>
          <DialogContent className="ios-ui-video-dialog">
            <DialogTitle>Recording · {formatBytes(open.item.bytes)}</DialogTitle>
            {open.url ? (
              // biome-ignore lint/a11y/useMediaCaption: a screen recording has no spoken track
              <video className="ios-ui-video" src={open.url} controls autoPlay playsInline />
            ) : (
              <Skeleton className="ios-ui-video-skeleton" />
            )}
          </DialogContent>
        </Dialog>
      ) : null}
    </div>
  )
}
