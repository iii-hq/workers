import { describe, expect, it } from 'vitest'
import {
  type ComposerInsert,
  composerTarget,
  insertIntoComposer,
  markComposerUsed,
  mountedComposerCount,
  onComposerFocusRequest,
  onComposerInsert,
  registerComposer,
  requestComposerFocus,
} from './composer-insert'

describe('composer inserts', () => {
  it('carries a send request alongside the text', () => {
    const seen: ComposerInsert[] = []
    const off = onComposerInsert((insert) => seen.push(insert))
    insertIntoComposer('say hi', { submit: true })
    insertIntoComposer('a draft')
    off()
    expect(seen).toEqual([
      { text: 'say hi', inline: false, submit: true },
      { text: 'a draft', inline: false, submit: false },
    ])
  })

  it('buffers a send request made before a composer is listening', () => {
    insertIntoComposer('queued', { submit: true })
    const seen: ComposerInsert[] = []
    const off = onComposerInsert((insert) => seen.push(insert))
    off()
    expect(seen).toEqual([{ text: 'queued', inline: false, submit: true }])
  })
})

describe('several chats open', () => {
  it('hands an insert to one composer, never to all of them', () => {
    const offA = registerComposer('left')
    const offB = registerComposer('right')
    const seen: string[] = []
    const offInsertA = onComposerInsert(
      (i) => seen.push(`left:${i.text}`),
      'left',
    )
    const offInsertB = onComposerInsert(
      (i) => seen.push(`right:${i.text}`),
      'right',
    )

    expect(mountedComposerCount()).toBe(2)
    // Neither used yet: the first mounted.
    insertIntoComposer('one')
    // The one the person typed in last.
    markComposerUsed('right')
    insertIntoComposer('two')
    markComposerUsed('left')
    insertIntoComposer('three')

    expect(seen).toEqual(['left:one', 'right:two', 'left:three'])
    offInsertA()
    offInsertB()
    offA()
    offB()
    expect(mountedComposerCount()).toBe(0)
  })

  it('focuses the composer asked for, or the one an insert would reach', () => {
    const offA = registerComposer('left')
    const offB = registerComposer('right')
    const focused: string[] = []
    const offFocusA = onComposerFocusRequest(() => focused.push('left'), 'left')
    const offFocusB = onComposerFocusRequest(
      () => focused.push('right'),
      'right',
    )

    requestComposerFocus('right')
    markComposerUsed('left')
    requestComposerFocus()
    // `requestAnimationFrame` hands it a timestamp, not an id.
    requestComposerFocus(16.7)

    expect(focused).toEqual(['right', 'left', 'left'])
    offFocusA()
    offFocusB()
    offA()
    offB()
  })

  it('forgets a composer once it unmounts', () => {
    const off = registerComposer('gone')
    markComposerUsed('gone')
    off()
    expect(composerTarget()).toBeNull()
    markComposerUsed('gone')
    expect(mountedComposerCount()).toBe(0)
  })
})
