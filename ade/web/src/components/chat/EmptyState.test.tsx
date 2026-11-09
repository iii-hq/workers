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
    expect(html).toContain('Connect a model')
    expect(html).toContain('Set up a model')
    expect(html).not.toContain('What would you like to build?')
  })

  it('offers a start and an update for a harness that is declared but stopped', () => {
    const html = renderToStaticMarkup(<EmptyState variant="harness-stopped" />)
    expect(html).toContain('Start the harness worker')
    expect(html).toContain('Start harness')
    expect(html).toContain('Update and start harness')
    expect(html).toContain('compose::up container=harness')
    expect(html).not.toContain('Add harness')
  })

  it('names the compose action its console runs or failed', () => {
    const starting = renderToStaticMarkup(
      <EmptyState variant="installing" action="start" />,
    )
    expect(starting).toContain('Starting harness…')
    expect(starting).toContain('compose::up container=harness')
    expect(starting).not.toContain('Installing harness')

    const updateFailed = renderToStaticMarkup(
      <EmptyState variant="install-failed" action="update" />,
    )
    expect(updateFailed).toContain('Harness update failed')
    expect(updateFailed).toContain('Retry update')
    expect(updateFailed).toContain('compose::update worker=harness')

    // The default action is still the add.
    const adding = renderToStaticMarkup(<EmptyState variant="installing" />)
    expect(adding).toContain('Installing harness…')
    expect(adding).toContain('compose::add worker=harness')
  })
})
