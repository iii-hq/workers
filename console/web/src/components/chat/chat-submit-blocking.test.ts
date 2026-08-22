import { describe, expect, it } from 'vitest'
import { isSessionSubmitBlockedByHydration } from './chat-submit-blocking'

describe('isSessionSubmitBlockedByHydration', () => {
  it('blocks a materialized real session until its transcript is hydrated', () => {
    expect(
      isSessionSubmitBlockedByHydration({
        realBackend: true,
        hydrated: false,
      }),
    ).toBe(true)
  })

  it('allows a hydrated user-only retry and local or mock drafts', () => {
    expect(
      isSessionSubmitBlockedByHydration({
        realBackend: true,
        draft: false,
        hydrated: true,
      }),
    ).toBe(false)
    expect(
      isSessionSubmitBlockedByHydration({
        realBackend: true,
        draft: true,
        hydrated: false,
      }),
    ).toBe(false)
    expect(
      isSessionSubmitBlockedByHydration({
        realBackend: false,
        draft: false,
        hydrated: false,
      }),
    ).toBe(false)
  })
})
