import * as ConsoleUi from '@iii-dev/console-ui'
import { useState } from 'react'

/** A picture in the editor body or a review row; it opens the shared viewer
    where the running console provides one. */
export function ImagePreview({
  src,
  name,
  description,
  inline = false,
}: {
  src: string
  name: string
  description?: string
  /** Inside a flowing list (review rows): cap the height instead of filling a pane. */
  inline?: boolean
}) {
  const [open, setOpen] = useState(false)
  const ThumbnailButton = ConsoleUi.ImageThumbnailButton
  const Viewer = ConsoleUi.ImageViewer
  const wrapClass = inline ? 'shui-image-wrap shui-image-wrap--inline' : 'shui-image-wrap'
  if (!ThumbnailButton || !Viewer) {
    return (
      <div className={wrapClass}>
        <img src={src} alt={name} />
      </div>
    )
  }
  return (
    <div className={wrapClass}>
      <ThumbnailButton
        title={name}
        onClick={() => setOpen(true)}
        className="shui-image-thumb"
      >
        <img src={src} alt={name} />
      </ThumbnailButton>
      <Viewer
        open={open}
        onOpenChange={setOpen}
        src={src}
        alt={name}
        title={name}
        description={description}
      />
    </div>
  )
}
