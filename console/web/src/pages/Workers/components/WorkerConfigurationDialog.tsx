import { useEffect } from 'react'
import { hashForWorkersConfiguration } from '@/hooks/use-hash-route'

interface WorkerConfigurationDialogProps {
  configurationId: string | null
  onClose: () => void
}

/**
 * Compatibility bridge for injected pages that still request a local worker
 * dialog. Configuration now lives in the console-owned settings modal. New
 * pages should declare `configurationId` in their page registration instead.
 */
export function WorkerConfigurationDialog({
  configurationId,
  onClose,
}: WorkerConfigurationDialogProps) {
  useEffect(() => {
    if (!configurationId) return
    window.location.hash = hashForWorkersConfiguration(configurationId)
    onClose()
  }, [configurationId, onClose])

  return null
}
