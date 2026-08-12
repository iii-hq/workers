import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { EmptyState } from './EmptyState'

describe('EmptyState', () => {
  it('shrinks and scrolls while keeping short content centered', () => {
    const html = renderToStaticMarkup(<EmptyState variant="ready" />)

    expect(html).toContain('min-h-0 overflow-y-auto')
    expect(html).toContain('my-auto')
  })
})
