import { Button, Chip } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { Download } from 'lucide-react'
import type { ReactNode } from 'react'
import { percent } from './progress'
import type { ModelInfo, ModelProgressEvent } from './types'

/** Shared by the overview, catalog and Settings draft. Never starts a download on selection. */
export function ModelDownload({
  model, progress, busy = false, disabled = false, onDownload, actions, className,
}: {
  model: ModelInfo
  progress?: ModelProgressEvent
  busy?: boolean
  disabled?: boolean
  onDownload: () => void
  /** Related actions share a spaced group, separate from the status badge. */
  actions?: ReactNode
  className?: string
}) {
  const downloading = busy || Boolean(progress && !progress.done)
  const pct = percent(progress)
  const status = downloading
    ? <Chip tone="accent">{pct === null ? 'Downloading…' : `Downloading ${pct}%`}</Chip>
    : model.installed
      ? <Chip tone="success">installed</Chip>
      : <Chip tone="warning">not downloaded</Chip>
  const canDownload = !downloading && !model.installed

  return (
    <span className={className ? `voice-model-download ${className}` : 'voice-model-download'}>
      <span className="voice-model-status" role="status">{status}</span>
      {actions || canDownload ? (
        <span className="voice-model-actions">
          {actions}
          {canDownload ? (
            <Button variant="primary" size="sm" disabled={disabled} onClick={onDownload}
              aria-label={`Download ${model.name}`}>
              <Download />
              Download {formatBytes(model.size_bytes)}
            </Button>
          ) : null}
        </span>
      ) : null}
    </span>
  )
}
