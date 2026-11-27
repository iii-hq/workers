import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { PermissionModePicker } from './PermissionModePicker'

vi.mock('@/hooks/use-media-query', () => ({
  DESKTOP_POINTER_QUERY: '(hover: hover) and (pointer: fine)',
  useMediaQuery: () => false,
}))

const GEAR = 'Configure the Auto mode allowlist'

describe('PermissionModePicker', () => {
  it('shows the mode as a capitalised pill', () => {
    const html = renderToStaticMarkup(
      <PermissionModePicker value="manual" onChange={() => {}} />,
    )
    expect(html).toContain('Manual')
    expect(html).not.toContain(GEAR)
  })

  it('carries a gear to the allowlist only in Auto mode', () => {
    const html = renderToStaticMarkup(
      <PermissionModePicker value="auto" onChange={() => {}} />,
    )
    expect(html).toContain('Auto')
    expect(html).toContain(GEAR)
  })
})
