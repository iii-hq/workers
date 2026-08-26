import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import {
  formatModelWaitElapsed,
  ModelWaitingIndicator,
} from './ModelWaitingIndicator'

describe('ModelWaitingIndicator', () => {
  it('renders the animated wordmark as a named live status', () => {
    const html = renderToStaticMarkup(
      <ModelWaitingIndicator label="dispatching model" />,
    )

    expect(html).toContain('role="status"')
    expect(html).toContain('aria-label="dispatching model"')
    expect(html.match(/model-waiting-wordmark-segment/g)).toHaveLength(3)
    expect(html).toContain('0.0s')
  })

  it('formats elapsed seconds and minutes', () => {
    expect(formatModelWaitElapsed(12_345)).toBe('12.3s')
    expect(formatModelWaitElapsed(61_250)).toBe('1m 1.2s')
  })
})
