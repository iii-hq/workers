import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Markdown } from './markdown'

describe('Markdown mention pills', () => {
  it('renders /skill:<id> as a command pill that hides the namespace', () => {
    const html = renderToStaticMarkup(
      <Markdown>{'run /skill:coder/index now.'}</Markdown>,
    )
    expect(html).toContain('data-slash-command="/skill:coder/index"')
    expect(html).toContain('>coder/index</span>')
    expect(html).not.toContain('skill:coder/index</span>')
    expect(html).toContain(' now.')
  })

  it('keeps a trailing period out of the id', () => {
    const html = renderToStaticMarkup(
      <Markdown>{'use /skill:review-pr.'}</Markdown>,
    )
    expect(html).toContain('data-slash-command="/skill:review-pr"')
    expect(html).toContain('</span>.')
  })

  it('leaves a literal inside code and a slash inside a word alone', () => {
    const html = renderToStaticMarkup(
      <Markdown>{'`/skill:x` and either/skill:y'}</Markdown>,
    )
    expect(html).not.toContain('data-slash-command')
  })

  it('still renders function and file mentions', () => {
    const html = renderToStaticMarkup(
      <Markdown>{'call @fn(engine::echo) in #file(src/a.ts)'}</Markdown>,
    )
    expect(html).toContain('data-function-id="engine::echo"')
    expect(html).toContain('data-file-path="src/a.ts"')
  })
})
