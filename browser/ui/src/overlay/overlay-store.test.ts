import { beforeEach, describe, expect, it } from 'vitest'
import {
  bringBrowserOverlayToFront,
  browserOverlaySessions,
  dismissBrowserOverlay,
  forgetBrowserOverlay,
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

  it('a dismissal removes the card and blocks a late start event for it', () => {
    showBrowserOverlay('s1')
    showBrowserOverlay('s2')
    dismissBrowserOverlay('s2')
    expect(browserOverlaySessions()).toEqual(['s1'])
    showBrowserOverlay('s2')
    expect(browserOverlaySessions()).toEqual(['s1'])
    // Dismissing before the event lands works the same way.
    dismissBrowserOverlay('s3')
    showBrowserOverlay('s3')
    expect(browserOverlaySessions()).toEqual(['s1'])
  })

  it('a stopped session leaves the deck and clears its dismissal', () => {
    showBrowserOverlay('s1')
    forgetBrowserOverlay('s1')
    expect(browserOverlaySessions()).toEqual([])
    dismissBrowserOverlay('s2')
    forgetBrowserOverlay('s2')
    showBrowserOverlay('s2')
    expect(browserOverlaySessions()).toEqual(['s2'])
  })
})
