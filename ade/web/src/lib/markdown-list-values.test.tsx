// @vitest-environment jsdom
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { markdownTree } from './markdown'

function values(source: string, selector: string): (string | null)[] {
  const container = document.createElement('div')
  container.innerHTML = renderToStaticMarkup(markdownTree(source, false))
  return Array.from(container.querySelectorAll<HTMLLIElement>(selector)).map(
    (li) => li.getAttribute('value'),
  )
}

describe('ordered list numbering', () => {
  it('keeps the numbers the author typed', () => {
    expect(values('3. first\n6. second\n', 'ol > li')).toEqual(['3', '6'])
  })

  it('leaves bullets alone', () => {
    expect(values('- a\n- b\n', 'ul > li')).toEqual([null, null])
  })
})
