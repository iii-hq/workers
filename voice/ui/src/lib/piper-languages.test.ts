import { describe, expect, it } from 'vitest'
import { filterPiperVoices, piperLanguages } from './piper-languages'
import type { ModelInfo } from './types'

const voice = (id: string, language: string, installed = false): ModelInfo => ({
  id, name: id, kind: 'piper_onnx', languages: [language], size_bytes: 63_000_000, installed,
})
const models: ModelInfo[] = [
  voice('faber', 'pt-BR', true), voice('cadu', 'pt-BR'), voice('tugao', 'pt-PT'),
  voice('lessac', 'en-US'), voice('alan', 'en-GB'), voice('siwis', 'fr-FR'),
  voice('davefx', 'es-ES'), voice('claude', 'es-MX'), voice('thorsten', 'de-DE'),
  voice('huayan', 'zh-CN'),
  { ...voice('zipformer', 'en'), kind: 'streaming_transducer' },
  { ...voice('whisper', 'multilingual'), kind: 'whisper_ggml' },
]
const ids = (query: string, locale = 'en') => filterPiperVoices(models, query, locale).map((m) => m.id)

describe('language-first Piper catalog', () => {
  it('asks for a language rather than assuming Brazilian Portuguese', () => {
    expect(ids('')).toEqual([])
    expect(ids('   ')).toEqual([])
  })
  it('accepts language names with accents and without, regardless of UI locale', () => {
    expect(ids('português')).toEqual(['faber', 'cadu', 'tugao'])
    expect(ids('PORTUGUES')).toEqual(['faber', 'cadu', 'tugao'])
    expect(ids('Portuguese', 'de')).toEqual(['faber', 'cadu', 'tugao'])
    expect(ids('português brasileiro')).toEqual(['faber', 'cadu'])
    expect(ids('português europeu')).toEqual(['tugao'])
  })
  it('distinguishes regional codes while allowing a base language', () => {
    expect(ids('pt-BR')).toEqual(['faber', 'cadu'])
    expect(ids('pt_br')).toEqual(['faber', 'cadu'])
    expect(ids('pt-PT')).toEqual(['tugao'])
    expect(ids('pt')).toEqual(['faber', 'cadu', 'tugao'])
    expect(ids('en-US')).toEqual(['lessac'])
    expect(ids('en')).toEqual(['lessac', 'alan'])
  })
  it('accepts English, native and localized language names', () => {
    expect(ids('inglês')).toEqual(['lessac', 'alan'])
    expect(ids('English')).toEqual(['lessac', 'alan'])
    expect(ids('español')).toEqual(['davefx', 'claude'])
    expect(ids('espanhol')).toEqual(['davefx', 'claude'])
    expect(ids('français')).toEqual(['siwis'])
    expect(ids('alemão')).toEqual(['thorsten'])
    expect(ids('Deutsch')).toEqual(['thorsten'])
    expect(ids('mandarim')).toEqual(['huayan'])
  })
  it('returns no matches for unavailable languages without falling back to Faber', () => {
    expect(ids('Klingon')).toEqual([])
    expect(ids('zz-ZZ')).toEqual([])
    expect(ids('Brazilian French')).toEqual([])
  })
  it('offers missing voices as well as installed ones, without mutating either', () => {
    const before = JSON.stringify(models)
    const results = filterPiperVoices(models, 'pt-BR')
    expect(results.map((m) => m.installed)).toEqual([true, false])
    expect(JSON.stringify(models)).toBe(before)
  })
  it('counts voices per locale and excludes transcription models', () => {
    const languages = piperLanguages(models, 'pt')
    expect(languages.find((l) => l.code === 'pt-BR')).toMatchObject({ count: 2 })
    expect(languages.find((l) => l.code === 'pt-PT')).toMatchObject({ count: 1 })
    expect(languages.some((l) => l.code === 'multilingual' || l.code === 'en')).toBe(false)
    expect(languages.find((l) => l.code === 'pt-BR')?.label).toContain('português')
  })
  it('does not search voice names as if they were languages', () => {
    expect(ids('faber')).toEqual([])
    expect(ids('pt-BR')).toHaveLength(2)
  })
})
