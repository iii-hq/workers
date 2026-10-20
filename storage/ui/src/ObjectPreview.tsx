/**
 * The selected object, shown inline when the browser can: an image (which
 * opens the console's zoomable viewer) or a video with the native controls.
 *
 * Both load from a presigned GET URL, so the bytes never pass through the
 * worker and a video seeks with Range requests (the local server answers
 * them). A bucket that cannot sign URLs (`direct_downloads: false`) shows an
 * image of at most 10 MiB read through the worker; anything else offers the
 * download instead. A failed load presigns once more — a local worker
 * signs with a per-process key, so a restart invalidates older links —
 * before it gives up and says why.
 */

import {
  Button,
  type Host,
  ImageThumbnailButton,
  ImageViewer,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { Download } from 'lucide-react'
import { useEffect, useState } from 'react'
import { errorMessage } from './lib/errors'
import {
  type BucketSummary,
  INLINE_CAP,
  mixedContent,
  type PreviewKind,
  previewKind,
  readThroughWorker,
} from './lib/objects'
import { leafName } from './widgets'

export interface PreviewMetadata {
  content_type: string
  etag: string
  size: number
}

/** Seconds a preview link stays valid; the page re-presigns on a failed load. */
const PREVIEW_TTL = 3600

function failureText(kind: PreviewKind, url: string | null): string {
  if (url && typeof location !== 'undefined' && mixedContent(location.protocol, url)) {
    return 'The console is served over https and this link is http, so the browser blocks it. Download the file instead.'
  }
  return kind === 'video'
    ? 'The browser could not play this video: its format or codec may be unsupported, or the link is unreachable from this browser.'
    : 'The browser could not show this image: its format may be unsupported, or the link is unreachable from this browser.'
}

export function ObjectPreview({
  host,
  bucket,
  objectKey,
  metadata,
  busy,
  onDownload,
}: {
  host: Host
  bucket: BucketSummary
  objectKey: string
  metadata: PreviewMetadata
  busy: boolean
  onDownload: () => void
}) {
  const kind = previewKind(metadata.content_type, objectKey)
  const direct = bucket.direct_downloads !== false
  const relayed = !direct && kind === 'image' && metadata.size <= INLINE_CAP
  const [src, setSrc] = useState<string | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  const [viewerOpen, setViewerOpen] = useState(false)
  const name = leafName(objectKey)

  useEffect(() => {
    setAttempt(0)
  }, [bucket.name, objectKey, metadata.etag])

  useEffect(() => {
    if (!kind || (!direct && !relayed)) return
    let alive = true
    let blobUrl: string | null = null
    setSrc(null)
    setFailure(null)
    const load: Promise<string> = direct
      ? host.iii
          .trigger<{ url: string }>('storage::presignUrl', {
            bucket: bucket.name,
            key: objectKey,
            method: 'GET',
            expires_in_seconds: PREVIEW_TTL,
          })
          .then((signed) => signed.url)
      : readThroughWorker(host, bucket.name, objectKey, metadata.size, { contentType: metadata.content_type }).then(
          (blob) => URL.createObjectURL(blob),
        )
    load
      .then((url) => {
        if (!direct) blobUrl = url
        if (!alive) {
          if (blobUrl) URL.revokeObjectURL(blobUrl)
          return
        }
        setSrc(url)
      })
      .catch((error: unknown) => {
        if (alive) setFailure(errorMessage(error))
      })
    return () => {
      alive = false
      if (blobUrl) URL.revokeObjectURL(blobUrl)
    }
  }, [host, bucket.name, objectKey, metadata.etag, metadata.size, metadata.content_type, kind, direct, relayed, attempt])

  if (!kind) return null

  const onMediaError = () => {
    if (direct && attempt === 0) {
      setAttempt(1)
      return
    }
    setFailure(failureText(kind, src))
  }

  const download = (
    <Button variant="ghost" size="sm" disabled={busy} onClick={onDownload}>
      <Download /> download
    </Button>
  )

  if (!direct && !relayed) {
    return (
      <div className="storage-ui-preview">
        <StatusPanel
          className="storage-ui-preview-status"
          headline="No inline preview"
          detail={
            kind === 'video'
              ? 'This bucket cannot sign download links, so the video cannot stream here. Download it to watch it.'
              : `This bucket cannot sign download links, and the image is larger than ${formatBytes(INLINE_CAP)}. Download it to see it.`
          }
          action={download}
        />
      </div>
    )
  }

  if (failure) {
    return (
      <div className="storage-ui-preview">
        <StatusPanel
          className="storage-ui-preview-status"
          variant="warn"
          headline="Preview unavailable"
          detail={failure}
          action={download}
        />
      </div>
    )
  }

  if (!src) {
    return (
      <div className="storage-ui-preview" aria-label="Loading preview">
        <Skeleton className="storage-ui-preview-skeleton" />
      </div>
    )
  }

  return (
    <div className="storage-ui-preview">
      {kind === 'image' ? (
        <>
          <ImageThumbnailButton title={name} className="storage-ui-preview-open" onClick={() => setViewerOpen(true)}>
            <img className="storage-ui-preview-media" src={src} alt={name} onError={onMediaError} />
          </ImageThumbnailButton>
          <ImageViewer
            open={viewerOpen}
            onOpenChange={setViewerOpen}
            src={src}
            alt={name}
            title={objectKey}
            description={`${formatBytes(metadata.size)} · ${metadata.content_type}`}
          />
        </>
      ) : (
        // biome-ignore lint/a11y/useMediaCaption: objects in a bucket carry no caption track
        <video
          key={src}
          className="storage-ui-preview-media"
          src={src}
          controls
          preload="metadata"
          playsInline
          aria-label={name}
          onError={onMediaError}
        />
      )}
    </div>
  )
}
