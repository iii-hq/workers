import { describe, expect, it } from 'vitest'
import { buildViewOptions } from './nav-options'

describe('buildViewOptions', () => {
  it('lists the first-party views (traces + workers); injected pages are appended by the caller', () => {
    expect(buildViewOptions().map((o) => o.value)).toEqual([
      'traces',
      'workers',
    ])
  })
})
