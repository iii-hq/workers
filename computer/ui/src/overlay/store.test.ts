// Run: node --test src/overlay/store.test.ts (Node >= 23.6 strips types).
import assert from 'node:assert/strict'
import { beforeEach, describe, it } from 'node:test'
import {
  bringToFront,
  claimOnPage,
  dismissCard,
  forgetSession,
  hideCard,
  markBlind,
  overlaySessions,
  resetOverlayStore,
  showCard,
} from './store.ts'

describe('computer overlay store', () => {
  beforeEach(() => resetOverlayStore())

  it('stacks the newest activity in front and drops a stopped session', () => {
    showCard('c1')
    showCard('c2')
    assert.deepEqual(overlaySessions(), ['c1', 'c2'])
    showCard('c1')
    assert.deepEqual(overlaySessions(), ['c2', 'c1'])
    bringToFront('c2')
    assert.deepEqual(overlaySessions(), ['c1', 'c2'])
    hideCard('c2')
    assert.deepEqual(overlaySessions(), ['c1'])
  })

  it('never covers a session the page shows live, until the page lets go', () => {
    showCard('c1')
    const release = claimOnPage('c1')
    assert.deepEqual(overlaySessions(), [], 'the page took the card over')
    showCard('c1', true)
    assert.deepEqual(overlaySessions(), [], 'its own viewport input must not pop it')
    release()
    showCard('c1')
    assert.deepEqual(overlaySessions(), ['c1'])
  })

  it('keeps a session with no picture off the deck until it stops or a run retries it', () => {
    showCard('wayland')
    showCard('x11')
    markBlind('wayland')
    assert.deepEqual(overlaySessions(), ['x11'], 'the blind card no longer hides the deck')
    showCard('wayland')
    assert.deepEqual(overlaySessions(), ['x11'], 'plain activity leaves it off')
    showCard('wayland', true)
    assert.deepEqual(overlaySessions(), ['x11', 'wayland'], 'a new run retries a capture error')
    markBlind('wayland')
    forgetSession('wayland')
    showCard('wayland')
    assert.deepEqual(overlaySessions(), ['x11', 'wayland'], 'a new session under that id may show')
  })

  it('keeps a hidden card hidden while activity continues, unless a run forces it', () => {
    dismissCard('c1', 0)
    showCard('c1', false, 60_000)
    assert.deepEqual(overlaySessions(), [], 'act within two minutes respects the hide')
    // Each ignored activity restarts the quiet window.
    showCard('c1', false, 170_000)
    assert.deepEqual(overlaySessions(), [])
    showCard('c1', false, 300_000)
    assert.deepEqual(overlaySessions(), ['c1'], 'two quiet minutes later it comes back')
    dismissCard('c1', 400_000)
    showCard('c1', true, 400_001)
    assert.deepEqual(overlaySessions(), ['c1'], 'a new run brings it back at once')
  })
})
