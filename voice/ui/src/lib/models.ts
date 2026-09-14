import type { JsonObject } from './config'
import { setPath } from './config'
import { formatBytes } from './format'
import type { ModelInfo } from './types'

export function modelOptions(models: readonly ModelInfo[], selected: string) {
  const options = models.map((m) => ({
    value: m.id,
    label: m.name,
    description: `${formatBytes(m.size_bytes)} · ${m.installed ? 'installed' : 'not downloaded'}`,
  }))
  if (selected && !models.some((m) => m.id === selected)) {
    options.push({ value: selected, label: selected, description: 'Custom or unavailable model' })
  }
  return options
}

/** Keep TTS voices separate from input models; older catalogs omit kind on live models. */
export function splitModelsByPurpose(models: readonly ModelInfo[]) {
  const listening: ModelInfo[] = []
  const reading: ModelInfo[] = []
  for (const model of models) {
    if (model.kind === 'piper_onnx') reading.push(model)
    else listening.push(model)
  }
  return { listening, reading }
}

export function modelUseLabel(model: ModelInfo): string {
  if (model.kind === 'piper_onnx') return 'Use for read aloud'
  if (model.kind === 'whisper_ggml' || model.kind === 'offline_nemo_transducer') return 'Use for transcription'
  return 'Use for live dictation'
}

/** A catalog choice only changes the backend when that model requires it. */
export function useModelConfig(current: JsonObject, model: ModelInfo): JsonObject {
  if (model.kind === 'piper_onnx') {
    return setPath(setPath(current, ['tts', 'backend'], 'piper'), ['tts', 'piper', 'model'], model.id)
  }
  if (model.kind === 'whisper_ggml') {
    return setPath(setPath(current, ['stt', 'backend'], 'whisper_cpp'), ['stt', 'whisper_cpp', 'model'], model.id)
  }
  if (model.kind === 'offline_nemo_transducer') {
    return setPath(setPath(current, ['stt', 'backend'], 'local'), ['stt', 'final_model'], model.id)
  }
  return setPath(current, ['stt', 'model'], model.id)
}
