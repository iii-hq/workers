import { describe, expect, it } from 'vitest'
import {
  type ComposerInsert,
  insertIntoComposer,
  onComposerInsert,
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
