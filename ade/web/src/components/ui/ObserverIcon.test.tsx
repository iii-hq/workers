import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ObserverIcon } from './ObserverIcon'

describe('ObserverIcon', () => {
  it('forwards Lucide-style size and color props to the filled SVG', () => {
    const html = renderToStaticMarkup(
      <ObserverIcon size={16} color="tomato" className="fill-warn" />,
    )

    expect(html).toContain('width="16"')
    expect(html).toContain('height="16"')
    expect(html).toContain('color="tomato"')
    expect(html).toContain('class="fill-warn"')
    expect(html).toContain('fill="currentColor"')
  })
})
