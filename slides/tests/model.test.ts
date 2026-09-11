import { describe, expect, it } from 'vitest'
import {
  deckToMarkdown,
  insertSlide,
  isDeck,
  mergeSlide,
  normalizeBlock,
  normalizeSlide,
  normalizeSlides,
  reorderSlides,
  slidesFromMarkdown,
  summarize,
} from '../src/model.js'
import { sampleDeck } from './support/sample.js'

describe('normalizeBlock', () => {
  it('accepts every block type and generates ids', () => {
    expect(normalizeBlock({ type: 'text', text: ' hello ' })).toMatchObject({ type: 'text', text: 'hello' })
    expect(normalizeBlock({ type: 'bullets', items: ['a', ' b ', ''] })).toMatchObject({ items: ['a', 'b'] })
    expect(normalizeBlock({ type: 'bullets', text: '- one\n- two' })).toMatchObject({ items: ['one', 'two'] })
    expect(normalizeBlock({ type: 'metric', value: '42', label: 'answers', column: 'right' })).toMatchObject({
      column: 'right',
    })
    expect(normalizeBlock({ type: 'heading', text: 'h' }).id).toMatch(/^block-/)
  })

  it('rejects unknown types and images without a source', () => {
    expect(() => normalizeBlock({ type: 'video' })).toThrow(/unknown type/)
    expect(() => normalizeBlock({ type: 'image' })).toThrow(/needs a src/)
    expect(() => normalizeBlock('nope')).toThrow(/expected an object/)
  })
})

describe('normalizeSlide', () => {
  it('defaults the layout and lifts shorthand bullets and body', () => {
    const slide = normalizeSlide({ title: 'T', bullets: ['x', 'y'], body: 'para', layout: 'weird' })
    expect(slide.layout).toBe('content')
    expect(slide.blocks.map((block) => block.type)).toEqual(['bullets', 'text'])
  })

  it('dedupes slide ids across a deck', () => {
    const slides = normalizeSlides([{ id: 'same' }, { id: 'same' }])
    expect(new Set(slides.map((slide) => slide.id)).size).toBe(2)
  })

  it('caps deck size', () => {
    expect(() => normalizeSlides(Array.from({ length: 201 }, () => ({})))).toThrow(/at most 200/)
  })
})

describe('slide operations', () => {
  it('inserts at an index and clamps out-of-range values', () => {
    const deck = sampleDeck()
    const slide = normalizeSlide({ id: 'new', title: 'New' })
    expect(insertSlide(deck, slide, 1).map((s) => s.id)[1]).toBe('new')
    expect(
      insertSlide(deck, slide, 99)
        .map((s) => s.id)
        .at(-1),
    ).toBe('new')
    expect(
      insertSlide(deck, slide)
        .map((s) => s.id)
        .at(-1),
    ).toBe('new')
  })

  it('reorders listed slides and appends the rest', () => {
    const deck = sampleDeck()
    expect(reorderSlides(deck, ['slide-c', 'slide-a']).map((s) => s.id)).toEqual([
      'slide-c',
      'slide-a',
      'slide-b',
      'slide-d',
      'slide-e',
    ])
    expect(() => reorderSlides(deck, ['slide-zz'])).toThrow(/SLIDE_NOT_FOUND/)
    expect(() => reorderSlides(deck, ['slide-a', 'slide-a'])).toThrow(/listed twice/)
  })

  it('merges a slide patch and clears fields with empty strings', () => {
    const current = sampleDeck().slides[1]
    const merged = mergeSlide(current, { title: 'Renamed', notes: '' })
    expect(merged.title).toBe('Renamed')
    expect(merged.notes).toBeUndefined()
    expect(merged.blocks).toEqual(current.blocks)
    expect(mergeSlide(current, { blocks: [{ type: 'text', text: 'only' }] }).blocks).toHaveLength(1)
  })

  it('summarizes and validates decks', () => {
    const deck = sampleDeck()
    expect(summarize(deck)).toEqual({
      id: 'deck-test0001',
      title: 'Quarterly review',
      subtitle: 'Q3 in twelve minutes',
      theme: 'midnight',
      slide_count: 5,
      revision: 3,
      updated_at_ms: 2,
    })
    expect(isDeck(deck)).toBe(true)
    expect(isDeck({ id: 'x' })).toBe(false)
  })
})

describe('markdown import', () => {
  const markdown = [
    '# Launch plan',
    'What ships in October',
    '---',
    '## Three bets',
    '- Faster onboarding',
    '- Usage-based pricing',
    '- Partner API',
    'Notes: keep this to one minute',
    '---',
    '<!-- layout: section -->',
    '## Timeline',
    '---',
    '> Make it work, make it right, make it fast.',
    '> \u2014 Kent Beck',
    '---',
    '## Snippet',
    '```ts',
    'const a = 1',
    '```',
    '![arch](https://example.com/a.png)',
    '<!-- notes: explain the diagram -->',
  ].join('\n')

  it('splits slides, infers layouts and captures notes', () => {
    const result = slidesFromMarkdown(markdown)
    expect(result.title).toBe('Launch plan')
    expect(result.subtitle).toBe('What ships in October')
    expect(result.slides.map((slide) => slide.layout)).toEqual(['title', 'content', 'section', 'statement', 'content'])
    expect(result.slides[1].blocks[0]).toMatchObject({
      type: 'bullets',
      items: ['Faster onboarding', 'Usage-based pricing', 'Partner API'],
    })
    expect(result.slides[1].notes).toBe('keep this to one minute')
    expect(result.slides[3].blocks[0]).toMatchObject({ type: 'quote', attribution: 'Kent Beck' })
    expect(result.slides[4].blocks.map((block) => block.type)).toEqual(['code', 'image'])
    expect(result.slides[4].notes).toBe('explain the diagram')
  })

  it('round-trips through deckToMarkdown', () => {
    const deck = sampleDeck()
    const back = slidesFromMarkdown(deckToMarkdown(deck))
    expect(back.slides).toHaveLength(deck.slides.length)
    expect(back.slides[1].blocks[0]).toMatchObject({
      type: 'bullets',
      items: ['Enterprise ARR up 31%', 'Churn down to 2.1%', 'Two new regions live'],
    })
    expect(back.slides[2].layout).toBe('two-column')
  })
})
