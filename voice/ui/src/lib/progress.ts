/**
 * `useModelProgress(host)` keeps the latest `voice::on-model-progress` event
 * per model id, received through `host.iii.on` + `host.iii.observe`, so any
 * section can show a download as it happens without polling.
 */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useId, useState } from 'react'
import type { ModelProgressEvent } from './types'

const PROGRESS_FN = 'iii::voice-ui::model-progress'

export type ProgressById = Readonly<Record<string, ModelProgressEvent>>

export function useModelProgress(host: Host, onDone?: (event: ModelProgressEvent) => void): ProgressById {
  const [progress, setProgress] = useState<ProgressById>({})
  const instance = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    const localFn = `${PROGRESS_FN}::${instance}`
    const offHandler = host.iii.on<ModelProgressEvent>(localFn, (event) => {
      setProgress((prev) => ({ ...prev, [event.id]: event }))
      if (event.done) onDone?.(event)
    })
    let offObserver: (() => void) | null = null
    try {
      offObserver = host.iii.observe('voice::on-model-progress', `${localFn}::${host.iii.browserId}`)
    } catch {
      offObserver = null
    }
    return () => {
      offObserver?.()
      offHandler()
    }
  }, [host, instance, onDone])

  return progress
}

export function percent(event: ModelProgressEvent | undefined): number | null {
  if (!event || event.done || event.total_bytes <= 0) return null
  return Math.max(0, Math.min(100, Math.round((event.received_bytes / event.total_bytes) * 100)))
}
