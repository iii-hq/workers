import {
  type Block,
  type Deck,
  insertSlide,
  MAX_BLOCKS,
  mergeSlide,
  newId,
  normalizeBlock,
  normalizeSlide,
  reorderSlides,
  type Slide,
  slideIndex,
  text,
} from './model.js'

export interface BlockEdit {
  block_id?: string
  block?: unknown
  index?: number
  after_block_id?: string
  block_ids?: string[]
}

export function blockIndex(slide: Slide, blockId: unknown): number {
  const id = text(blockId)
  if (!id) throw new Error('INVALID_BLOCK: block_id is required')
  const index = slide.blocks.findIndex((block) => block.id === id)
  if (index < 0) throw new Error(`BLOCK_NOT_FOUND: ${id}`)
  return index
}

export function insertBlock(slide: Slide, edit: BlockEdit): { slide: Slide; block: Block } {
  if (slide.blocks.length >= MAX_BLOCKS) throw new Error(`INVALID_SLIDE: a slide supports at most ${MAX_BLOCKS} blocks`)
  const block = normalizeBlock(edit.block ?? { type: 'text', text: '' }, slide.blocks.length)
  block.id = newId('block')
  const after = text(edit.after_block_id)
  const at = after
    ? blockIndex(slide, after) + 1
    : Math.max(0, Math.min(slide.blocks.length, edit.index ?? slide.blocks.length))
  const blocks = [...slide.blocks]
  blocks.splice(at, 0, block)
  return { slide: { ...slide, blocks }, block }
}

export function updateBlock(slide: Slide, edit: BlockEdit): Slide {
  const index = blockIndex(slide, edit.block_id)
  if (!edit.block || typeof edit.block !== 'object' || Array.isArray(edit.block))
    throw new Error('INVALID_BLOCK: block must be an object')
  const current = slide.blocks[index]
  const patch = edit.block as Record<string, unknown>
  const merged = patch.type && patch.type !== current.type ? { ...patch } : { ...current, ...patch }
  const blocks = [...slide.blocks]
  blocks[index] = normalizeBlock({ ...merged, id: current.id }, index)
  return { ...slide, blocks }
}

export function removeBlock(slide: Slide, edit: BlockEdit): Slide {
  const index = blockIndex(slide, edit.block_id)
  return { ...slide, blocks: slide.blocks.filter((_, candidate) => candidate !== index) }
}

export function reorderBlocks(slide: Slide, blockIds: unknown): Slide {
  if (!Array.isArray(blockIds)) throw new Error('INVALID_ORDER: block_ids must be an array')
  const byId = new Map(slide.blocks.map((block) => [block.id, block]))
  const seen = new Set<string>()
  const ordered: Block[] = []
  for (const id of blockIds) {
    const block = byId.get(String(id))
    if (!block) throw new Error(`BLOCK_NOT_FOUND: ${String(id)}`)
    if (seen.has(block.id)) throw new Error(`INVALID_ORDER: ${block.id} listed twice`)
    seen.add(block.id)
    ordered.push(block)
  }
  for (const block of slide.blocks) if (!seen.has(block.id)) ordered.push(block)
  return { ...slide, blocks: ordered }
}

export function withSlide(deck: Deck, slideId: string, slide: Slide): Deck {
  const index = slideIndex(deck, slideId)
  return { ...deck, slides: deck.slides.map((current, candidate) => (candidate === index ? slide : current)) }
}

export const OPERATIONS = [
  'block.insert',
  'block.update',
  'block.remove',
  'block.reorder',
  'slide.update',
  'slide.insert',
  'slide.remove',
  'slide.reorder',
] as const
export type OperationKind = (typeof OPERATIONS)[number]

export interface DeckOperation extends BlockEdit {
  op: OperationKind
  slide_id?: string
  slide?: unknown
  after_slide_id?: string
  slide_ids?: string[]
}

export interface AppliedOperation {
  op: OperationKind
  slide_id: string
  block_id?: string
}

function slideOf(deck: Deck, slideId: unknown): Slide {
  const id = text(slideId)
  if (!id) throw new Error('INVALID_OPERATION: slide_id is required')
  return deck.slides[slideIndex(deck, id)]
}

export function applyOperations(deck: Deck, ops: unknown): { deck: Deck; applied: AppliedOperation[] } {
  if (!Array.isArray(ops) || !ops.length) throw new Error('INVALID_OPERATION: ops must be a non-empty array')
  let next = deck
  const applied: AppliedOperation[] = []
  for (const [position, raw] of ops.entries()) {
    if (!raw || typeof raw !== 'object') throw new Error(`INVALID_OPERATION: ops[${position}] must be an object`)
    const op = raw as DeckOperation
    if (!(OPERATIONS as readonly string[]).includes(op.op))
      throw new Error(`INVALID_OPERATION: ops[${position}].op must be one of ${OPERATIONS.join(', ')}`)
    if (op.op === 'slide.insert') {
      const slide = normalizeSlide(op.slide ?? { layout: 'content', title: 'New slide' })
      slide.id = newId('slide')
      const index = text(op.after_slide_id) ? slideIndex(next, op.after_slide_id as string) + 1 : op.index
      next = { ...next, slides: insertSlide(next, slide, index) }
      applied.push({ op: op.op, slide_id: slide.id })
      continue
    }
    if (op.op === 'slide.reorder') {
      if (!Array.isArray(op.slide_ids)) throw new Error('INVALID_ORDER: slide_ids must be an array')
      next = { ...next, slides: reorderSlides(next, op.slide_ids) }
      applied.push({ op: op.op, slide_id: op.slide_ids[0] ?? '' })
      continue
    }
    const slide = slideOf(next, op.slide_id)
    if (op.op === 'slide.remove') {
      next = { ...next, slides: next.slides.filter((candidate) => candidate.id !== slide.id) }
      applied.push({ op: op.op, slide_id: slide.id })
    } else if (op.op === 'slide.update') {
      next = withSlide(next, slide.id, mergeSlide(slide, op.slide ?? {}))
      applied.push({ op: op.op, slide_id: slide.id })
    } else if (op.op === 'block.insert') {
      const result = insertBlock(slide, op)
      next = withSlide(next, slide.id, result.slide)
      applied.push({ op: op.op, slide_id: slide.id, block_id: result.block.id })
    } else if (op.op === 'block.update') {
      next = withSlide(next, slide.id, updateBlock(slide, op))
      applied.push({ op: op.op, slide_id: slide.id, block_id: text(op.block_id) })
    } else if (op.op === 'block.remove') {
      next = withSlide(next, slide.id, removeBlock(slide, op))
      applied.push({ op: op.op, slide_id: slide.id, block_id: text(op.block_id) })
    } else {
      next = withSlide(next, slide.id, reorderBlocks(slide, op.block_ids))
      applied.push({ op: op.op, slide_id: slide.id })
    }
  }
  return { deck: next, applied }
}
