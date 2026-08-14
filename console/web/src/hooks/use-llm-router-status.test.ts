import { describe, expect, it } from 'vitest'
import {
  isLlmRouterAvailable,
  LLM_ROUTER_WATCH_FN,
  LLM_ROUTER_WORKER_NAME,
} from './use-llm-router-status'

describe('llm-router presence probe wiring', () => {
  it('probes the llm-router worker with its own watch handler id', () => {
    // The model picker's router::* reads gate on THIS worker — gating on the
    // harness blanked the picker whenever the harness was slow or absent.
    expect(LLM_ROUTER_WORKER_NAME).toBe('llm-router')
    // Must be unique per presence probe so the browser-local `worker` trigger
    // handlers never collide (shell uses console::shell-watch).
    expect(LLM_ROUTER_WATCH_FN).toBe('console::llm-router-watch')
  })

  it('gates on both presence and the initial probe settling', () => {
    expect(isLlmRouterAvailable({ present: true, loading: false })).toBe(true)
    expect(isLlmRouterAvailable({ present: true, loading: true })).toBe(false)
    expect(isLlmRouterAvailable({ present: false, loading: false })).toBe(false)
  })
})
