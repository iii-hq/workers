import type { Host } from '@iii-dev/console-ui'
import type { Deck, DeckSummary, ExportFormat, Slide, Theme } from './types'

export type Api = ReturnType<typeof createApi>

export function createApi(host: Host) {
  const call = <T>(fn: string, payload: Record<string, unknown> = {}, timeoutMs = 30_000) =>
    host.iii.trigger<T>(fn, payload, { timeoutMs })
  return {
    list: () => call<{ decks: DeckSummary[] }>('slides::list'),
    get: (deck_id: string) => call<{ deck: Deck }>('slides::get', { deck_id }),
    create: (input: Record<string, unknown>) => call<{ deck: Deck }>('slides::create', input),
    update: (deck_id: string, patch: Record<string, unknown>) =>
      call<{ deck: Deck }>('slides::update', { deck_id, ...patch }),
    remove: (deck_id: string) => call<{ deleted: boolean }>('slides::delete', { deck_id }),
    themes: () => call<{ default_theme: string; themes: Theme[] }>('slides::themes::list'),
    render: (deck_id: string) => call<{ html: string; revision: number }>('slides::render', { deck_id }),
    exportDeck: (deck_id: string, format: ExportFormat) =>
      call<{ path: string; content_type: string; size: number; data_base64?: string }>(
        'slides::export',
        { deck_id, format, inline: true },
        120_000,
      ),
    outline: (input: Record<string, unknown>) => call<{ deck: Deck; model: string }>('slides::outline', input, 300_000),
    saveSlides: (deck: Deck, slides: Slide[]) =>
      call<{ deck: Deck }>('slides::update', {
        deck_id: deck.id,
        title: deck.title,
        subtitle: deck.subtitle ?? '',
        author: deck.author ?? '',
        theme: deck.theme,
        theme_overrides: deck.theme_overrides ?? {},
        motion: deck.motion ?? {},
        slides,
      }),
  }
}

export function downloadBytes(base64: string, contentType: string, filename: string) {
  const binary = atob(base64)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i)
  const url = URL.createObjectURL(new Blob([bytes], { type: contentType }))
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 5_000)
}
