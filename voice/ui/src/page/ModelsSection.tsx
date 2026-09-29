/**
 * The Models section: every speech model the worker knows, what each one
 * is for, whether it is on disk, and the actions: download, remove, use
 * it. Rows, not a table, so a narrow pane still reads. Choosing a model
 * writes the same `voice` configuration the Settings form edits.
 */

import { Button, Chip, ConfirmDialog, EmptyState, type Host, StatusDot } from '@iii-dev/console-ui'
import { Check, Mic, Trash2, Volume2 } from 'lucide-react'
import { useState } from 'react'
import { modelsDownload, modelsRemove } from '../lib/client'
import { patchConfig } from '../lib/config'
import { ModelDownload } from '../lib/ModelDownload'
import { modelUseLabel, splitModelsByPurpose, useModelConfig } from '../lib/models'
import { PiperLanguageFilter } from '../lib/PiperVoicePicker'
import { filterPiperVoices } from '../lib/piper-languages'
import type { ProgressById } from '../lib/progress'
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
  const [piperLanguage, setPiperLanguage] = useState('')
  const { listening, reading } = splitModelsByPurpose(models?.models ?? [])
  const visibleReading = piperLanguage.trim()
    ? filterPiperVoices(reading, piperLanguage, typeof navigator === 'undefined' ? 'en' : navigator.language)
    : reading.filter((m) => m.installed)

  const roleOf = (m: ModelInfo): 'transcripts' | 'live words' | 'read aloud' | null => {
    if (m.kind === 'piper_onnx' && report.tts.backend === 'piper' && report.tts.model === m.id) return 'read aloud'
    if (report.stt.backend === 'whisper_cpp' && m.kind === 'whisper_ggml' && m.id === report.stt.model)
      return 'transcripts'
    if (report.stt.backend === 'local' && m.id === report.stt.final_model) return 'transcripts'
    if (m.id === report.stt.live_model) return 'live words'
    return null
  }

  const pickModel = (m: ModelInfo) => run(
    m.id,
    patchConfig(host.iii, (current) => useModelConfig(current, m)),
    `${m.name} is selected.${m.installed ? '' : ' Download it before use.'}`,
  )

  const removingDescription = (() => {
    if (!removing) return undefined
    const reuse = removing.kind === 'piper_onnx'
      ? 'Download it again before reading aloud with Piper.'
      : removing.kind === 'whisper_ggml'
      ? 'Download it again before using it with whisper.cpp.'
      : roleOf(removing)
      ? 'It is in use; it downloads again on the next dictation.'
      : 'It downloads again if you use it later.'
    return `${formatBytes(removing.size_bytes)} will be deleted from the models directory. ${reuse}`
  })()

  const installedCount = (group: readonly ModelInfo[]) => models ? (
    <Chip tone="neutral">{group.filter((m) => m.installed).length}/{group.length} installed</Chip>
  ) : null

  // The same actions, progress and removal confirmation serve both categories.
  const renderModels = (group: readonly ModelInfo[], label: string) => (
          <ul className="voice-model-list" aria-label={label}>
            {group.map((m) => {
              const downloading = Boolean(progress[m.id] && !progress[m.id].done) || busy === `download:${m.id}`
              const role = roleOf(m)
              const purpose =
                m.kind === 'piper_onnx' ? 'neural read aloud · requires Piper'
                  : m.kind === 'whisper_ggml' ? 'multilingual transcripts · requires whisper-cli'
                  : m.kind === 'offline_nemo_transducer' ? 'accurate transcripts, punctuation' : 'live words while speaking'
              return (
                <li key={m.id} className="voice-model-row">
                  <span className="voice-model-indicator">
                    <StatusDot tone={m.installed ? 'accent' : 'ink'} pulse={downloading} />
                  </span>
                  <div className="voice-model-main">
                    <span className="voice-fact-line">
                      <span className="voice-strong">{m.name}</span>
                      {role ? <Chip tone="accent">in use for {role}</Chip> : null}
                    </span>
                    <span className="voice-sub">
                      {purpose} · {m.languages.join(', ')} · {formatBytes(m.size_bytes)}
                    </span>
                    <span className="voice-sub">
                      {m.author ? `by ${m.author}` : ''}
                      {m.author && m.license ? ' · ' : ''}
                      {m.source ? (
                        <a className="voice-link" href={m.source} target="_blank" rel="noreferrer">
                          {m.license ?? 'model card'}
                        </a>
                      ) : (
                        (m.license ?? '')
                      )}
                    </span>
                    <span className="voice-mono voice-sub">{m.id}</span>
                  </div>
                  <ModelDownload
                    className="voice-model-footer"
                    model={m}
                    progress={progress[m.id]}
                    busy={busy === `download:${m.id}`}
                    disabled={busy !== null}
                    onDownload={() => run(`download:${m.id}`,
                      modelsDownload(host.iii, { id: m.id }), `${m.name} is installed.`)}
                    actions={
                      <>
                        {!role ? (
                          <Button variant={m.installed ? 'primary' : 'ghost'} size="sm"
                            className="voice-model-use" aria-label={`${modelUseLabel(m)}: ${m.name}`}
                            disabled={busy !== null || downloading} onClick={() => pickModel(m)}>
                            <Check />
                            {modelUseLabel(m)}
                          </Button>
                        ) : null}
                        {m.installed ? (
                          <Button variant="ghost" size="sm" className="voice-model-remove"
                            aria-label={`Remove ${m.name}`} disabled={busy !== null || downloading}
                            onClick={() => setRemoving(m)}>
                            <Trash2 />
                            Remove
                          </Button>
                        ) : null}
                      </>
                    }
                  />
                </li>
              )
            })}
          </ul>
  )

  return (
    <>
      <SectionCard
        title={<span className="voice-fact-line"><Mic size={16} />Listening models · Speech to text</span>}
        actions={installedCount(listening)}
      >
        <p className="voice-note">
          These models listen to your microphone or a recording and turn speech into written text.
          They power Dictate and Transcribe; they do not generate the voice that reads replies.
          Transcription writes what you said — translation into another language is not configured here.
        </p>
        <p className="voice-note">
          Zipformer provides live words while you speak. Parakeet refines English transcripts;
          Whisper handles multilingual final transcripts and requires whisper-cli.
          Choose a model for live dictation or transcription, then download it if missing.
          This does not change your read-aloud voice.
        </p>
        {!models ? <LoadingRows rows={2} /> : listening.length > 0
          ? renderModels(listening, 'Listening models')
          : <EmptyState icon={Mic} title="No listening models available"
              description="The worker did not return any local speech-to-text models." />}
      </SectionCard>

      <SectionCard
        title={<span className="voice-fact-line"><Volume2 size={16} />Reading models · Text to speech</span>}
        actions={installedCount(reading)}
      >
        <p className="voice-note">
          These voices turn written text into speech. Piper speaks selected passages and replies
          through Read aloud and Voice chat, with audio played only in your browser.
          A reading voice does not listen to the microphone or transcribe your speech.
          Choosing one changes only the read-aloud voice, not your listening models.
        </p>
        <PiperLanguageFilter models={reading} value={piperLanguage}
          onChange={setPiperLanguage} disabled={models === null || busy !== null} />
        {!piperLanguage.trim() ? <p className="voice-note">
          Installed reading voices are shown below. Enter your language above to find and download more voices.
        </p> : null}
        {!models ? <LoadingRows rows={2} /> : visibleReading.length > 0
          ? renderModels(visibleReading, 'Reading models')
          : <EmptyState icon={Volume2}
              title={piperLanguage.trim() ? 'No reading voices match this language' : 'No reading voices downloaded'}
              description={piperLanguage.trim()
                ? 'Try another language name or locale code. Your listening models are unaffected.'
                : 'Enter your language above, choose a Piper voice, then click Download. Nothing downloads automatically.'} />}
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
