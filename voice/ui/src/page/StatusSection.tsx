/**
 * The voice page's status card: `voice::doctor` summary (stt/tts backend,
 * model, installed/loaded state) plus a "Download model" action that calls
 * `voice::models::download` and tracks live `voice::model-progress` events
 * bound through `host.iii.on` + `host.iii.registerTrigger`. Laid out as a
 * label/value grid that stacks in narrow panes.
 */

import { Badge, Button, type Host, StatusPanel } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { doctor, modelsDownload } from '../lib/client'
import { errorMessage } from '../lib/format'
import type { DoctorResponse, ModelProgressEvent } from '../lib/types'

const PROGRESS_FN = 'iii::voice-ui::model-progress'

type DoctorState = { phase: 'loading' } | { phase: 'ready'; data: DoctorResponse } | { phase: 'error'; message: string }

export function StatusSection({ host }: { host: Host }) {
  const [state, setState] = useState<DoctorState>({ phase: 'loading' })
  const [downloading, setDownloading] = useState(false)
  const [progress, setProgress] = useState<ModelProgressEvent | null>(null)
  const [downloadError, setDownloadError] = useState<string | null>(null)
  const [reloadToken, setReloadToken] = useState(0)

  // biome-ignore lint/correctness/useExhaustiveDependencies: reloadToken is a manual re-fetch trigger, not read in the body
  useEffect(() => {
    let cancelled = false
    doctor(host.iii)
      .then((data) => {
        if (!cancelled) setState({ phase: 'ready', data })
      })
      .catch((err: unknown) => {
        if (!cancelled) setState({ phase: 'error', message: errorMessage(err) })
      })
    return () => {
      cancelled = true
    }
  }, [host, reloadToken])

  useEffect(() => {
    const offHandler = host.iii.on<ModelProgressEvent>(PROGRESS_FN, (event) => {
      setProgress(event)
      if (event.done) {
        setDownloading(false)
        if (!event.error) setReloadToken((n) => n + 1)
      }
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'voice::model-progress',
      function_id: `${PROGRESS_FN}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host])

  const download = () => {
    if (downloading) return
    setDownloading(true)
    setDownloadError(null)
    setProgress(null)
    modelsDownload(host.iii, {})
      .then(() => setReloadToken((n) => n + 1))
      .catch((err: unknown) => {
        setDownloading(false)
        setDownloadError(errorMessage(err))
      })
  }

  if (state.phase === 'loading') {
    return <div className="voice-section voice-note">checking voice::doctor…</div>
  }
  if (state.phase === 'error') {
    return (
      <StatusPanel variant="alert" headline="voice::doctor failed" detail={state.message} className="voice-section" />
    )
  }

  const { stt, tts, sessions, version } = state.data
  const pct =
    progress && progress.total_bytes > 0 ? Math.round((progress.received_bytes / progress.total_bytes) * 100) : null
  const sttState = !stt.installed ? 'not installed' : stt.loaded ? 'loaded' : 'installed'
  const finalProgress = progress && progress.id === stt.final_model && !progress.done ? progress : null
  const finalPct =
    finalProgress && finalProgress.total_bytes > 0
      ? Math.round((finalProgress.received_bytes / finalProgress.total_bytes) * 100)
      : null
  const finalLabel =
    stt.final_state === 'downloading' && finalPct !== null
      ? `downloading ${finalPct}%`
      : stt.final_state === 'downloading'
        ? 'downloading'
        : stt.final_state === 'missing'
          ? 'not downloaded yet'
          : stt.final_state
  const downloadFinal = () => {
    if (downloading) return
    setDownloading(true)
    setDownloadError(null)
    setProgress(null)
    modelsDownload(host.iii, { id: stt.final_model })
      .then(() => setReloadToken((n) => n + 1))
      .catch((err: unknown) => {
        setDownloading(false)
        setDownloadError(errorMessage(err))
      })
  }

  return (
    <section className="voice-section voice-status">
      <h3 className="voice-section-title">Status</h3>
      <dl className="voice-status-grid">
        <dt className="voice-status-label">Speech to text</dt>
        <dd className="voice-status-value">
          <Badge variant={stt.installed ? 'ok' : 'warn'}>{stt.backend}</Badge>
          <span className="voice-status-detail" title={stt.model}>
            {stt.model}
          </span>
          <span className="voice-status-meta">{sttState}</span>
        </dd>
        {stt.backend === 'local' && stt.final_state !== 'off' ? (
          <>
            <dt className="voice-status-label">Final text</dt>
            <dd className="voice-status-value">
              <Badge variant={stt.final_state === 'loaded' || stt.final_state === 'installed' ? 'ok' : 'default'}>
                {stt.final_state === 'unknown' ? 'unknown' : 'second pass'}
              </Badge>
              <span className="voice-status-detail" title={stt.final_model}>
                {stt.final_model}
              </span>
              <span className="voice-status-meta">{finalLabel}</span>
              {stt.final_state === 'missing' ? (
                <Button variant="ghost" size="sm" onClick={downloadFinal} disabled={downloading}>
                  {downloading ? 'downloading…' : 'Download (660 MB)'}
                </Button>
              ) : null}
            </dd>
          </>
        ) : null}
        <dt className="voice-status-label">Text to speech</dt>
        <dd className="voice-status-value">
          <Badge variant={tts.available ? 'ok' : 'default'}>{tts.backend}</Badge>
          {tts.command ? <span className="voice-status-detail">{tts.command}</span> : null}
          {!tts.available && tts.backend !== 'off' ? <span className="voice-status-meta">command missing</span> : null}
        </dd>
        <dt className="voice-status-label">Sessions</dt>
        <dd className="voice-status-value">
          <span className="voice-status-detail">{sessions} active</span>
        </dd>
      </dl>
      {!stt.installed ? (
        <div className="voice-status-download">
          <Button variant="primary" size="sm" onClick={download} disabled={downloading}>
            {downloading ? 'downloading…' : 'Download model'}
          </Button>
          {pct !== null ? (
            <span className="voice-status-progress">
              {pct}%{progress?.file ? ` · ${progress.file}` : ''}
            </span>
          ) : null}
          {downloadError ? <span className="voice-status-progress warn">{downloadError}</span> : null}
        </div>
      ) : null}
      <div className="voice-status-version">voice v{version}</div>
    </section>
  )
}
