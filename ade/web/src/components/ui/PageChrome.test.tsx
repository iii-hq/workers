import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { PaneConfigurationProvider } from '@/lib/pane-configuration'
import { PageHeader } from './PageChrome'
import { TooltipProvider } from './Tooltip'

function renderHeader(configurationId?: string) {
  return renderToStaticMarkup(
    <TooltipProvider>
      <PaneConfigurationProvider configurationId={configurationId}>
        <PageHeader title="Worker" />
      </PaneConfigurationProvider>
    </TooltipProvider>,
  )
}

describe('PageHeader worker configuration action', () => {
  it('adds the console-owned configuration action for a configured page', () => {
    expect(renderHeader('browser')).toContain('aria-label="Configure worker"')
  })

  it('does not add a configuration action without page metadata', () => {
    expect(renderHeader()).not.toContain('aria-label="Configure worker"')
  })
})
