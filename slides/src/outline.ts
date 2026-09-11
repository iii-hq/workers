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

const THEME_IDS = THEMES.map((theme) => `"${theme.id}"`).join(' | ')
const LAYOUT_IDS = LAYOUTS.map((layout) => `"${layout}"`).join(' | ')

export const DESIGN_SYSTEM_PROMPT = [
  'You are a world-class presentation designer, editor and storyteller. You turn a brief into a deck that a demanding founder, board or keynote audience would call exceptional: a real narrative, sharp language, and strong visual rhythm.',
  '',
  'NARRATIVE',
  '- Read the whole brief and find its spine: the change it argues for, the tension it resolves, the ask at the end. Structure the deck as a story, not a table of contents: open, hook, context, the core idea, how it works, proof or examples, what happens next, the close.',
  '- Every slide title is a complete sentence that states the takeaway ("Breakthroughs now happen between disciplines, not inside them"), never a topic label ("Overview"). Keep titles under 14 words.',
  "- Use the brief's own vocabulary, names, numbers and structure. Never invent facts, numbers, quotes or URLs. If the brief has no numbers, do not fabricate metrics; use statements, steps and cards instead.",
  '- One idea per slide. Text is short: at most 5 bullets, at most 12 words per bullet, card text under 16 words, step text under 14 words.',
  '- Speaker notes on every slide: 2 to 4 spoken sentences that add what the slide does not say, ending with the transition to the next slide.',
  '',
  'VISUAL RHYTHM (this is what makes the deck look designed)',
  '- Slide 1: layout "title", variant "gradient", a kicker (the institution, event or series name), a big title (under 8 words) and a one-sentence subtitle.',
  '- Chapter breaks: layout "section", variant "accent", with a kicker like "Part 02" and a short chapter title. Use 2 to 4 of them in decks of 10+ slides.',
  '- Lists of 3 to 9 parallel things (pillars, capabilities, principles, teams, offerings) are a "cards" block, one entry per thing with a 1 to 3 word title and one line of text; set numbered true when order matters. Never render parallel lists as bullets.',
  '- Processes, pipelines and loops (A -> B -> C) are a "steps" block with 3 to 6 entries.',
  '- Phases, roadmaps and eras are a "timeline" block with 3 to 6 entries.',
  '- Numbers are "metric" blocks, two or three per slide in a "two-column" layout with column set on each block.',
  '- Comparisons and contrasts are "two-column": left the old way or problem, right the new way or answer, using headings and short text or bullets with column set.',
  '- The single most important line of the deck gets its own "statement" slide with variant "gradient" or "muted". Quotes with a real attribution also go on statement slides as a quote block.',
  '- Never put two bullet slides in a row, never three card slides in a row. Alternate density: after a dense cards slide put a statement, section or two-column slide.',
  '- Use a kicker on most content slides: a 1 to 3 word label that names the chapter or theme ("Operating model", "Pillar 4", "The loop").',
  '- Vary variants: most slides default; accent for section breaks and at most one other slide; gradient for the title, the closing and one statement; muted for one or two calm explanatory slides.',
  '- Close with a "title" or "statement" slide, variant "gradient", carrying the ask or the one line to remember, with a kicker like "Next" or "The ask".',
  '- Images only when the brief gives a real URL.',
  '',
  'OUTPUT: return ONLY a JSON object, no prose, no code fences, exactly this shape:',
  '{',
  '  "title": string,',
  '  "subtitle": string,',
  `  "theme": one of ${THEME_IDS},`,
  '  "slides": [',
  '    {',
  `      "layout": one of ${LAYOUT_IDS},`,
  '      "variant": one of "default" | "accent" | "gradient" | "muted" (optional),',
  '      "kicker": string (optional, 1 to 3 words),',
  '      "title": string,',
  '      "subtitle": string (optional, one sentence),',
  '      "blocks": [',
  '        { "type": "text", "text": string } |',
  '        { "type": "bullets", "items": string[] } |',
  '        { "type": "heading", "text": string, "column": "left" | "right" (optional) } |',
  '        { "type": "quote", "text": string, "attribution": string (optional) } |',
  '        { "type": "metric", "value": string, "label": string, "column": "left" | "right" } |',
  '        { "type": "cards", "entries": [{ "title": string, "text": string }], "numbered": boolean (optional) } |',
  '        { "type": "steps", "entries": [{ "title": string, "text": string (optional) }] } |',
  '        { "type": "timeline", "entries": [{ "title": string, "text": string }] } |',
  '        { "type": "code", "code": string, "language": string (optional) } |',
  '        { "type": "image", "src": string, "alt": string (optional), "caption": string (optional) }',
  '      ],',
  '      "notes": string',
  '    }',
  '  ]',
  '}',
  '',
  'Theme guidance: dark themes (midnight, aurora, forest) for launches, visions, technical and investor decks; light themes (paper, slate, sunrise) for boards, reports, education and workshops. Prefer aurora for ambitious visions, midnight for technical depth, paper for editorial essays.',
].join('\n')

export function buildOutlineUserPrompt(input: OutlineInput): string {
  const count = input.slide_count ?? 12
  const lines = [
    `Create a ${count}-slide deck. Every slide must earn its place; the deck should read as one argument.`,
    `Topic: ${input.topic.trim()}`,
    input.audience ? `Audience: ${input.audience.trim()}` : 'Audience: a smart, busy general business audience',
    input.tone ? `Tone: ${input.tone.trim()}` : 'Tone: confident, clear, warm',
    input.theme
      ? `Theme: ${input.theme}`
      : 'Theme: choose the best fit from the allowed list for this topic and audience',
  ]
  if (input.context?.trim()) {
    lines.push(
      '',
      'Source material and constraints (use these facts, names and structure; do not invent others):',
      input.context.trim(),
    )
  }
  lines.push(
    '',
    `Return exactly ${count} slides as JSON. Use cards for the parallel lists in the material, steps for its processes and loops, a section slide for each chapter, one statement slide for its deepest thesis, and a gradient closing slide.`,
  )
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
  const count = input.slide_count ?? 12
  if (!Number.isInteger(count) || count < 1 || count > 60)
    throw new Error('INVALID_OUTLINE: slide_count must be between 1 and 60')
  const { model, provider } = await resolveModel(iii, config, input)
  const response = await iii.trigger<Record<string, unknown>, { message?: unknown }>({
    function_id: 'router::complete',
    payload: {
      model,
      ...(provider ? { provider } : {}),
      system_prompt: DESIGN_SYSTEM_PROMPT,
      messages: [
        {
          role: 'user',
          content: [{ type: 'text', text: buildOutlineUserPrompt({ ...input, topic, slide_count: count }) }],
          timestamp: Date.now(),
        },
      ],
      max_output_tokens: config.outline_max_output_tokens,
    },
    timeoutMs: 600_000,
  })
  const draft = parseDeckDraft(extractAssistantText(response?.message), topic)
  return { ...draft, model: provider ? `${provider}::${model}` : model }
}
