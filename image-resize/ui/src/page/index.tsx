/** The image-resize page: pick an image, resize it, compare it with the original. */

import { EmptyState, PageHeader, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { ImageIcon } from 'lucide-react'

export function ImageResizePage({ onRequestClose }: Partial<PageRenderProps>) {
  return (
    <PageShell>
      <PageHeader
        icon={<ImageIcon />}
        title="Image resize"
        description="Downscale a JPEG, PNG or WebP image and compare it with the original."
        onClose={onRequestClose}
      />
      <div className="imgr">
        <EmptyState
          title="No image yet"
          description="Images up to 64 MiB can be resized here and compared with the original."
        />
      </div>
    </PageShell>
  )
}
