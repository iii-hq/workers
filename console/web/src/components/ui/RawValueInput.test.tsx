import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { RawValueInput } from './RawValueInput'

describe('RawValueInput', () => {
  it('keeps the raw value visible and makes literal replacement explicit', () => {
    const html = renderToStaticMarkup(
      <RawValueInput
        label="Maximum connections"
        kind="environment"
        value="${DATABASE_POOL_SIZE}"
        replacementLabel="10"
        onChange={() => {}}
        onUseLiteral={() => {}}
      />,
    )

    expect(html).toContain('Environment')
    expect(html).toContain(`value="\${DATABASE_POOL_SIZE}"`)
    expect(html).toContain('>Use 10</button>')
    expect(html).toContain(
      'aria-label="Replace Maximum connections raw value with a literal value"',
    )
  })
})
