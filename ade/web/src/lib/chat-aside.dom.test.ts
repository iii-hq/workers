// @vitest-environment jsdom

import { describe, expect, it } from 'vitest'
import { findTextRange } from './chat-aside'

function prose(html: string): HTMLElement {
  const root = document.createElement('div')
  root.innerHTML = html
  return root
}

describe('findTextRange', () => {
  it('finds a passage that crosses inline elements', () => {
    const root = prose(
      '<p>The views stay <strong>in the browser</strong>, per pane.</p>',
    )
    expect(findTextRange(root, 'stay in the browser, per')?.toString()).toBe(
      'stay in the browser, per',
    )
  })

  it('ignores the newlines a selection adds between blocks', () => {
    const root = prose('<p>First paragraph.</p><ul><li>One item</li></ul>')
    expect(findTextRange(root, 'paragraph.\n\nOne item')?.toString()).toBe(
      'paragraph.One item',
    )
  })

  it('returns null when the text is not there', () => {
    expect(findTextRange(prose('<p>abc</p>'), '**abc**')).toBeNull()
    expect(findTextRange(prose('<p>abc</p>'), '   ')).toBeNull()
  })
})
