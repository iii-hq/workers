import { describe, expect, it, vi } from 'vitest'
import type { ExtensionIii } from '@iii-dev/console-ui'
import { patchConfig, readConfig } from './config'

describe('voice configuration identity', () => {
  it('reads the entry owned by the addressed voice worker', async () => {
    const call = vi.fn(async (fn: string) => fn === 'voice::configuration-id'
      ? { id: 'project-voice-hash' } : { value: { stt: { backend: 'router' } } })
    expect(await readConfig({ call } as unknown as ExtensionIii)).toEqual({ stt: { backend: 'router' } })
    expect(call).toHaveBeenCalledWith('configuration::get', { id: 'project-voice-hash' })
  })

  it('patches one resolved entry and preserves raw environment references', async () => {
    const call = vi.fn(async (fn: string) => {
      if (fn === 'voice::configuration-id') return { id: 'custom-voice' }
      if (fn === 'configuration::get') return { value: { token: '${API_TOKEN}' } }
      return {}
    })
    await patchConfig({ call } as unknown as ExtensionIii, (value) => ({ ...value, max_sessions: 4 }))
    expect(call.mock.calls.filter(([fn]) => fn === 'voice::configuration-id')).toHaveLength(1)
    expect(call).toHaveBeenCalledWith('configuration::get', { id: 'custom-voice', raw: true })
    expect(call).toHaveBeenCalledWith('configuration::set', { id: 'custom-voice', value: { token: '${API_TOKEN}', max_sessions: 4 } })
  })

  it('never writes a legacy global entry if identity discovery fails', async () => {
    const call = vi.fn().mockRejectedValue(new Error('worker disconnected'))
    await expect(patchConfig({ call } as unknown as ExtensionIii, () => ({}))).rejects.toThrow('worker disconnected')
    expect(call).toHaveBeenCalledTimes(1)
  })
})

describe('booleanAt', () => {
  it('reads a boolean and falls back for anything else', async () => {
    const { booleanAt } = await import('./config')
    expect(booleanAt({ tts: { chat_controls: true } }, ['tts', 'chat_controls'], false)).toBe(true)
    expect(booleanAt({ tts: { chat_controls: 'yes' } }, ['tts', 'chat_controls'], false)).toBe(false)
    expect(booleanAt(undefined, ['stt', 'cleanup', 'enabled'], true)).toBe(true)
  })
})
