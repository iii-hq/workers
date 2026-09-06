/**
 * The voice worker's configuration form, registered through
 * `host.configForms` and rendered inside global Settings. It edits the
 * working draft via `onChange`; dirty tracking, save/reset and error
 * mapping stay host-owned (the console's save bar drives
 * `configuration::set`). Every field of WorkerConfig (voice/src/config.rs)
 * is here in plain words, grouped as a person thinks about it: which
 * speech model, how sentences end, how replies are read, and limits.
 */

import type { Host } from '@iii-dev/console-ui'
import {
  type ConfigFormProps,
  Input,
  type JsonValue,
  Select,
  SettingsField,
  SettingsList,
  SettingsSection,
} from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { modelsList } from '../lib/client'
import { DEFAULTS, NONE, numberAt, setPath, stringAt } from '../lib/config'
import { routerModelOptions, useRouterSpeechModels } from '../lib/router'
import type { ModelInfo } from '../lib/types'

function describeModel(m: ModelInfo): string {
  const size =
    m.size_bytes >= 1_000_000_000
      ? `${(m.size_bytes / 1_000_000_000).toFixed(1)} GB`
      : `${Math.round(m.size_bytes / 1_000_000)} MB`
  return `${m.name} · ${size} · ${m.installed ? 'installed' : 'not downloaded'}`
}

export function createVoiceConfigForm(host: Host) {
  return function VoiceConfigForm(props: ConfigFormProps) {
    const { value, onChange } = props
    const [models, setModels] = useState<ModelInfo[] | null>(null)

    useEffect(() => {
      let cancelled = false
      modelsList(host.iii)
        .then((res) => {
          if (!cancelled) setModels(res.models)
        })
        .catch(() => {
          if (!cancelled) setModels([])
        })
      return () => {
        cancelled = true
      }
    }, [])

    const set = (path: readonly string[], next: JsonValue | undefined) => onChange(setPath(value, path, next))
    const setNumber = (path: readonly string[], raw: string, integer: boolean, min = 0) => {
      if (raw.trim() === '') {
        set(path, undefined)
        return
      }
      const n = Number(raw)
      if (!Number.isFinite(n) || n < min || (integer && !Number.isInteger(n))) return
      set(path, n)
    }
    const [drafts, setDrafts] = useState<Record<string, string>>({})
    const numberField = (path: readonly string[], fallback: number, integer: boolean, min = 0) => {
      const key = path.join('.')
      return {
        value: drafts[key] ?? String(numberAt(value, path, fallback)),
        onChange: (raw: string) => {
          setDrafts((current) => ({ ...current, [key]: raw }))
          setNumber(path, raw, integer, min)
        },
        onBlur: () => {
          setDrafts((current) => {
            if (!(key in current)) return current
            const { [key]: _committed, ...rest } = current
            return rest
          })
        },
      }
    }

    const sttBackend = stringAt(value, ['stt', 'backend'], DEFAULTS.sttBackend)
    const ttsBackend = stringAt(value, ['tts', 'backend'], DEFAULTS.ttsBackend)
    const finalModel = stringAt(value, ['stt', 'final_model'], DEFAULTS.finalModel)
    const liveModel = stringAt(value, ['stt', 'model'], DEFAULTS.model)
    const offline = (models ?? []).filter((m) => m.kind === 'offline_nemo_transducer')
    const streaming = (models ?? []).filter((m) => m.kind === 'streaming_transducer')
    const routerStt = useRouterSpeechModels(host.iii, 'stt', sttBackend === 'router')
    const routerTts = useRouterSpeechModels(host.iii, 'tts', ttsBackend === 'router')

    return (
      <>
        <SettingsSection
          title="Speech to text"
          description="Where spoken words become text. Local runs entirely on this machine with models the worker downloads once; an OpenAI-compatible endpoint sends audio to that server."
        >
          <SettingsList>
            <SettingsField
              label="Engine"
              description="Local models, a speech provider registered with llm-router (ElevenLabs, OpenAI, ...), or any server that speaks the OpenAI audio API (a local whisper server counts)."
              renderControl={(c) => (
                <Select
                  id={c.id}
                  value={sttBackend}
                  onChange={(next) => set(['stt', 'backend'], next)}
                  options={[
                    { value: 'local', label: 'Local models on this machine' },
                    { value: 'router', label: 'A speech provider through llm-router' },
                    { value: 'openai', label: 'OpenAI-compatible endpoint' },
                  ]}
                />
              )}
            />
            {sttBackend === 'router' ? (
              <>
                <SettingsField
                  label="Model"
                  description="A speech-to-text model the router lists; its provider's key lives in the router's own settings. Empty lets the router pick."
                  renderControl={(c) => (
                    <Select
                      id={c.id}
                      value={stringAt(value, ['stt', 'router', 'model'])}
                      onChange={(next) => set(['stt', 'router', 'model'], next)}
                      aria-busy={routerStt.models === null}
                      options={routerModelOptions(routerStt.models, stringAt(value, ['stt', 'router', 'model']))}
                    />
                  )}
                />
                <SettingsField
                  label="Language hint"
                  description="BCP-47 tag sent with each request, for example en or hi. Empty lets the model detect it."
                  controlSize="compact"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      value={stringAt(value, ['stt', 'router', 'language'])}
                      onChange={(raw) => set(['stt', 'router', 'language'], raw)}
                    />
                  )}
                />
              </>
            ) : null}
            {sttBackend === 'local' ? (
              <>
                <SettingsField
                  label="Accurate model"
                  description="Re-decodes each sentence after you pause, adding punctuation and casing. This is the text you keep. None keeps only the live words."
                  renderControl={(c) => (
                    <Select
                      id={c.id}
                      value={finalModel === '' ? NONE : finalModel}
                      onChange={(next) => set(['stt', 'final_model'], next === NONE ? '' : next)}
                      aria-busy={models === null}
                      options={[
                        ...offline.map((m) => ({ value: m.id, label: m.id, description: describeModel(m) })),
                        ...(offline.some((m) => m.id === finalModel) || finalModel === ''
                          ? []
                          : [{ value: finalModel, label: finalModel }]),
                        { value: NONE, label: 'None: live words only', description: 'Fast, no punctuation' },
                      ]}
                    />
                  )}
                />
                <SettingsField
                  label="Live model"
                  description="Small streaming model that shows words as you speak and decides where a sentence ends."
                  renderControl={(c) => (
                    <Select
                      id={c.id}
                      value={liveModel}
                      onChange={(next) => set(['stt', 'model'], next)}
                      aria-busy={models === null}
                      options={[
                        ...streaming.map((m) => ({ value: m.id, label: m.id, description: describeModel(m) })),
                        ...(streaming.some((m) => m.id === liveModel) ? [] : [{ value: liveModel, label: liveModel }]),
                      ]}
                    />
                  )}
                />
                <SettingsField
                  label="Pause that ends a sentence"
                  description="Seconds of silence after speech before the sentence is committed. Lower feels snappier; higher tolerates thinking pauses."
                  controlSize="compact"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      type="number"
                      step="0.1"
                      min="0.2"
                      {...numberField(['stt', 'silence_after_speech_secs'], DEFAULTS.silenceAfterSpeech, false, 0.2)}
                    />
                  )}
                />
                <SettingsField
                  label="Longest sentence"
                  description="Seconds before a sentence is committed even without a pause."
                  controlSize="compact"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      type="number"
                      step="1"
                      min="2"
                      {...numberField(['stt', 'max_utterance_secs'], DEFAULTS.maxUtterance, false, 2)}
                    />
                  )}
                />
                <SettingsField
                  label="Silence before any speech"
                  description="Seconds of silence that end an empty segment. Keep it above two seconds so the first words of a sentence are never cut."
                  controlSize="compact"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      type="number"
                      step="0.1"
                      min="1"
                      {...numberField(['stt', 'silence_without_speech_secs'], DEFAULTS.silenceWithoutSpeech, false, 1)}
                    />
                  )}
                />
                <SettingsField
                  label="Decoder threads"
                  description="CPU threads for the local models."
                  controlSize="compact"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      type="number"
                      step="1"
                      min="1"
                      {...numberField(['stt', 'num_threads'], DEFAULTS.numThreads, true, 1)}
                    />
                  )}
                />
              </>
            ) : null}
            {sttBackend === 'openai' ? (
              <>
                <SettingsField
                  label="Base URL"
                  description="The API root, for example https://api.openai.com/v1 or http://127.0.0.1:8000/v1."
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      value={stringAt(value, ['stt', 'openai', 'base_url'], DEFAULTS.openaiBaseUrl)}
                      onChange={(raw) => set(['stt', 'openai', 'base_url'], raw)}
                      preserveCase
                    />
                  )}
                />
                <SettingsField
                  label="API key"
                  description="Bearer token. ${OPENAI_API_KEY} reads it from the engine's environment. Leave empty for servers that need none."
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      type="password"
                      value={stringAt(value, ['stt', 'openai', 'api_key'])}
                      onChange={(raw) => set(['stt', 'openai', 'api_key'], raw)}
                      preserveCase
                    />
                  )}
                />
                <SettingsField
                  label="Model"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      value={stringAt(value, ['stt', 'openai', 'model'], DEFAULTS.openaiSttModel)}
                      onChange={(raw) => set(['stt', 'openai', 'model'], raw)}
                      preserveCase
                    />
                  )}
                />
                <SettingsField
                  label="Language hint"
                  description="ISO 639-1 code sent with each request; empty lets the server detect it."
                  controlSize="compact"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      value={stringAt(value, ['stt', 'openai', 'language'])}
                      onChange={(raw) => set(['stt', 'openai', 'language'], raw)}
                    />
                  )}
                />
              </>
            ) : null}
          </SettingsList>
        </SettingsSection>

        <SettingsSection title="Read aloud" description="How replies are spoken when you ask for it.">
          <SettingsList>
            <SettingsField
              label="Engine"
              description="The host command plays on the machine running the worker (say on macOS, espeak-ng on Linux). A router speech provider or an OpenAI-compatible endpoint returns audio to the browser."
              renderControl={(c) => (
                <Select
                  id={c.id}
                  value={ttsBackend}
                  onChange={(next) => set(['tts', 'backend'], next)}
                  options={[
                    { value: 'host', label: "This machine's speech command" },
                    { value: 'router', label: 'A speech provider through llm-router' },
                    { value: 'openai', label: 'OpenAI-compatible endpoint' },
                    { value: 'off', label: 'Off' },
                  ]}
                />
              )}
            />
            {ttsBackend === 'router' ? (
              <>
                <SettingsField
                  label="Model"
                  description="A text-to-speech model the router lists. Empty lets the router pick."
                  renderControl={(c) => (
                    <Select
                      id={c.id}
                      value={stringAt(value, ['tts', 'router', 'model'])}
                      onChange={(next) => set(['tts', 'router', 'model'], next)}
                      aria-busy={routerTts.models === null}
                      options={routerModelOptions(routerTts.models, stringAt(value, ['tts', 'router', 'model']))}
                    />
                  )}
                />
                <SettingsField
                  label="Voice"
                  description="A voice id or name as the provider knows it (for ElevenLabs, a voice on the account such as George). Empty uses the provider's default."
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      value={stringAt(value, ['tts', 'router', 'voice'])}
                      onChange={(raw) => set(['tts', 'router', 'voice'], raw)}
                      preserveCase
                    />
                  )}
                />
                <SettingsField
                  label="Audio format"
                  description="What the browser receives. mp3 works everywhere."
                  controlSize="compact"
                  renderControl={(c) => (
                    <Select
                      id={c.id}
                      value={stringAt(value, ['tts', 'router', 'format'], DEFAULTS.routerFormat)}
                      onChange={(next) => set(['tts', 'router', 'format'], next)}
                      options={[
                        { value: 'mp3', label: 'mp3' },
                        { value: 'wav', label: 'wav' },
                        { value: 'opus', label: 'opus' },
                      ]}
                    />
                  )}
                />
              </>
            ) : null}
            {ttsBackend === 'host' ? (
              <>
                <SettingsField
                  label="Voice"
                  description="A voice name the command knows (say -v, espeak-ng -v). Empty uses the system default."
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      value={stringAt(value, ['tts', 'voice'])}
                      onChange={(raw) => set(['tts', 'voice'], raw)}
                      preserveCase
                    />
                  )}
                />
                <SettingsField
                  label="Speaking rate"
                  description="Words per minute. 0 uses the command's default."
                  controlSize="compact"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      type="number"
                      step="10"
                      min="0"
                      {...numberField(['tts', 'rate_wpm'], 0, true, 0)}
                    />
                  )}
                />
              </>
            ) : null}
            {ttsBackend === 'openai' ? (
              <>
                <SettingsField
                  label="Base URL"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      value={stringAt(value, ['tts', 'openai', 'base_url'], DEFAULTS.openaiBaseUrl)}
                      onChange={(raw) => set(['tts', 'openai', 'base_url'], raw)}
                      preserveCase
                    />
                  )}
                />
                <SettingsField
                  label="API key"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      type="password"
                      value={stringAt(value, ['tts', 'openai', 'api_key'])}
                      onChange={(raw) => set(['tts', 'openai', 'api_key'], raw)}
                      preserveCase
                    />
                  )}
                />
                <SettingsField
                  label="Model"
                  controlSize="compact"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      value={stringAt(value, ['tts', 'openai', 'model'], DEFAULTS.openaiTtsModel)}
                      onChange={(raw) => set(['tts', 'openai', 'model'], raw)}
                      preserveCase
                    />
                  )}
                />
                <SettingsField
                  label="Voice"
                  controlSize="compact"
                  renderControl={(c) => (
                    <Input
                      id={c.id}
                      value={stringAt(value, ['tts', 'openai', 'voice'], DEFAULTS.openaiTtsVoice)}
                      onChange={(raw) => set(['tts', 'openai', 'voice'], raw)}
                      preserveCase
                    />
                  )}
                />
              </>
            ) : null}
            {ttsBackend !== 'off' ? (
              <SettingsField
                label="Longest text per request"
                description="Characters one read-aloud call will speak."
                controlSize="compact"
                renderControl={(c) => (
                  <Input
                    id={c.id}
                    type="number"
                    step="100"
                    min="1"
                    {...numberField(['tts', 'max_speak_chars'], DEFAULTS.maxSpeakChars, true, 1)}
                  />
                )}
              />
            ) : null}
          </SettingsList>
        </SettingsSection>

        <SettingsSection title="Storage and limits">
          <SettingsList>
            <SettingsField
              label="Models directory"
              description="Where downloaded models live. Relative paths resolve against the project directory."
              renderControl={(c) => (
                <Input
                  id={c.id}
                  value={stringAt(value, ['models_dir'], DEFAULTS.modelsDir)}
                  onChange={(raw) => set(['models_dir'], raw)}
                  preserveCase
                />
              )}
            />
            <SettingsField
              label="Largest inline audio file"
              description="Megabytes accepted by voice::transcribe when the file is sent inline; larger files are passed by path."
              controlSize="compact"
              renderControl={(c) => (
                <Input
                  id={c.id}
                  type="number"
                  step="1"
                  min="1"
                  value={
                    drafts.max_audio_bytes ??
                    String(Math.round(numberAt(value, ['max_audio_bytes'], DEFAULTS.maxAudioBytes) / (1024 * 1024)))
                  }
                  onChange={(raw) => {
                    setDrafts((current) => ({ ...current, max_audio_bytes: raw }))
                    const mb = Number(raw)
                    if (raw.trim() === '') set(['max_audio_bytes'], undefined)
                    else if (Number.isInteger(mb) && mb >= 1) set(['max_audio_bytes'], mb * 1024 * 1024)
                  }}
                  onBlur={() =>
                    setDrafts((current) => {
                      const { max_audio_bytes: _committed, ...rest } = current
                      return rest
                    })
                  }
                />
              )}
            />
            <SettingsField
              label="Open dictation sessions"
              description="Across every caller at once."
              controlSize="compact"
              renderControl={(c) => (
                <Input
                  id={c.id}
                  type="number"
                  step="1"
                  min="1"
                  {...numberField(['max_sessions'], DEFAULTS.maxSessions, true, 1)}
                />
              )}
            />
            <SettingsField
              label="Idle session timeout"
              description="Seconds without audio before the worker closes a dictation session."
              controlSize="compact"
              renderControl={(c) => (
                <Input
                  id={c.id}
                  type="number"
                  step="10"
                  min="10"
                  {...numberField(['session_idle_secs'], DEFAULTS.sessionIdleSecs, true, 10)}
                />
              )}
            />
          </SettingsList>
        </SettingsSection>
      </>
    )
  }
}
