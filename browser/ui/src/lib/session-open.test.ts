import { describe, expect, it } from 'vitest'
import { shouldOpenBrowserSession } from './session-open'

const CONSOLE = 'http://127.0.0.1:3113'

describe('shouldOpenBrowserSession', () => {
  it('opens for an outside page', () => {
    expect(shouldOpenBrowserSession('https://iii.dev/docs', CONSOLE)).toBe(true)
    expect(
      shouldOpenBrowserSession('file:///tmp/demo/index.html', CONSOLE),
    ).toBe(true)
  })

  it('stays quiet for sessions pointed at the console itself', () => {
    expect(shouldOpenBrowserSession('http://127.0.0.1:3113/', CONSOLE)).toBe(
      false,
    )
    expect(
      shouldOpenBrowserSession('http://127.0.0.1:3113/#/workers', CONSOLE),
    ).toBe(false)
  })

  it('stays quiet without a url, opens on an unparsable one', () => {
    expect(shouldOpenBrowserSession(undefined, CONSOLE)).toBe(false)
    expect(shouldOpenBrowserSession('not a url', CONSOLE)).toBe(true)
  })
})
