import { beforeEach, describe, expect, it } from 'vitest'
import {
  bringBrowserOverlayToFront,
  browserOverlaySessions,
  hideBrowserOverlay,
  resetBrowserOverlayStore,
  showBrowserOverlay,
  subscribeBrowserOverlay,
} from './overlay-store'

describe('browser overlay store', () => {
  beforeEach(() => resetBrowserOverlayStore())

  it('stacks started sessions with the latest in front and notifies once per change', () => {
    let notified = 0
    const off = subscribeBrowserOverlay(() => {
      notified += 1
    })
    showBrowserOverlay('s1')
    showBrowserOverlay('s1')
    showBrowserOverlay('s2')
    expect(browserOverlaySessions()).toEqual(['s1', 's2'])
    expect(notified).toBe(2)
    off()
  })

  it('brings a picked card to the front and ignores unknown ids', () => {
    showBrowserOverlay('s1')
    showBrowserOverlay('s2')
    showBrowserOverlay('s3')
    bringBrowserOverlayToFront('s1')
    expect(browserOverlaySessions()).toEqual(['s2', 's3', 's1'])
    bringBrowserOverlayToFront('nope')
    expect(browserOverlaySessions()).toEqual(['s2', 's3', 's1'])
  })

  it('hiding removes the card and ignores ids not on the deck', () => {
    showBrowserOverlay('s1')
    showBrowserOverlay('s2')
    hideBrowserOverlay('s2')
    expect(browserOverlaySessions()).toEqual(['s1'])
    let notified = 0
    const off = subscribeBrowserOverlay(() => {
      notified += 1
    })
    hideBrowserOverlay('nope')
    expect(notified).toBe(0)
    off()
  })
})
