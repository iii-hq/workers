import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { EmptyState } from './EmptyState'

describe('EmptyState', () => {
  it('introduces the task without another project or profile selector', () => {
    const html = renderToStaticMarkup(
      <EmptyState variant="ready" workingDir="/workspace/console" />,
    )
    expect(html).toContain('What would you like to build?')
    expect(html).toContain(
      'Explore your project, fix an issue, or start something new.',
    )
    expect(html).not.toContain('session setup')
    expect(html).not.toContain('working directory')
    expect(html).not.toContain('Configure an agent manually')
  })

  it('keeps provider setup available when there is no model', () => {
    const html = renderToStaticMarkup(<EmptyState variant="no-provider" />)
    expect(html).toContain('Connect a model.')
    expect(html).toContain('set up a model')
    expect(html).not.toContain('What would you like to build?')
  })
})
