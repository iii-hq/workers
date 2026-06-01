// Iterative pre-order flattening shared by the flame view. The flame tree
// can be thousands of nodes deep (long-running workflows / nested tool
// calls), so the flatten MUST NOT recurse — the transforms were already
// rewritten to an explicit stack to survive these traces.

import { describe, expect, it } from 'vitest'
import { flattenPreorder } from './treeFlatten'

interface Node {
  id: string
  children: Node[]
}

const leaf = (id: string, children: Node[] = []): Node => ({ id, children })

describe('flattenPreorder', () => {
  it('returns an empty array for no roots', () => {
    expect(flattenPreorder([])).toEqual([])
  })

  it('emits a single node', () => {
    expect(flattenPreorder([leaf('a')]).map((n) => n.id)).toEqual(['a'])
  })

  it('emits parents before children (pre-order) and preserves sibling order', () => {
    const tree = [leaf('a', [leaf('b'), leaf('c', [leaf('d')])]), leaf('e')]
    expect(flattenPreorder(tree).map((n) => n.id)).toEqual([
      'a',
      'b',
      'c',
      'd',
      'e',
    ])
  })

  it('does not blow the stack on a very deep chain', () => {
    // A recursive traversal throws RangeError around ~10k frames; build a
    // chain well past that so the regression is unambiguous.
    let root = leaf('n0')
    const head = root
    for (let i = 1; i < 50_000; i++) {
      const child = leaf(`n${i}`)
      root.children.push(child)
      root = child
    }
    expect(() => flattenPreorder([head])).not.toThrow()
    expect(flattenPreorder([head])).toHaveLength(50_000)
  })
})
