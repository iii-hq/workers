/**
 * The router's speech models for one family, fetched once per mount.
 * `null` while loading, `[]` when the router is absent or lists none; the
 * error names why so a page can say it.
 */
import { useEffect, useState } from 'react'
import { routerSpeechModels } from './client'
import { errorMessage } from './format'
import type { ExtensionIii, RouterSpeechModel } from './types'

export interface RouterModelsState {
  models: RouterSpeechModel[] | null
  error: string | null
}

export function useRouterSpeechModels(iii: ExtensionIii, modality: 'stt' | 'tts', enabled: boolean): RouterModelsState {
  const [state, setState] = useState<RouterModelsState>({ models: null, error: null })
  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    routerSpeechModels(iii, modality)
      .then((res) => {
        if (!cancelled) setState({ models: res.models ?? [], error: null })
      })
      .catch((err: unknown) => {
        if (!cancelled) setState({ models: [], error: errorMessage(err) })
      })
    return () => {
      cancelled = true
    }
  }, [iii, modality, enabled])
  return state
}

/** The config value for a router model: the console's `provider::model` form. */
export function routerModelValue(model: RouterSpeechModel): string {
  return `${model.provider}::${model.id}`
}

export function routerModelOptions(
  models: RouterSpeechModel[] | null,
  current: string,
): Array<{ value: string; label: string; description?: string }> {
  const options: Array<{ value: string; label: string; description?: string }> = (models ?? []).map((m) => ({
    value: routerModelValue(m),
    label: m.display_name ?? m.id,
    description: `${m.provider}${m.speech?.languages?.length ? ` · ${m.speech.languages.length} languages` : ''}`,
  }))
  if (current && !options.some((o) => o.value === current)) options.push({ value: current, label: current })
  options.unshift({ value: '', label: 'Let the router pick', description: 'First provider that offers one' })
  return options
}
