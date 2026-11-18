import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { FilesystemAccessPrompt } from './FilesystemAccessPrompt'

const html = (onceOnly?: boolean) =>
  renderToStaticMarkup(
    <FilesystemAccessPrompt
      requestedRoot="/x"
      namespace="laptop"
      onceOnly={onceOnly}
      onResolve={() => {}}
    />,
  )

describe('FilesystemAccessPrompt', () => {
  it('offers every duration by default', () => {
    expect(html()).toContain('allow this session')
    expect(html()).toContain('always allow…')
  })

  it('offers only allow once and deny with onceOnly', () => {
    const out = html(true)
    expect(out).toContain('allow once')
    expect(out).toContain('deny')
    expect(out).not.toContain('allow this session')
    expect(out).not.toContain('always allow…')
    expect(out).toContain('lasts one call')
  })
})
