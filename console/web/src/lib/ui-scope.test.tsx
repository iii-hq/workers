import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ExtensionScopeProvider, PortalScope } from './ui-scope'

describe('PortalScope', () => {
  it('carries an injectable worker scope to portalled descendants', () => {
    const html = renderToStaticMarkup(
      <ExtensionScopeProvider scope="memory">
        <PortalScope>
          <div className="worker-popup">content</div>
        </PortalScope>
      </ExtensionScopeProvider>,
    )
    expect(html).toContain('data-iii-ui="memory"')
    expect(html).toContain('class="worker-popup"')
  })

  it('adds no wrapper for native Console content', () => {
    const html = renderToStaticMarkup(
      <PortalScope>
        <div>content</div>
      </PortalScope>,
    )
    expect(html).toBe('<div>content</div>')
  })
})
