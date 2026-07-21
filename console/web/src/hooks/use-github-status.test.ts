import { describe, expect, it } from 'vitest'
import {
  GITHUB_WATCH_FN,
  GITHUB_WORKER_NAME,
  isGithubAvailable,
} from './use-github-status'

describe('github presence probe wiring', () => {
  it('probes the github worker with its own watch handler id', () => {
    expect(GITHUB_WORKER_NAME).toBe('github')
    expect(GITHUB_WATCH_FN).toBe('console::github-watch')
  })

  it('gates on both presence and the initial probe settling', () => {
    expect(isGithubAvailable({ present: true, loading: false })).toBe(true)
    expect(isGithubAvailable({ present: true, loading: true })).toBe(false)
    expect(isGithubAvailable({ present: false, loading: false })).toBe(false)
  })
})
