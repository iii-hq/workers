import { describe, expect, it } from 'vitest'
import { markdownTree } from './markdown'

describe('markdownTree', () => {
  it('parses a settled reply once and reuses the tree', () => {
    const text = `# cached ${Math.random()}\n\nsome **bold** text`
    expect(markdownTree(text, true)).toBe(markdownTree(text, true))
  })

  it('does not keep a reply that is still streaming', () => {
    const text = `streaming ${Math.random()}`
    expect(markdownTree(text, false)).not.toBe(markdownTree(text, false))
  })
})
