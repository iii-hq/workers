import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { SegmentedControl } from './ModeToggle'
import { TooltipProvider } from './Tooltip'

describe('SegmentedControl', () => {
  it('uses shared line tabs and semantic icons by default', () => {
    const html = renderToStaticMarkup(
      <SegmentedControl
        value="data"
        onChange={() => undefined}
        options={[
          { value: 'data', label: 'Data' },
          { value: 'sql', label: 'SQL' },
        ]}
        aria-label="Database panel"
      />,
    )

    expect(html).toContain('role="tablist"')
    expect(html).toContain('iii-ui-tabs-list')
    expect(html).toContain('iii-ui-tab__icon')
    expect(html).toContain('iii-ui-icon')
    expect(html).toContain('aria-selected="true"')
  })

  it('keeps persistent choices segmented and icon-free', () => {
    const html = renderToStaticMarkup(
      <SegmentedControl
        value="dark"
        onChange={() => undefined}
        variant="radio"
        options={[
          { value: 'light', label: 'Light' },
          { value: 'dark', label: 'Dark' },
        ]}
        aria-label="Theme"
      />,
    )

    expect(html).toContain('role="radiogroup"')
    expect(html).toContain('iii-ui-segmented')
    expect(html).not.toContain('iii-ui-tab__icon')
  })

  it('renders icon-only tabs with accessible labels and tooltip triggers', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider delayDuration={0}>
        <SegmentedControl
          value="skills"
          onChange={() => undefined}
          iconOnly
          options={[
            { value: 'skills', label: 'Skills' },
            { value: 'agents', label: 'Agent Profiles' },
          ]}
          aria-label="Directory collection"
        />
      </TooltipProvider>,
    )

    expect(html).toContain('data-icon-only="true"')
    expect(html).toContain('aria-label="Skills"')
    expect(html).toContain('aria-label="Agent Profiles"')
    expect(html).toContain('data-state="closed"')
    expect(html).not.toContain('<span>Skills</span>')
  })
})
