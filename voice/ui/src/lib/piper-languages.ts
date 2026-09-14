import type { ModelInfo } from './types'

function normalized(value: string): string {
  return value.normalize('NFD').replace(/\p{M}/gu, '').toLocaleLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, ' ').trim()
}
const aliases: Record<string, string> = {
  'pt-BR': 'brasileiro brasileira brazilian', 'pt-PT': 'europeu europeia european',
  'en-US': 'americano americana american', 'en-GB': 'britanico britanica british',
  'es-MX': 'mexicano mexicana mexican', 'es-AR': 'argentino argentina argentinian',
  'zh-CN': 'mandarim mandarin chinese chines',
}
function displayName(language: string, locale: string): string {
  try { return new Intl.DisplayNames([locale], { type: 'language' }).of(language) ?? language }
  catch { return language }
}
export interface PiperLanguage { code: string; label: string; search: string; count: number }

/** Names in the UI locale, English, Portuguese, and the language itself. */
export function piperLanguages(models: readonly ModelInfo[], locale = 'en'): PiperLanguage[] {
  const counts = new Map<string, number>()
  for (const model of models) {
    if (model.kind !== 'piper_onnx') continue
    for (const language of model.languages) counts.set(language, (counts.get(language) ?? 0) + 1)
  }
  return [...counts].map(([code, count]) => {
    const label = displayName(code, locale)
    const names = [locale, 'en', 'pt', code.split('-')[0]].map((l) => displayName(code, l))
    return { code, count, label, search: normalized([code, ...names, aliases[code] ?? ''].join(' ')) }
  }).sort((a, b) => a.label.localeCompare(b.label, locale))
}

/** Empty language means ask first, not silently choose Brazilian Portuguese. */
export function filterPiperVoices(models: readonly ModelInfo[], query: string, locale = 'en'): ModelInfo[] {
  const tokens = normalized(query).split(' ').filter(Boolean)
  if (tokens.length === 0) return []
  const available = piperLanguages(models, locale)
  const codeQuery = query.trim().toLowerCase().replaceAll('_', '-')
  const isCode = available.some((l) => l.code.toLowerCase() === codeQuery || l.code.split('-')[0] === codeQuery)
  const languages = new Set(available.filter((language) => isCode
    ? language.code.toLowerCase() === codeQuery || language.code.split('-')[0] === codeQuery
    : tokens.every((token) => language.search.includes(token))).map((l) => l.code))
  return models.filter((model) => model.kind === 'piper_onnx' && model.languages.some((l) => languages.has(l)))
}
