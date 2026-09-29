import { Children, createElement, isValidElement, type ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { modelsDownload } from './client'
import { DEFAULTS } from './config'
import { ModelDownload } from './ModelDownload'
import { modelOptions, modelUseLabel, splitModelsByPurpose, useModelConfig } from './models'
import { percent } from './progress'
import { filterPiperVoices } from './piper-languages'
import type { ExtensionIii, ModelInfo, ModelProgressEvent } from './types'

vi.mock('@iii-dev/console-ui', () => ({ Button: 'button', Chip: 'span' }))

const model: ModelInfo = {
  id: 'whisper-tiny', name: 'Whisper tiny', kind: 'whisper_ggml',
  languages: ['multilingual'], size_bytes: 77_691_713, installed: false,
}
const event: ModelProgressEvent = {
  id: model.id, file: 'ggml-tiny.bin', received_bytes: 50, total_bytes: 100, done: false,
}
function nodes(node: ReactNode): ReturnType<typeof Children.toArray> {
  return Children.toArray(node).flatMap((child) => isValidElement<{ children?: ReactNode }>(child)
    ? [child, ...nodes(child.props.children)] : [child])
}
function text(node: ReactNode) {
  return nodes(node).filter((child) => typeof child === 'string' || typeof child === 'number').join('')
}

describe('voice model choices', () => {
  it('offers missing models and preserves custom paths', () => {
    expect(modelOptions([model], '/models/custom.bin')).toEqual([
      { value: model.id, label: model.name, description: '74.1 MiB · not downloaded' },
      { value: '/models/custom.bin', label: '/models/custom.bin', description: 'Custom or unavailable model' },
    ])
  })
  it('does not duplicate the selected catalog model', () => {
    expect(modelOptions([model], model.id)).toHaveLength(1)
  })
  it('switches Whisper backend without losing language, command or unrelated settings', () => {
    const current = { tts: { backend: 'host' }, stt: { backend: 'local', model: 'zipformer-en-20m',
      whisper_cpp: { model: '/custom.bin', language: 'pt', command: '/bin/whisper-cli' } } }
    expect(useModelConfig(current, model)).toEqual({ ...current, stt: { ...current.stt,
      backend: 'whisper_cpp', whisper_cpp: { ...current.stt.whisper_cpp, model: model.id } } })
    expect(current.stt.whisper_cpp.model).toBe('/custom.bin')
  })
  it('switches back to local for Parakeet', () => {
    expect(useModelConfig({ stt: { backend: 'whisper_cpp' } }, {
      ...model, id: 'parakeet', kind: 'offline_nemo_transducer',
    })).toEqual({ stt: { backend: 'local', final_model: 'parakeet' } })
  })
  it('changes live words without changing a remote final pass', () => {
    expect(useModelConfig({ stt: { backend: 'router' } }, {
      ...model, id: 'zipformer-en-large', kind: 'streaming_transducer',
    })).toEqual({ stt: { backend: 'router', model: 'zipformer-en-large' } })
  })
  it('selecting a neural voice changes only TTS and preserves dictation', () => {
    const stt = { backend: 'whisper_cpp', model: 'zipformer-en-large' }
    expect(useModelConfig({ stt, tts: { backend: 'host', piper: { command: '/bin/piper' } } }, {
      ...model, id: 'piper-pt-br-faber-medium', kind: 'piper_onnx',
    })).toEqual({ stt, tts: { backend: 'piper', piper: { command: '/bin/piper', model: 'piper-pt-br-faber-medium' } } })
  })
  it('uses automatic processing by default without changing the selected voice', () => {
    expect(DEFAULTS.piperDevice).toBe('auto')
    const current = { tts: { backend: 'piper', piper: { device: 'cpu', command: '/bin/piper' } } }
    expect(useModelConfig(current, { ...model, id: 'piper-faber', kind: 'piper_onnx' }))
      .toEqual({ tts: { backend: 'piper', piper: { device: 'cpu', command: '/bin/piper', model: 'piper-faber' } } })
  })
  it('defaults to a selectable Whisper catalog id', () => {
    expect(DEFAULTS.whisperCppModel).toBe('whisper-large-v3-turbo')
  })
})

describe('separate listening and reading model configuration', () => {
  const streaming: ModelInfo = { ...model, id: 'zipformer', kind: 'streaming_transducer' }
  const offline: ModelInfo = { ...model, id: 'parakeet', kind: 'offline_nemo_transducer' }
  const faber: ModelInfo = { ...model, id: 'piper-faber', kind: 'piper_onnx', languages: ['pt-BR'], installed: true }
  const lessac: ModelInfo = { ...faber, id: 'piper-lessac', languages: ['en-US'], installed: false }
  const catalog = [faber, streaming, model, lessac, offline]

  it('keeps all transcription types separate from reading voices, installed or not', () => {
    const before = JSON.stringify(catalog)
    const groups = splitModelsByPurpose(catalog)
    expect(groups.listening).toEqual([streaming, model, offline])
    expect(groups.reading).toEqual([faber, lessac])
    expect(groups.listening.length + groups.reading.length).toBe(catalog.length)
    expect(JSON.stringify(catalog)).toBe(before)
  })

  it('filters only reading voices by language, never the listening catalog', () => {
    const { listening, reading } = splitModelsByPurpose(catalog)
    expect(filterPiperVoices(reading, 'en-US')).toEqual([lessac])
    expect(filterPiperVoices(reading, 'unknown language')).toEqual([])
    expect(listening).toEqual([streaming, model, offline])
  })

  it('counts installed models independently for each group', () => {
    const { listening, reading } = splitModelsByPurpose(catalog)
    expect(listening.filter((m) => m.installed)).toHaveLength(0)
    expect(reading.filter((m) => m.installed)).toHaveLength(1)
  })

  it('gives each selection button its actual configuration purpose', () => {
    expect(modelUseLabel(faber)).toBe('Use for read aloud')
    expect(modelUseLabel(model)).toBe('Use for transcription')
    expect(modelUseLabel(offline)).toBe('Use for transcription')
    expect(modelUseLabel(streaming)).toBe('Use for live dictation')
  })

  it('preserves empty catalogs and older streaming entries', () => {
    expect(splitModelsByPurpose([])).toEqual({ listening: [], reading: [] })
    const legacy = { ...streaming, kind: undefined }
    expect(splitModelsByPurpose([legacy])).toEqual({ listening: [legacy], reading: [] })
  })

  it('selecting any listening model preserves the configured reading voice', () => {
    const tts = { backend: 'piper', piper: { model: faber.id, command: '/bin/piper' } }
    for (const listening of [streaming, offline, model]) {
      expect(useModelConfig({ tts }, listening).tts).toEqual(tts)
    }
  })
})

describe('missing-model download control', () => {
  it('shows size and starts a download only when clicked', () => {
    const onDownload = vi.fn()
    const view = ModelDownload({ model, onDownload })
    expect(text(view)).toContain('not downloadedDownload 74.1 MiB')
    expect(onDownload).not.toHaveBeenCalled()
    const button = nodes(view).find((node) => isValidElement(node) && node.type === 'button')
    if (!isValidElement<{ onClick: () => void; 'aria-label': string }>(button)) throw new Error('missing button')
    expect(button.props['aria-label']).toBe('Download Whisper tiny')
    button.props.onClick()
    expect(onDownload).toHaveBeenCalledOnce()
  })
  it('keeps the status outside the spaced action group', () => {
    const use = createElement('button', { onClick: vi.fn() }, 'Use model')
    const view = ModelDownload({ model, actions: use, className: 'voice-model-footer', onDownload: vi.fn() })
    expect(view.props.className).toBe('voice-model-download voice-model-footer')
    const groups = Children.toArray(view.props.children)
    expect(groups).toHaveLength(2)
    const [status, actions] = groups
    if (!isValidElement<{ className: string; role: string }>(status)
      || !isValidElement<{ className: string }>(actions)) throw new Error('missing layout groups')
    expect(status.props.className).toBe('voice-model-status')
    expect(status.props.role).toBe('status')
    expect(actions.props.className).toBe('voice-model-actions')
    expect(text(status)).toBe('not downloaded')
    expect(text(actions)).toBe('Use modelDownload 74.1 MiB')
  })
  it('keeps related actions visible when the model is installed', () => {
    const view = ModelDownload({ model: { ...model, installed: true },
      actions: createElement('button', {}, 'Remove'), onDownload: vi.fn() })
    expect(text(view)).toBe('installedRemove')
  })
  it('hides the button when installed', () => {
    expect(text(ModelDownload({ model: { ...model, installed: true }, onDownload: vi.fn() }))).toBe('installed')
  })
  it('shows progress instead of a duplicate download action', () => {
    expect(text(ModelDownload({ model, progress: event, onDownload: vi.fn() }))).toBe('Downloading 50%')
  })
  it('shows an indeterminate state before the first progress event', () => {
    expect(text(ModelDownload({ model, busy: true, onDownload: vi.fn() }))).toBe('Downloading…')
  })
  it('allows retry after an error', () => {
    expect(text(ModelDownload({ model, progress: { ...event, done: true, error: 'offline' },
      onDownload: vi.fn() }))).toContain('Download 74.1 MiB')
  })
  it('does not mistake unknown download size for inactivity', () => {
    expect(text(ModelDownload({ model, progress: { ...event, total_bytes: 0 }, onDownload: vi.fn() })))
      .toBe('Downloading…')
  })
  it('clamps progress to the displayed percentage range', () => {
    expect(percent({ ...event, received_bytes: 200 })).toBe(100)
  })
  it('sends the selected id with a download-sized timeout and propagates failures', async () => {
    const trigger = vi.fn().mockRejectedValue(new Error('network unavailable'))
    const iii = { trigger } as unknown as ExtensionIii
    await expect(modelsDownload(iii, { id: model.id })).rejects.toThrow('network unavailable')
    expect(trigger).toHaveBeenCalledWith('voice::models::download', { id: model.id }, { timeoutMs: 3_600_000 })
  })
})
