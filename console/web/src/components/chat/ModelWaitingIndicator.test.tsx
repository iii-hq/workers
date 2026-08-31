import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  formatModelWaitElapsed,
  ModelWaitingIndicator,
  modelWaitElapsedMs,
} from './ModelWaitingIndicator'

describe('ModelWaitingIndicator', () => {
  afterEach(() => vi.useRealTimers())

  it('renders the animated wordmark as a named live status', () => {
    const html = renderToStaticMarkup(
      <ModelWaitingIndicator label="dispatching model" />,
    )

    expect(html).toContain('role="status"')
    expect(html).toContain('aria-label="dispatching model"')
    expect(html).toContain('data-active="true"')
    expect(html.match(/model-waiting-wordmark-segment/g)).toHaveLength(3)
    expect(html).toContain('0.0s')
  })

  it('formats elapsed seconds and minutes', () => {
    expect(formatModelWaitElapsed(12_345)).toBe('12.3s')
    expect(formatModelWaitElapsed(61_250)).toBe('1m 1.2s')
  })

  it('keeps an inactive root mounted without exposing a live status', () => {
    const html = renderToStaticMarkup(
      <ModelWaitingIndicator active={false} label="waiting for model" />,
    )

    expect(html).toContain('data-model-waiting=""')
    expect(html).toContain('data-active="false"')
    expect(html).toContain('aria-hidden="true"')
    expect(html).toContain('0.0s')
  })

  it('uses the fake clock deterministically for elapsed time', () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-08-28T12:00:00.000Z'))
    const startedAt = Date.now()

    vi.advanceTimersByTime(12_345)

    expect(formatModelWaitElapsed(modelWaitElapsedMs(startedAt))).toBe('12.3s')
  })
})
