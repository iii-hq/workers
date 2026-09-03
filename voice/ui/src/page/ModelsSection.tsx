/**
 * The Models section: every speech model the worker knows, what each one
 * is for, whether it is on disk, and the actions: download, remove, use
 * it. Rows, not a table, so a narrow pane still reads. Choosing a model
 * writes the same `voice` configuration the Settings form edits.
 */

import { Button, Chip, ConfirmDialog, type Host, StatusDot } from '@iii-dev/console-ui'
import { useState } from 'react'
import { modelsDownload, modelsRemove } from '../lib/client'
import { patchConfig, setPath } from '../lib/config'
import { type ProgressById, percent } from '../lib/progress'
import type { DoctorResponse, ModelInfo, ModelsListResponse } from '../lib/types'
import type { Notice } from './Overview'
import { Fact, Facts, formatBytes, LoadingRows, SectionCard, useBusyAction } from './shared'

export function ModelsSection({
  host,
  report,
  models,
  progress,
  onNotice,
  onChanged,
}: {
  host: Host
  report: DoctorResponse
  models: ModelsListResponse | null
  progress: ProgressById
  onNotice: (notice: Notice) => void
  onChanged: () => void
}) {
  const [busy, run] = useBusyAction(onNotice, onChanged)
  const [removing, setRemoving] = useState<ModelInfo | null>(null)

  const roleOf = (m: ModelInfo): 'transcripts' | 'live words' | null => {
    if (m.id === report.stt.final_model) return 'transcripts'
    if (m.id === report.stt.model) return 'live words'
    return null
  }

  const modelActions = (m: ModelInfo, downloading: boolean, pct: number | null, role: ReturnType<typeof roleOf>) => {
    if (downloading) return <Chip tone="accent">{pct === null ? 'downloading' : `${pct}%`}</Chip>
    if (m.installed) {
      return (
        <>
          {role ? null : (
            <Button variant="ghost" size="sm" onClick={() => pickModel(m)} disabled={busy !== null}>
              Use
            </Button>
          )}
          <Button variant="ghost" size="sm" onClick={() => setRemoving(m)} disabled={busy !== null}>
            Remove
          </Button>
        </>
      )
    }
    return (
      <Button
        variant="primary"
        size="sm"
        onClick={() => run(m.id, modelsDownload(host.iii, { id: m.id }), `${m.id} is installed.`)}
        disabled={busy !== null}
      >
        Download
      </Button>
    )
  }

  const pickModel = (m: ModelInfo) => {
    const path = m.kind === 'offline_nemo_transducer' ? ['stt', 'final_model'] : ['stt', 'model']
    run(
      m.id,
      patchConfig(host.iii, (current) => setPath(current, path, m.id)),
      `${m.id} is now in use.`,
    )
  }

  const removingDescription = (() => {
    if (!removing) return undefined
    const reuse = roleOf(removing)
      ? 'It is in use; it downloads again on the next dictation.'
      : 'It downloads again if you use it later.'
    return `${formatBytes(removing.size_bytes)} will be deleted from the models directory. ${reuse}`
  })()

  return (
    <>
      <SectionCard
        title="Speech models"
        actions={
          models ? (
            <Chip tone="neutral">{`${models.models.filter((m) => m.installed).length}/${models.models.length} installed`}</Chip>
          ) : null
        }
      >
        <p className="voice-note">
          Transcripts come from an accurate model that re-decodes each sentence after you pause. Live words while you
          speak come from a small streaming model. Both download once into the models directory.
        </p>
        {!models ? (
          <LoadingRows rows={2} />
        ) : (
          <ul className="voice-model-list">
            {models.models.map((m) => {
              const pct = percent(progress[m.id])
              const downloading = pct !== null || (busy === m.id && !m.installed)
              const role = roleOf(m)
              const purpose =
                m.kind === 'offline_nemo_transducer' ? 'accurate transcripts, punctuation' : 'live words while speaking'
              return (
                <li key={m.id} className="voice-model-row">
                  <StatusDot tone={m.installed ? 'accent' : 'ink'} pulse={downloading} />
                  <div className="voice-model-main">
                    <span className="voice-fact-line">
                      <span className="voice-strong">{m.name}</span>
                      {role ? <Chip tone="accent">in use for {role}</Chip> : null}
                    </span>
                    <span className="voice-sub">
                      {purpose} · {m.languages.join(', ')} · {formatBytes(m.size_bytes)}
                      {m.license ? ` · ${m.license}` : ''}
                    </span>
                    <span className="voice-mono voice-sub">{m.id}</span>
                  </div>
                  <div className="voice-model-side">{modelActions(m, downloading, pct, role)}</div>
                </li>
              )
            })}
          </ul>
        )}
      </SectionCard>
      <SectionCard title="Storage">
        <Facts>
          <Fact label="Directory">
            <span className="voice-mono voice-wrap">{models?.models_dir ?? report.stt.models_dir}</span>
          </Fact>
          <Fact label="Verification">
            <span className="voice-sub">
              Every file is checked against its SHA-256 before it is used; a failed download is discarded.
            </span>
          </Fact>
        </Facts>
      </SectionCard>
      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(open) => {
          if (!open) setRemoving(null)
        }}
        title={removing ? `Remove ${removing.name}?` : 'Remove model?'}
        description={removingDescription}
        confirmLabel="Remove"
        onConfirm={() => {
          if (!removing) return
          const target = removing
          setRemoving(null)
          run(target.id, modelsRemove(host.iii, { id: target.id }), `${target.id} was removed.`)
        }}
      />
    </>
  )
}
