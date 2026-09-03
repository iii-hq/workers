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

    const sttBackend = stringAt(value, ['stt', 'backend'], DEFAULTS.sttBackend)
    const ttsBackend = stringAt(value, ['tts', 'backend'], DEFAULTS.ttsBackend)
    const finalModel = stringAt(value, ['stt', 'final_model'], DEFAULTS.finalModel)
    const liveModel = stringAt(value, ['stt', 'model'], DEFAULTS.model)
    const offline = (models ?? []).filter((m) => m.kind === 'offline_nemo_transducer')
    const streaming = (models ?? []).filter((m) => m.kind === 'streaming_transducer')

    return (
      <>
        <SettingsSection
          title="Speech to text"
          description="Where spoken words become text. Local runs entirely on this machine with models the worker downloads once; an OpenAI-compatible endpoint sends audio to that server."
        >
          <SettingsList>
            <SettingsField
              label="Engine"
              description="Local models, or any server that speaks the OpenAI audio API (a local whisper server counts)."
              renderControl={(c) => (
                <Select
                  id={c.id}
                  value={sttBackend}
                  onChange={(next) => set(['stt', 'backend'], next)}
                  options={[
                    { value: 'local', label: 'Local models on this machine' },
                    { value: 'openai', label: 'OpenAI-compatible endpoint' },
                  ]}
                />
              )}
            />
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
                      value={String(numberAt(value, ['stt', 'silence_after_speech_secs'], DEFAULTS.silenceAfterSpeech))}
                      onChange={(raw) => setNumber(['stt', 'silence_after_speech_secs'], raw, false, 0.1)}
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
                      value={String(numberAt(value, ['stt', 'max_utterance_secs'], DEFAULTS.maxUtterance))}
                      onChange={(raw) => setNumber(['stt', 'max_utterance_secs'], raw, false, 1)}
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
                      value={String(
                        numberAt(value, ['stt', 'silence_without_speech_secs'], DEFAULTS.silenceWithoutSpeech),
                      )}
                      onChange={(raw) => setNumber(['stt', 'silence_without_speech_secs'], raw, false, 0.5)}
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
                      value={String(numberAt(value, ['stt', 'num_threads'], DEFAULTS.numThreads))}
                      onChange={(raw) => setNumber(['stt', 'num_threads'], raw, true, 1)}
                    />
                  )}
                />
              </>
            ) : (
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
            )}
          </SettingsList>
        </SettingsSection>

        <SettingsSection title="Read aloud" description="How replies are spoken when you ask for it.">
          <SettingsList>
            <SettingsField
              label="Engine"
              description="The host command plays on the machine running the worker (say on macOS, espeak-ng on Linux). An OpenAI-compatible endpoint returns audio to the browser."
              renderControl={(c) => (
                <Select
                  id={c.id}
                  value={ttsBackend}
                  onChange={(next) => set(['tts', 'backend'], next)}
                  options={[
                    { value: 'host', label: "This machine's speech command" },
                    { value: 'openai', label: 'OpenAI-compatible endpoint' },
                    { value: 'off', label: 'Off' },
                  ]}
                />
              )}
            />
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
                      value={String(numberAt(value, ['tts', 'rate_wpm'], 0))}
                      onChange={(raw) => setNumber(['tts', 'rate_wpm'], raw, true, 0)}
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
                    value={String(numberAt(value, ['tts', 'max_speak_chars'], DEFAULTS.maxSpeakChars))}
                    onChange={(raw) => setNumber(['tts', 'max_speak_chars'], raw, true, 1)}
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
                  value={String(
                    Math.round(numberAt(value, ['max_audio_bytes'], DEFAULTS.maxAudioBytes) / (1024 * 1024)),
                  )}
                  onChange={(raw) => {
                    const mb = Number(raw)
                    if (raw.trim() === '') set(['max_audio_bytes'], undefined)
                    else if (Number.isInteger(mb) && mb >= 1) set(['max_audio_bytes'], mb * 1024 * 1024)
                  }}
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
                  value={String(numberAt(value, ['max_sessions'], DEFAULTS.maxSessions))}
                  onChange={(raw) => setNumber(['max_sessions'], raw, true, 1)}
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
                  value={String(numberAt(value, ['session_idle_secs'], DEFAULTS.sessionIdleSecs))}
                  onChange={(raw) => setNumber(['session_idle_secs'], raw, true, 5)}
                />
              )}
            />
          </SettingsList>
        </SettingsSection>
      </>
    )
  }
}
