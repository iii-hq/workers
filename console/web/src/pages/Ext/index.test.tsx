import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { ExtPage } from '.'

describe('ExtPage missing-worker state', () => {
  it('keeps the hosting panel close action visible', () => {
    const markup = renderToStaticMarkup(
      <ExtPage pageId="memory" onMissing={vi.fn()} onRequestClose={vi.fn()} />,
    )

    expect(markup).toContain('aria-label="close panel"')
    expect(markup).toContain('waiting for worker')
    expect(markup).toContain('extension page not loaded')
  })
})
