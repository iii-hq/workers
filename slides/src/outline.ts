import type { IIIClient } from 'iii-sdk'
import type { RuntimeConfig } from './config.js'
import { LAYOUTS, normalizeSlides, type Slide, text } from './model.js'
import { THEMES } from './themes.js'

export interface OutlineInput {
  topic: string
  audience?: string
  slide_count?: number
  tone?: string
  context?: string
  theme?: string
  model?: string
  provider?: string
}

export interface DeckDraft {
  title: string
  subtitle?: string
  theme?: string
  slides: Slide[]
}

export const DESIGN_SYSTEM_PROMPT = `You are a world-class presentation designer and storyteller. You turn a brief into a slide deck that an executive would call excellent.

Principles you never break:
- One idea per slide. The title states the takeaway as a full sentence, not a topic label ("Churn fell 40% after onboarding redesign", not "Churn").
- Open with a title slide, then a hook or agenda, build a narrative arc (context, tension, insight, resolution, next steps), and close with a clear ask or summary.
- Short text. At most 5 bullets per slide, at most 12 words per bullet. Prefer a metric, a quote, or a statement slide over a wall of bullets.
- Vary the rhythm: alternate content slides with section, statement, metric and two-column slides. Never put three bullet slides in a row.
- Make numbers big. Use metric blocks for key figures with a short label.
- Write speaker notes for every slide: 2 to 4 sentences the presenter would actually say, including the transition to the next slide.
- Use images only when you have a real URL supplied in the brief. Never invent image URLs.
- Plain, confident language. No filler, no jargon the audience would not use, no exclamation marks.

Output format: return ONLY a JSON object, no prose, no code fences, with this exact shape:
{
  "title": string,
  "subtitle": string,
  "theme": one of ${THEMES.map((theme) => `"${theme.id}"`).join(' | ')},
  "slides": [
    {
      "layout": one of ${LAYOUTS.map((layout) => `"${layout}"`).join(' | ')},
      "title": string,
      "subtitle": string (optional),
      "blocks": [
        { "type": "text", "text": string } |
        { "type": "bullets", "items": string[] } |
        { "type": "heading", "text": string } |
        { "type": "quote", "text": string, "attribution": string (optional) } |
        { "type": "metric", "value": string, "label": string } |
        { "type": "code", "code": string, "language": string (optional) } |
        { "type": "image", "src": string, "alt": string (optional), "caption": string (optional) }
      ],
      "notes": string
    }
  ]
}

Layout guidance: "title" for the first slide only; "section" for chapter breaks (title only); "statement" for a single bold sentence or quote; "two-column" when comparing or pairing text with a metric (give each block a "column": "left" or "right"); "content" for everything else. Metric blocks look best two or three per slide in a two-column layout.`

export function buildOutlineUserPrompt(input: OutlineInput): string {
  const count = input.slide_count ?? 10
  const lines = [
    `Create a ${count}-slide deck.`,
    `Topic: ${input.topic.trim()}`,
    input.audience ? `Audience: ${input.audience.trim()}` : 'Audience: a smart, busy general business audience',
    input.tone ? `Tone: ${input.tone.trim()}` : 'Tone: confident, clear, warm',
    input.theme
      ? `Theme: ${input.theme}`
      : `Theme: choose the best fit from the allowed list for this topic and audience`,
  ]
  if (input.context?.trim())
    lines.push('', 'Source material and constraints (use these facts, do not invent others):', input.context.trim())
  lines.push('', `Return exactly ${count} slides as JSON.`)
  return lines.join('\n')
}

export function extractAssistantText(message: unknown): string {
  const content = (message as { content?: unknown } | undefined)?.content
  if (Array.isArray(content)) {
    return content
      .filter((part) => part && typeof part === 'object' && (part as { type?: string }).type === 'text')
      .map((part) => String((part as { text?: string }).text ?? ''))
      .join('')
  }
  return typeof content === 'string' ? content : ''
}

export function parseJsonObject(raw: string): Record<string, unknown> {
  let source = raw.trim()
  const fenced = source.match(/```(?:json)?\s*\n?([\s\S]*?)\n?```/i)
  if (fenced) source = fenced[1].trim()
  const start = source.indexOf('{')
  const end = source.lastIndexOf('}')
  if (start >= 0 && end > start) source = source.slice(start, end + 1)
  try {
    const parsed = JSON.parse(source)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('not an object')
    return parsed as Record<string, unknown>
  } catch {
    throw new Error(
      `DRAFT_INVALID_JSON: the model did not return a JSON deck (${source.slice(0, 120).replace(/\s+/g, ' ')})`,
    )
  }
}

export function parseDeckDraft(raw: string, fallbackTitle: string): DeckDraft {
  const parsed = parseJsonObject(raw)
  const slides = normalizeSlides(parsed.slides)
  if (!slides.length) throw new Error('DRAFT_EMPTY: the model returned no slides')
  const theme = text(parsed.theme)
  return {
    title: text(parsed.title) ?? fallbackTitle,
    ...(text(parsed.subtitle) ? { subtitle: text(parsed.subtitle) } : {}),
    ...(theme && THEMES.some((candidate) => candidate.id === theme) ? { theme } : {}),
    slides,
  }
}

type CatalogModel = { id?: string; provider?: string; supports_tools?: boolean }

export async function resolveModel(
  iii: IIIClient,
  config: RuntimeConfig,
  input: Pick<OutlineInput, 'model' | 'provider'>,
): Promise<{ model: string; provider?: string }> {
  const explicit = text(input.model) ?? text(config.default_model)
  const provider = text(input.provider) ?? text(config.default_provider)
  if (explicit) {
    const split = explicit.match(/^([^:]+)::(.+)$/)
    if (split && !provider) return { model: split[2], provider: split[1] }
    return { model: explicit, ...(provider ? { provider } : {}) }
  }
  const response = await iii.trigger<Record<string, unknown>, { models?: CatalogModel[] }>({
    function_id: 'router::models::list',
    payload: { modality: 'chat', ...(provider ? { provider } : {}) },
    timeoutMs: 10_000,
  })
  const first = (response?.models ?? []).find((model) => model.id && model.provider)
  if (!first?.id)
    throw new Error('MODEL_UNAVAILABLE: no chat model in the router catalog; pass model or set default_model')
  return { model: first.id, ...(first.provider ? { provider: first.provider } : {}) }
}

export async function draftDeck(
  iii: IIIClient,
  config: RuntimeConfig,
  input: OutlineInput,
): Promise<DeckDraft & { model: string }> {
  const topic = text(input.topic)
  if (!topic) throw new Error('INVALID_OUTLINE: topic is required')
  const count = input.slide_count ?? 10
  if (!Number.isInteger(count) || count < 1 || count > 60)
    throw new Error('INVALID_OUTLINE: slide_count must be between 1 and 60')
  const { model, provider } = await resolveModel(iii, config, input)
  const response = await iii.trigger<Record<string, unknown>, { message?: unknown }>({
    function_id: 'router::complete',
    payload: {
      model,
      ...(provider ? { provider } : {}),
      system_prompt: DESIGN_SYSTEM_PROMPT,
      messages: [{ role: 'user', content: buildOutlineUserPrompt({ ...input, topic, slide_count: count }) }],
      max_output_tokens: config.outline_max_output_tokens,
    },
    timeoutMs: 240_000,
  })
  const draft = parseDeckDraft(extractAssistantText(response?.message), topic)
  return { ...draft, model: provider ? `${provider}::${model}` : model }
}
