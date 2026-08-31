import { describe, expect, it } from 'vitest'
import { HARNESS_FUNCTION_IDS, isHarnessFunction } from '../parsers'

describe('isHarnessFunction', () => {
  it('matches every id in the explicit allowlist', () => {
    for (const id of HARNESS_FUNCTION_IDS) {
      expect(isHarnessFunction(id)).toBe(true)
    }
  })

  it('rejects unrelated ids', () => {
    expect(isHarnessFunction('harness::send')).toBe(false)
    expect(isHarnessFunction('harness::')).toBe(false)
    expect(isHarnessFunction('submit_results')).toBe(false)
  })
})
