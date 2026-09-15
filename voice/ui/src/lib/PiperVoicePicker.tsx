import { useId, useState } from 'react'
import { Select } from '@iii-dev/console-ui'
import { filterPiperVoices, piperLanguages } from './piper-languages'
import { ModelDownload } from './ModelDownload'
import { modelOptions } from './models'
import type { ProgressById } from './progress'
import type { ModelInfo } from './types'

const uiLocale = () => typeof navigator === 'undefined' ? 'en' : navigator.language

export function PiperLanguageFilter({ models, value, onChange, disabled = false }: {
  models: readonly ModelInfo[]; value: string; onChange: (language: string) => void; disabled?: boolean
}) {
  const id = useId()
  const languages = piperLanguages(models, uiLocale())
  const matches = filterPiperVoices(models, value, uiLocale())
  return <div className="voice-piper-language">
    <label className="voice-strong" htmlFor={id}>What language do you speak?</label>
    <input id={id} type="text" className="voice-language-input" list={`${id}-languages`}
      value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)}
      placeholder="e.g. português, English, español, pt-BR…" autoComplete="off"
      aria-describedby={`${id}-help`} />
    <datalist id={`${id}-languages`}>
      {languages.map((language) => <option key={language.code} value={language.code}>
        {language.label} · {language.count} voices
      </option>)}
    </datalist>
    <span id={`${id}-help`} className="voice-sub" role="status">
      {!value.trim() ? 'Enter your language to see available Piper voices. No model downloads automatically.'
        : matches.length ? `${matches.length} matching voices. Choose one, then click Download.`
        : 'No Piper voices match this language in the bundled catalog. Try its name or a locale code, or choose another language.'}
    </span>
  </div>
}

/** Language search is a local filter, never a config change or a download. */
export function PiperVoicePicker({ models, selected, disabled = false, onSelect, onDownload, progress, busyId, controlId }: {
  models: readonly ModelInfo[] | null
  selected: string
  disabled?: boolean
  onSelect: (id: string) => void
  onDownload: (id: string) => void
  progress: ProgressById
  busyId?: string | null
  controlId?: string
}) {
  const [language, setLanguage] = useState('')
  const voices = filterPiperVoices(models ?? [], language, uiLocale())
  const selectedModel = (models ?? []).find((m) => m.kind === 'piper_onnx' && m.id === selected)
  const visibleSelection = voices.find((m) => m.id === selected)
  return <div className="voice-piper-picker">
    <PiperLanguageFilter models={models ?? []} value={language} onChange={setLanguage} disabled={disabled || models === null} />
    {voices.length > 0 ? <span className="voice-choice">
      <Select id={controlId} aria-label="Piper voice in your language" value={visibleSelection ? selected : ''}
        disabled={disabled} onChange={(next) => { if (next) onSelect(next) }}
        options={[{ value: '', label: 'Choose a voice…' }, ...modelOptions(voices, '')]} />
      {visibleSelection ? <ModelDownload model={visibleSelection} progress={progress[selected]}
        busy={busyId === selected} disabled={disabled} onDownload={() => onDownload(selected)} /> : null}
    </span> : null}
    <span className="voice-sub">Current voice: {selectedModel?.name ?? (selected || 'none')}.
      {' '}Filtering does not change it or delete downloaded voices.</span>
    {models === null ? <span className="voice-sub" role="status">Loading voice catalog…</span> : null}
  </div>
}
