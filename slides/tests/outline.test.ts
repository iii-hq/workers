import { describe, expect, it } from 'vitest'
import {
  buildOutlineUserPrompt,
  extractAssistantText,
  parseDeckDraft,
  parseJsonObject,
  resolveModel,
} from '../src/outline.js'

const config = {
  output_dir: '~/.iii/slides',
  default_theme: 'midnight',
  default_model: '',
  default_provider: '',
  outline_max_output_tokens: 8000,
  brand_accent: '',
  brand_footer: '',
}

describe('outline parsing', () => {
  it('extracts text from string and block-array messages', () => {
    expect(extractAssistantText({ content: 'hi' })).toBe('hi')
    expect(
      extractAssistantText({
        content: [{ type: 'text', text: 'a' }, { type: 'tool_use' }, { type: 'text', text: 'b' }],
      }),
    ).toBe('ab')
    expect(extractAssistantText(undefined)).toBe('')
  })

  it('parses fenced and prose-wrapped JSON', () => {
    expect(parseJsonObject('```json\n{"a":1}\n```')).toEqual({ a: 1 })
    expect(parseJsonObject('Here you go: {"a":{"b":2}} thanks')).toEqual({ a: { b: 2 } })
    expect(() => parseJsonObject('not json')).toThrow(/DRAFT_INVALID_JSON/)
  })

  it('normalizes a draft into slides and validates the theme', () => {
    const draft = parseDeckDraft(
      JSON.stringify({
        title: 'T',
        theme: 'aurora',
        slides: [
          { layout: 'title', title: 'T' },
          { title: 'Point', blocks: [{ type: 'bullets', items: ['a'] }], notes: 'say a' },
        ],
      }),
      'fallback',
    )
    expect(draft.title).toBe('T')
    expect(draft.theme).toBe('aurora')
    expect(draft.slides).toHaveLength(2)
    expect(draft.slides[1].notes).toBe('say a')
    expect(parseDeckDraft('{"slides":[{}], "theme": "nope"}', 'fallback')).toMatchObject({ title: 'fallback' })
    expect(parseDeckDraft('{"slides":[{}], "theme": "nope"}', 'fallback').theme).toBeUndefined()
    expect(() => parseDeckDraft('{"slides":[]}', 'x')).toThrow(/DRAFT_EMPTY/)
  })

  it('builds a brief that carries every input', () => {
    const prompt = buildOutlineUserPrompt({
      topic: 'Q3',
      audience: 'board',
      slide_count: 8,
      tone: 'crisp',
      context: 'ARR 4.2M',
      theme: 'paper',
    })
    for (const needle of ['8-slide', 'Topic: Q3', 'Audience: board', 'Tone: crisp', 'ARR 4.2M', 'Theme: paper'])
      expect(prompt).toContain(needle)
  })
})

describe('resolveModel', () => {
  const fakeClient = (models: Array<{ id: string; provider: string }>) =>
    ({ trigger: async () => ({ models }) }) as unknown as Parameters<typeof resolveModel>[0]

  it('prefers the explicit model, then the configured default, then the catalog', async () => {
    expect(await resolveModel(fakeClient([]), config, { model: 'm1', provider: 'p1' })).toEqual({
      model: 'm1',
      provider: 'p1',
    })
    expect(await resolveModel(fakeClient([]), config, { model: 'p2::m2' })).toEqual({ model: 'm2', provider: 'p2' })
    expect(await resolveModel(fakeClient([]), { ...config, default_model: 'm3' }, {})).toEqual({ model: 'm3' })
    expect(await resolveModel(fakeClient([{ id: 'm4', provider: 'p4' }]), config, {})).toEqual({
      model: 'm4',
      provider: 'p4',
    })
    await expect(resolveModel(fakeClient([]), config, {})).rejects.toThrow(/MODEL_UNAVAILABLE/)
  })
})
