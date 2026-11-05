import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ModelMark } from './ModelMark'

const SVG = '<svg viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>'

describe('ModelMark', () => {
  it('paints the serving provider’s mark, so an API key and a subscription differ', () => {
    const api = renderToStaticMarkup(
      <ModelMark
        modelId="anthropic::claude-opus-5-5"
        modelLabel="Claude Opus 5.5"
        providerId="anthropic"
        providerLabel="Anthropic"
        providerIconSvg={SVG}
      />,
    )
    expect(api).toContain('data-provider-icon="mark"')
    expect(api).not.toContain('data-model-mark')
  })

  it('falls back to the family mark when the provider declared none', () => {
    const html = renderToStaticMarkup(
      <ModelMark
        modelId="claude-opus-5-5"
        modelLabel="claude-opus-5-5"
        providerLabel="claude-opus-5-5"
      />,
    )
    expect(html).toContain('data-model-mark="claude"')
  })
})
