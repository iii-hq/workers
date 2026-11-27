import { Markdown, Skeleton, StatusPanel } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useState } from 'react'
import type { ArtifactApi, GetResponse } from '../lib/api'
import { previewDocument, sandboxFor } from '../lib/preview'

interface ArtifactPreviewProps {
  api: ArtifactApi
  id: string
  version?: number
  /** `card` is the fixed-height chat preview; `page` fills its column. */
  size: 'card' | 'page'
}

type Load =
  | { state: 'loading' }
  | { state: 'ready'; data: GetResponse }
  | { state: 'error'; message: string }

/** One version of an artifact, rendered in isolation. */
export function ArtifactPreview({ api, id, version, size }: ArtifactPreviewProps) {
  const [load, setLoad] = useState<Load>({ state: 'loading' })

  useEffect(() => {
    let current = true
    setLoad({ state: 'loading' })
    api.get(id, version).then(
      (data) => current && setLoad({ state: 'ready', data }),
      (err) => current && setLoad({ state: 'error', message: errorMessage(err) }),
    )
    return () => {
      current = false
    }
  }, [api, id, version])

  const className = `artifact-ui-preview artifact-ui-preview-${size}`
  if (load.state === 'loading') {
    return (
      <div className={className} aria-busy="true">
        <Skeleton className="artifact-ui-preview-skeleton" />
      </div>
    )
  }
  if (load.state === 'error') {
    return (
      <StatusPanel
        variant="alert"
        className={className}
        headline="Could not load this artifact"
        detail={load.message}
      />
    )
  }
  const { artifact, content } = load.data
  if (artifact.format === 'markdown') {
    return (
      <div className={`${className} artifact-ui-preview-markdown`}>
        <Markdown>{content}</Markdown>
      </div>
    )
  }
  return (
    <iframe
      className={className}
      title={`${artifact.title} v${load.data.version}`}
      sandbox={sandboxFor(artifact.format)}
      srcDoc={previewDocument(artifact.format, content)}
      referrerPolicy="no-referrer"
    />
  )
}
