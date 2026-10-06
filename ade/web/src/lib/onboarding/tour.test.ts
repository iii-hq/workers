import { beforeEach, describe, expect, it, vi } from 'vitest'

const harness = vi.hoisted(() => ({
  registered: false,
  add: vi.fn(async () => ({ note: 'onboarding running' })),
  wait: vi.fn(async () => true),
}))

vi.mock('@/lib/ui-slots', () => ({
  getExtPage: (id: string) =>
    harness.registered && id === 'onboarding' ? { id } : undefined,
  whenExtPage: harness.wait,
}))
vi.mock('./api', () => ({ addWorkersWithProgress: harness.add }))

import { prepareTour, TOUR_PAGE_TIMEOUT_MS } from './tour'

beforeEach(() => {
  harness.registered = false
  harness.add.mockClear()
  harness.wait.mockClear()
})

describe('prepareTour', () => {
  it('adds the onboarding worker and waits for its page', async () => {
    await prepareTour()
    expect(harness.add).toHaveBeenCalledWith(
      ['onboarding'],
      expect.any(Function),
    )
    expect(harness.wait).toHaveBeenCalledWith(
      'onboarding',
      TOUR_PAGE_TIMEOUT_MS,
    )
  })

  it('adds nothing when the tour page is already there', async () => {
    harness.registered = true
    await prepareTour()
    expect(harness.add).not.toHaveBeenCalled()
    expect(harness.wait).not.toHaveBeenCalled()
  })

  it('passes a failed add on, for the step to show', async () => {
    harness.add.mockRejectedValueOnce(new Error('compose is not running'))
    await expect(prepareTour()).rejects.toThrow('compose is not running')
    expect(harness.wait).not.toHaveBeenCalled()
  })
})
