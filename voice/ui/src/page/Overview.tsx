/**
 * The Overview section: what the worker does right now, in the terms a
 * person chooses. Speech to text (which model, is it here, pick another),
 * read aloud (which engine), open dictation sessions, and the door to the
 * full configuration. Every choice here writes the same `voice`
 * configuration the Settings form edits.
 */

import {
  type Host,
  ShadcnButton,
  ShadcnTable,
  ShadcnTableBody,
  ShadcnTableCell,
  ShadcnTableHead,
  ShadcnTableHeader,
  ShadcnTableRow,
  StatusDot,
} from '@iii-dev/console-ui'
import { Mic, Settings2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { modelsDownload } from '../lib/client'
import { NONE, patchConfig, readConfig, setPath, stringAt } from '../lib/config'
import { ModelDownload } from '../lib/ModelDownload'
import { modelOptions } from '../lib/models'
import { PiperVoicePicker } from '../lib/PiperVoicePicker'
import type { ProgressById } from '../lib/progress'
import { routerModelOptions, useRouterSpeechModels } from '../lib/router'
import type { DictationListEntry, DoctorResponse, ModelInfo, ModelsListResponse } from '../lib/types'
import { OptionSelect } from '../ui/option-select'
import { StatusBadge } from '../ui/status'
import { Fact, Facts, formatDuration, PanelEmpty, SectionCard, useBusyAction } from './shared'

export type Notice = { kind: 'error' | 'success'; text: string } | null

export function Overview({
  host,
  report,
  models,
  sessions,
  progress,
  onDictate,
  onConfigure,
  onNotice,
  onChanged,
}: {
  host: Host
  report: DoctorResponse
  models: ModelsListResponse | null
  sessions: readonly DictationListEntry[]
  progress: ProgressById
  onDictate: () => void
  onConfigure: () => void
  onNotice: (notice: Notice) => void
  onChanged: () => void
}) {
  const { stt, tts } = report
  const [busy, run] = useBusyAction(onNotice, onChanged)
  const offline = (models?.models ?? []).filter((m) => m.kind === 'offline_nemo_transducer')
  const accurate: ModelInfo | undefined = offline.find((m) => m.id === stt.final_model)
  const live: ModelInfo | undefined = models?.models.find((m) => m.id === stt.live_model)
  const streaming = (models?.models ?? []).filter((m) => m.kind === 'streaming_transducer')
  const whisper = (models?.models ?? []).filter((m) => m.kind === 'whisper_ggml')
  const whisperModel = whisper.find((m) => m.id === stt.model)
  const routerStt = useRouterSpeechModels(host.iii, 'stt', stt.backend === 'router')
  const routerTts = useRouterSpeechModels(host.iii, 'tts', tts.backend === 'router')
  const [routerTtsModel, setRouterTtsModel] = useState('')
  useEffect(() => {
    if (tts.backend !== 'router') return
    let cancelled = false
    readConfig(host.iii)
      .then((value) => {
        if (!cancelled) setRouterTtsModel(stringAt(value, ['tts', 'router', 'model']))
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [host.iii, tts.backend])

  const choose = (path: readonly string[], next: string, label: string) =>
    run(
      'config',
      patchConfig(host.iii, (current) => setPath(current, path, next)),
      label,
    )

  const download = (id: string) => run(id, modelsDownload(host.iii, { id }), `${id} is installed and ready.`)

  const liveWordsFact = (
    <Fact
      label="Live words"
      hint={<>Shows words as you speak and decides where a sentence ends; it always runs on this machine.</>}
    >
      <span className="voice-choice">
        <OptionSelect
          aria-label="live speech model"
          value={stt.live_model}
          disabled={busy !== null || models === null}
          onChange={(next) => choose(['stt', 'model'], next, `Live words use ${next}.`)}
          options={modelOptions(streaming, stt.live_model)}
        />
        {live ? (
          <ModelDownload
            model={live}
            progress={progress[live.id]}
            busy={busy === live.id}
            disabled={busy !== null}
            onDownload={() => download(live.id)}
          />
        ) : models ? (
          <StatusBadge tone="warn">Not in the catalog</StatusBadge>
        ) : null}
      </span>
    </Fact>
  )

  const providerOf = (model: string) => model.split('::')[0] ?? ''

  const ttsAvailability = (() => {
    if (tts.backend !== 'host') return null
    if (tts.available) return <StatusBadge tone="ok">{tts.command}</StatusBadge>
    return <StatusBadge tone="warn">No speech command found</StatusBadge>
  })()

  return (
    <>
      <SectionCard
        title="Speech to text"
        description="How the mic and recordings become text."
        actions={
          <ShadcnButton variant="ghost" size="sm" className="voice-group-ghost" onClick={onConfigure}>
            <Settings2 />
            All settings
          </ShadcnButton>
        }
      >
        <Facts>
          <Fact label="Engine">
            <span className="voice-choice">
              <OptionSelect
                aria-label="speech to text engine"
                value={stt.backend}
                disabled={busy !== null}
                onChange={(next) => choose(['stt', 'backend'], next, `Speech to text uses ${next}.`)}
                options={[
                  { value: 'local', label: 'Bundled local models', description: 'English, nothing leaves the machine' },
                  {
                    value: 'whisper_cpp',
                    label: 'whisper.cpp on this machine',
                    description: 'Multilingual with a compatible GGML model',
                  },
                  {
                    value: 'router',
                    label: 'A speech provider through llm-router',
                    description: 'ElevenLabs, OpenAI, ...',
                  },
                  {
                    value: 'openai',
                    label: 'OpenAI-compatible endpoint',
                    description: 'Any /audio/transcriptions server',
                  },
                ]}
              />
            </span>
          </Fact>
        </Facts>
        {stt.backend === 'router' ? (
          <Facts>
            <Fact
              label="Model"
              hint={
                <>
                  Audio goes to the provider that serves this model; its key lives in the router's Settings. While you
                  dictate, live words stay local and each finished sentence is re-decoded by this model.
                </>
              }
            >
              <span className="voice-choice">
                <OptionSelect
                  aria-label="router speech to text model"
                  value={report.stt.model === 'router picks' ? '' : report.stt.model}
                  disabled={busy !== null || routerStt.models === null}
                  onChange={(next) =>
                    choose(
                      ['stt', 'router', 'model'],
                      next,
                      next ? `Transcripts use ${next}.` : 'The router picks the model.',
                    )
                  }
                  options={routerModelOptions(
                    routerStt.models,
                    report.stt.model === 'router picks' ? '' : report.stt.model,
                  )}
                />
                {routerStt.models !== null && routerStt.models.length === 0 ? (
                  <StatusBadge tone="warn">No speech provider registered</StatusBadge>
                ) : null}
              </span>
            </Fact>
            {liveWordsFact}
          </Facts>
        ) : null}
        {stt.backend === 'local' ? (
          <Facts>
            <Fact
              label="Model"
              hint={
                stt.final_model === ''
                  ? 'Only the live model runs: words appear instantly but without punctuation.'
                  : 'Each sentence is re-decoded by this model after you pause, so transcripts get punctuation, casing and its accuracy.'
              }
            >
              <span className="voice-choice">
                <OptionSelect
                  aria-label="speech to text model"
                  value={stt.final_model === '' ? NONE : stt.final_model}
                  disabled={busy !== null || models === null}
                  onChange={(next) =>
                    choose(
                      ['stt', 'final_model'],
                      next === NONE ? '' : next,
                      next === NONE ? 'Using the live model only.' : `Using ${next} for transcripts.`,
                    )
                  }
                  options={[
                    ...modelOptions(offline, stt.final_model),
                    { value: NONE, label: 'Live model only', description: 'Fast, no punctuation' },
                  ]}
                />
                {accurate ? (
                  <ModelDownload
                    model={accurate}
                    progress={progress[accurate.id]}
                    busy={busy === accurate.id || stt.final_state === 'downloading'}
                    disabled={busy !== null}
                    onDownload={() => download(accurate.id)}
                  />
                ) : stt.final_model && models ? (
                  <StatusBadge tone="warn">Not in the catalog</StatusBadge>
                ) : null}
              </span>
            </Fact>
            {liveWordsFact}
            <Fact label="Runs on" hint="Audio never leaves the worker's machine.">
              <span className="voice-row-value">This machine</span>
            </Fact>
          </Facts>
        ) : null}
        {stt.backend === 'whisper_cpp' ? (
          <Facts>
            <Fact
              label="Model"
              hint={
                <>
                  Choose a multilingual model and download it once. Larger models need more memory and processing time.
                  whisper-cli must be installed separately; set its command, language, or a custom model path under All
                  settings.
                </>
              }
            >
              <span className="voice-choice">
                <OptionSelect
                  aria-label="whisper.cpp model"
                  value={stt.model}
                  disabled={busy !== null || models === null}
                  options={modelOptions(whisper, stt.model)}
                  onChange={(next) => choose(['stt', 'whisper_cpp', 'model'], next, `Transcripts use ${next}.`)}
                />
                {whisperModel ? (
                  <ModelDownload
                    model={whisperModel}
                    progress={progress[whisperModel.id]}
                    busy={busy === whisperModel.id}
                    disabled={busy !== null}
                    onDownload={() => download(whisperModel.id)}
                  />
                ) : (
                  <StatusBadge tone={stt.installed ? 'ok' : 'warn'}>
                    {stt.installed ? 'Custom model installed' : 'Custom model missing'}
                  </StatusBadge>
                )}
              </span>
            </Fact>
            {liveWordsFact}
            <Fact label="Runs on" hint="Audio never leaves the worker's machine.">
              <span className="voice-row-value">This machine</span>
            </Fact>
          </Facts>
        ) : null}
        {stt.backend === 'openai' ? (
          <Facts>
            <Fact
              label="Endpoint model"
              hint={<>Audio is sent to the configured endpoint; its address and key are under All settings.</>}
            >
              <span className="voice-choice">
                <span className="voice-mono">{stt.model}</span>
                <StatusBadge tone="ok">OpenAI-compatible</StatusBadge>
              </span>
            </Fact>
            {liveWordsFact}
          </Facts>
        ) : null}
        {stt.problem ? <p className="voice-panel-note voice-warn">{stt.problem}</p> : null}
      </SectionCard>

      <SectionCard title="Read aloud" description="The voice that reads replies and selected text.">
        <Facts>
          <Fact label="Engine">
            <span className="voice-choice">
              <OptionSelect
                aria-label="read aloud engine"
                value={tts.backend}
                disabled={busy !== null}
                onChange={(next) =>
                  choose(['tts', 'backend'], next, `Read aloud uses ${next === 'off' ? 'nothing' : next}.`)
                }
                options={[
                  {
                    value: 'piper',
                    label: 'Piper · natural local voice',
                    description: 'Neural voice on the worker, audio in this browser',
                  },
                  {
                    value: 'host',
                    label: 'System voice · basic / robotic',
                    description: 'Generated on the worker, played only in your browser',
                  },
                  {
                    value: 'router',
                    label: 'A speech provider through llm-router',
                    description: 'Audio plays in the browser',
                  },
                  { value: 'openai', label: 'OpenAI-compatible endpoint', description: 'Audio plays in the browser' },
                  { value: 'off', label: 'Off' },
                ]}
              />
              {ttsAvailability}
            </span>
          </Fact>
          {tts.backend === 'piper' ? (
            <Fact
              stack
              label="Neural voice"
              hint={
                <>
                  Processing preference:{' '}
                  {tts.device === 'cpu' ? 'CPU only' : 'Automatic · prefer GPU, fall back to CPU'}. Change under All
                  settings. Automatic is a preference, not confirmation of GPU use.
                </>
              }
            >
              <PiperVoicePicker
                models={models?.models ?? null}
                selected={tts.model ?? ''}
                disabled={busy !== null}
                progress={progress}
                busyId={busy}
                onSelect={(next) => choose(['tts', 'piper', 'model'], next, 'Neural voice selected.')}
                onDownload={download}
              />
              {tts.problem ? <p className="voice-row-hint voice-warn">{tts.problem}</p> : null}
            </Fact>
          ) : null}
          {tts.backend === 'router' ? (
            <Fact label="Model" hint={tts.command ?? 'The voice name and audio format are under All settings.'}>
              <span className="voice-choice">
                <OptionSelect
                  aria-label="router text to speech model"
                  value={routerTtsModel}
                  disabled={busy !== null || routerTts.models === null}
                  onChange={(next) => {
                    const providerChanged = providerOf(next) !== providerOf(routerTtsModel)
                    setRouterTtsModel(next)
                    run(
                      'config',
                      patchConfig(host.iii, (current) => {
                        const withModel = setPath(current, ['tts', 'router', 'model'], next)
                        return providerChanged ? setPath(withModel, ['tts', 'router', 'voice'], '') : withModel
                      }),
                      next
                        ? `Read aloud uses ${next}.${providerChanged ? " Voice reset to that provider's default." : ''}`
                        : 'The router picks the voice model.',
                    )
                  }}
                  options={routerModelOptions(routerTts.models, routerTtsModel)}
                />
                {routerTts.models !== null && routerTts.models.length === 0 ? (
                  <StatusBadge tone="warn">No speech provider registered</StatusBadge>
                ) : null}
              </span>
            </Fact>
          ) : null}
          <Fact label="Playback" hint="Audio is returned to the browser that asked for it.">
            <span className="voice-row-value">This browser only</span>
          </Fact>
        </Facts>
      </SectionCard>

      <SectionCard
        title="Dictation sessions"
        description="Live recognizer sessions on the worker, from any browser."
        actions={sessions.length > 0 ? <StatusBadge tone="busy">{sessions.length} running</StatusBadge> : null}
      >
        {sessions.length === 0 ? (
          <PanelEmpty
            title="No dictation running"
            description="Hold the mic in the chat composer to talk, or start a session to watch the recognizer work."
            action={
              <ShadcnButton variant="secondary" size="sm" onClick={onDictate}>
                <Mic />
                Start dictation
              </ShadcnButton>
            }
          />
        ) : (
          <ShadcnTable className="voice-table">
            <ShadcnTableHeader>
              <ShadcnTableRow>
                <ShadcnTableHead>Session</ShadcnTableHead>
                <ShadcnTableHead>Audio</ShadcnTableHead>
                <ShadcnTableHead>Sentences</ShadcnTableHead>
                <ShadcnTableHead>Idle</ShadcnTableHead>
              </ShadcnTableRow>
            </ShadcnTableHeader>
            <ShadcnTableBody>
              {sessions.map((s) => (
                <ShadcnTableRow key={s.session_id}>
                  <ShadcnTableCell>
                    <span className="voice-choice">
                      <StatusDot tone="accent" pulse={s.idle_secs < 2} />
                      <span className="voice-mono">{s.session_id.slice(0, 10)}</span>
                    </span>
                  </ShadcnTableCell>
                  <ShadcnTableCell>{formatDuration(s.duration_secs)}</ShadcnTableCell>
                  <ShadcnTableCell>{s.segments}</ShadcnTableCell>
                  <ShadcnTableCell>{formatDuration(s.idle_secs)}</ShadcnTableCell>
                </ShadcnTableRow>
              ))}
            </ShadcnTableBody>
          </ShadcnTable>
        )}
      </SectionCard>
    </>
  )
}
