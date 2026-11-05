import { ShadcnButton } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { Download } from 'lucide-react'
import type { ReactNode } from 'react'
import { StatusBadge } from '../ui/status'
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
    ? <StatusBadge tone="busy">{pct === null ? 'Downloading…' : `Downloading ${pct}%`}</StatusBadge>
    : model.installed
      ? <StatusBadge tone="ok">Installed</StatusBadge>
      : <StatusBadge tone="off">Not downloaded</StatusBadge>
  const canDownload = !downloading && !model.installed

  return (
    <span className={className ? `voice-model-download ${className}` : 'voice-model-download'}>
      <span className="voice-model-status" role="status">{status}</span>
      {actions || canDownload ? (
        <span className="voice-model-actions">
          {actions}
          {canDownload ? (
            <ShadcnButton variant="secondary" size="sm" disabled={disabled} onClick={onDownload}
              aria-label={`Download ${model.name}`}>
              <Download />
              Download {formatBytes(model.size_bytes)}
            </ShadcnButton>
          ) : null}
        </span>
      ) : null}
    </span>
  )
}
