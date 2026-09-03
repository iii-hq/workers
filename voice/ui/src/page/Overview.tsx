/**
 * The Overview section: what the worker does right now, in the terms a
 * person chooses. Speech to text (which model, is it here, pick another),
 * read aloud (which engine), open dictation sessions, and the door to the
 * full configuration. Every choice here writes the same `voice`
 * configuration the Settings form edits.
 */

import {
  Button,
  Chip,
  EmptyState,
  type Host,
  Select,
  StatusDot,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { modelsDownload } from '../lib/client'
import { NONE, patchConfig, readConfig, setPath, stringAt } from '../lib/config'
import { MicIcon } from '../lib/icons'
import { type ProgressById, percent } from '../lib/progress'
import { routerModelOptions, useRouterSpeechModels } from '../lib/router'
import type { DictationListEntry, DoctorResponse, ModelInfo, ModelsListResponse } from '../lib/types'
import { Fact, Facts, formatBytes, formatDuration, SectionCard, useBusyAction } from './shared'

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
  const live: ModelInfo | undefined = models?.models.find((m) => m.id === stt.model)
  const pct = percent(progress[stt.final_model])
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
    <Fact label="Live words">
      <span className="voice-choice">
        <span className="voice-fact-line">
          <StatusDot tone={stt.loaded ? 'accent' : 'ink'} />
          <span>{live?.name ?? stt.model}</span>
        </span>
        {!stt.installed ? (
          <Chip tone="warning">downloads on first use</Chip>
        ) : (
          <Chip tone="success">{stt.loaded ? 'running' : 'installed'}</Chip>
        )}
      </span>
      <span className="voice-sub voice-block">
        Shows words as you speak and decides where a sentence ends; it always runs on this machine. Change it under All
        settings.
      </span>
    </Fact>
  )

  const providerOf = (model: string) => model.split('::')[0] ?? ''

  const accurateStatus = (() => {
    if (stt.final_state === 'off') return null
    if (stt.final_state === 'loaded') return <Chip tone="success">ready</Chip>
    if (stt.final_state === 'installed') return <Chip tone="success">installed</Chip>
    if (stt.final_state === 'downloading')
      return <Chip tone="accent">{pct === null ? 'downloading' : `downloading ${pct}%`}</Chip>
    if (stt.final_state === 'unknown') return <Chip tone="danger">not in the catalog</Chip>
    return <Chip tone="warning">not downloaded</Chip>
  })()

  const ttsAvailability = (() => {
    if (tts.backend !== 'host') return null
    if (tts.available) return <Chip tone="success">{tts.command}</Chip>
    return <Chip tone="warning">no speech command found</Chip>
  })()

  return (
    <>
      <SectionCard
        title="Speech to text"
        actions={
          <Button variant="ghost" size="sm" onClick={onConfigure}>
            All settings
          </Button>
        }
      >
        <Facts>
          <Fact label="Engine">
            <span className="voice-choice">
              <Select
                aria-label="speech to text engine"
                value={stt.backend}
                disabled={busy !== null}
                onChange={(next) => choose(['stt', 'backend'], next, `Speech to text uses ${next}.`)}
                options={[
                  { value: 'local', label: 'Local models on this machine', description: 'Nothing leaves the machine' },
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
            <Fact label="Model">
              <span className="voice-choice">
                <Select
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
                  <Chip tone="warning">no speech provider registered</Chip>
                ) : null}
              </span>
              <span className="voice-sub voice-block">
                Audio goes to the provider that serves this model; its key lives in the router's Settings. While you
                dictate, live words stay local and each finished sentence is re-decoded by this model.
              </span>
            </Fact>
            {liveWordsFact}
          </Facts>
        ) : null}
        {stt.backend === 'local' ? (
          <Facts>
            <Fact label="Model">
              <span className="voice-choice">
                <Select
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
                    ...offline.map((m) => ({
                      value: m.id,
                      label: m.name,
                      description: `${formatBytes(m.size_bytes)} · ${m.installed ? 'installed' : 'downloads on first use'}`,
                    })),
                    { value: NONE, label: 'Live model only', description: 'Fast, no punctuation' },
                  ]}
                />
                {accurateStatus}
                {stt.final_state === 'missing' ? (
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={() => download(stt.final_model)}
                    disabled={busy !== null}
                  >
                    Download {accurate ? formatBytes(accurate.size_bytes) : ''}
                  </Button>
                ) : null}
              </span>
              <span className="voice-sub voice-block">
                {stt.final_model === ''
                  ? 'Only the live model runs: words appear instantly but without punctuation.'
                  : 'Each sentence is re-decoded by this model after you pause, so transcripts get punctuation, casing and its accuracy.'}
              </span>
            </Fact>
            {liveWordsFact}
            <Fact label="Runs on">
              <span>this machine, nothing leaves it</span>
            </Fact>
          </Facts>
        ) : null}
        {stt.backend === 'openai' ? (
          <Facts>
            <Fact label="Endpoint model">
              <span className="voice-fact-line">
                <Chip tone="neutral">OpenAI-compatible</Chip>
                <span className="voice-mono">{stt.model}</span>
              </span>
              <span className="voice-sub voice-block">
                Audio is sent to the configured endpoint; its address and key are under All settings.
              </span>
            </Fact>
          </Facts>
        ) : null}
        {stt.problem ? <p className="voice-note voice-warn">{stt.problem}</p> : null}
      </SectionCard>

      <SectionCard title="Read aloud">
        <Facts>
          <Fact label="Engine">
            <span className="voice-choice">
              <Select
                aria-label="read aloud engine"
                value={tts.backend}
                disabled={busy !== null}
                onChange={(next) =>
                  choose(['tts', 'backend'], next, `Read aloud uses ${next === 'off' ? 'nothing' : next}.`)
                }
                options={[
                  {
                    value: 'host',
                    label: "This machine's speech command",
                    description: 'say on macOS, espeak-ng on Linux',
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
          {tts.backend === 'router' ? (
            <Fact label="Model">
              <span className="voice-choice">
                <Select
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
                  <Chip tone="warning">no speech provider registered</Chip>
                ) : null}
              </span>
              <span className="voice-sub voice-block">
                {tts.command ?? 'The voice name and audio format are under All settings.'}
              </span>
            </Fact>
          ) : null}
          <Fact label="Playing">
            <span>{tts.playing === 0 ? 'nothing' : `${tts.playing} clip${tts.playing === 1 ? '' : 's'}`}</span>
          </Fact>
        </Facts>
      </SectionCard>

      <SectionCard
        title="Dictation sessions"
        actions={<Chip tone={sessions.length > 0 ? 'accent' : 'neutral'}>{sessions.length}</Chip>}
      >
        {sessions.length === 0 ? (
          <EmptyState
            icon={MicIcon}
            title="No dictation running"
            description="Hold the mic in the chat composer to talk, or start a session here to watch the recognizer work."
            action={{ label: 'Start dictation', onClick: onDictate }}
          />
        ) : (
          <TableViewport>
            <TableFrame>
              <Table density="compact">
                <TableHeader>
                  <TableRow>
                    <TableHead>session</TableHead>
                    <TableHead>audio</TableHead>
                    <TableHead>sentences</TableHead>
                    <TableHead>idle</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {sessions.map((s) => (
                    <TableRow key={s.session_id}>
                      <TableCell>
                        <span className="voice-fact-line">
                          <StatusDot tone="accent" pulse={s.idle_secs < 2} />
                          <span className="voice-mono">{s.session_id.slice(0, 10)}</span>
                        </span>
                      </TableCell>
                      <TableCell>{formatDuration(s.duration_secs)}</TableCell>
                      <TableCell>{s.segments}</TableCell>
                      <TableCell>{formatDuration(s.idle_secs)}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </TableFrame>
          </TableViewport>
        )}
      </SectionCard>
    </>
  )
}
