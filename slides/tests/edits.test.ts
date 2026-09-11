import { describe, expect, it } from 'vitest'
import { applyOperations, insertBlock, removeBlock, reorderBlocks, updateBlock, withSlide } from '../src/edits.js'
import { sampleDeck } from './support/sample.js'

describe('block edits', () => {
  it('inserts at the end, at an index and after a block', () => {
    const slide = sampleDeck().slides[1]
    const end = insertBlock(slide, { block: { type: 'text', text: 'tail' } })
    expect(end.slide.blocks.map((block) => block.id)).toEqual(['block-1', 'block-2', end.block.id])
    expect(end.block.id).toMatch(/^block-/)
    const first = insertBlock(slide, { block: { type: 'heading', text: 'head' }, index: 0 })
    expect(first.slide.blocks[0].type).toBe('heading')
    const after = insertBlock(slide, { block: { type: 'metric', value: '1', label: 'one' }, after_block_id: 'block-1' })
    expect(after.slide.blocks[1].type).toBe('metric')
    expect(slide.blocks).toHaveLength(2)
  })

  it('updates a block in place, keeps its id and rejects unknown ids', () => {
    const slide = sampleDeck().slides[1]
    const next = updateBlock(slide, { block_id: 'block-1', block: { items: ['only one'] } })
    expect(next.blocks[0]).toEqual({ id: 'block-1', type: 'bullets', items: ['only one'] })
    const retyped = updateBlock(slide, { block_id: 'block-2', block: { type: 'quote', text: 'q', attribution: 'a' } })
    expect(retyped.blocks[1]).toEqual({ id: 'block-2', type: 'quote', text: 'q', attribution: 'a' })
    expect(() => updateBlock(slide, { block_id: 'nope', block: {} })).toThrow(/BLOCK_NOT_FOUND/)
    expect(() => updateBlock(slide, { block_id: 'block-1', block: 'x' })).toThrow(/INVALID_BLOCK/)
  })

  it('removes and reorders blocks', () => {
    const slide = sampleDeck().slides[1]
    expect(removeBlock(slide, { block_id: 'block-2' }).blocks.map((block) => block.id)).toEqual(['block-1'])
    expect(reorderBlocks(slide, ['block-2']).blocks.map((block) => block.id)).toEqual(['block-2', 'block-1'])
    expect(() => reorderBlocks(slide, ['block-2', 'block-2'])).toThrow(/INVALID_ORDER/)
    expect(() => reorderBlocks(slide, ['missing'])).toThrow(/BLOCK_NOT_FOUND/)
  })

  it('replaces one slide in a deck', () => {
    const deck = sampleDeck()
    const next = withSlide(deck, 'slide-b', { ...deck.slides[1], title: 'Changed' })
    expect(next.slides[1].title).toBe('Changed')
    expect(deck.slides[1].title).not.toBe('Changed')
    expect(() => withSlide(deck, 'slide-z', deck.slides[0])).toThrow(/SLIDE_NOT_FOUND/)
  })
})

describe('apply operations', () => {
  it('runs a mixed batch in order and reports touched ids', () => {
    const deck = sampleDeck()
    const { deck: next, applied } = applyOperations(deck, [
      { op: 'block.update', slide_id: 'slide-b', block_id: 'block-2', block: { text: 'Mid-market flat.' } },
      { op: 'block.insert', slide_id: 'slide-b', block: { type: 'metric', value: '3', label: 'regions' }, index: 0 },
      { op: 'slide.update', slide_id: 'slide-c', slide: { notes: 'Two numbers.' } },
      { op: 'slide.insert', after_slide_id: 'slide-a', slide: { layout: 'section', title: 'Agenda' } },
      { op: 'slide.remove', slide_id: 'slide-e' },
      { op: 'slide.reorder', slide_ids: ['slide-d'] },
    ])
    expect(applied.map((entry) => entry.op)).toEqual([
      'block.update',
      'block.insert',
      'slide.update',
      'slide.insert',
      'slide.remove',
      'slide.reorder',
    ])
    expect(applied[1].block_id).toMatch(/^block-/)
    expect(next.slides.map((slide) => slide.id)).toEqual([
      'slide-d',
      'slide-a',
      applied[3].slide_id,
      'slide-b',
      'slide-c',
    ])
    const slideB = next.slides.find((slide) => slide.id === 'slide-b')
    expect(slideB?.blocks.map((block) => block.type)).toEqual(['metric', 'bullets', 'text'])
    expect(next.slides.find((slide) => slide.id === 'slide-c')?.notes).toBe('Two numbers.')
    expect(deck.slides).toHaveLength(5)
  })

  it('rejects empty batches and unknown operations without partial results', () => {
    const deck = sampleDeck()
    expect(() => applyOperations(deck, [])).toThrow(/INVALID_OPERATION/)
    expect(() => applyOperations(deck, [{ op: 'slide.explode', slide_id: 'slide-a' }])).toThrow(/INVALID_OPERATION/)
    expect(() =>
      applyOperations(deck, [
        { op: 'slide.update', slide_id: 'slide-a', slide: { title: 'x' } },
        { op: 'block.remove', slide_id: 'slide-b', block_id: 'missing' },
      ]),
    ).toThrow(/BLOCK_NOT_FOUND/)
  })
})
