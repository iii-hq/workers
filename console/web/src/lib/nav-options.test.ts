import { describe, expect, it } from 'vitest'
import { buildViewOptions } from './nav-options'

describe('buildViewOptions', () => {
  it('hides the worktrees entry while the worker is absent', () => {
    expect(buildViewOptions(false).map((o) => o.value)).toEqual([
      'traces',
      'traces-v2',
      'workers',
    ])
  })

  it('appends the worktrees entry when the worker is present', () => {
    expect(buildViewOptions(true).map((o) => o.value)).toEqual([
      'traces',
      'traces-v2',
      'workers',
      'worktrees',
    ])
  })
})
