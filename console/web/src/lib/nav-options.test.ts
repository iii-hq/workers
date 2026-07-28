import { describe, expect, it } from 'vitest'
import { buildViewOptions } from './nav-options'

describe('buildViewOptions', () => {
  it('hides the optional-worker entries while their workers are absent', () => {
    expect(buildViewOptions(false, false).map((o) => o.value)).toEqual([
      'traces',
      'workers',
    ])
  })

  it('appends the browser entry when the worker is present', () => {
    expect(buildViewOptions(true, false).map((o) => o.value)).toEqual([
      'traces',
      'workers',
      'browser',
    ])
  })

  it('appends the github entry when the worker is present', () => {
    expect(buildViewOptions(false, true).map((o) => o.value)).toEqual([
      'traces',
      'workers',
      'github',
    ])
  })

  it('appends every entry when all optional workers are present', () => {
    expect(buildViewOptions(true, true).map((o) => o.value)).toEqual([
      'traces',
      'workers',
      'browser',
      'github',
    ])
  })
})
