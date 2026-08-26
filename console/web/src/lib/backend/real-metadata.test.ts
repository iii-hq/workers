import { describe, expect, it } from 'vitest'
import {
  buildTurnMetadata,
  FALLBACK_FUNCTION_POLICY,
  toHarnessSendError,
  toProviderOptions,
  toThinkingLevel,
} from './real'

describe('buildTurnMetadata — fs_scope forwarding', () => {
  it('carries fs_scope.root when a directory is set', () => {
    expect(buildTurnMetadata('s-1', 'm-1', '/proj/A')).toEqual({
      session_id: 's-1',
      message_id: 'm-1',
      fs_scope: { root: '/proj/A' },
    })
  })

  it('omits fs_scope when null, undefined, or empty', () => {
    for (const wd of [null, undefined, ''] as const) {
      expect(buildTurnMetadata('s-1', 'm-1', wd)).toEqual({
        session_id: 's-1',
        message_id: 'm-1',
      })
    }
  })
})

describe('fallback function policy', () => {
  it('does not expose workspace picker functions to the agent', () => {
    expect(FALLBACK_FUNCTION_POLICY.deny).toContain('shell::workspace::*')
  })

  it('allows configuration updates without exposing registration', () => {
    expect(FALLBACK_FUNCTION_POLICY.deny).toContain('configuration::register')
    expect(FALLBACK_FUNCTION_POLICY.deny).not.toContain('configuration::*')
    expect(FALLBACK_FUNCTION_POLICY.deny).not.toContain('configuration::get')
    expect(FALLBACK_FUNCTION_POLICY.deny).not.toContain('configuration::set')
  })
})

describe('model reasoning effort forwarding', () => {
  it('omits the generic level for default', () => {
    expect(toThinkingLevel('default')).toBeUndefined()
    expect(toProviderOptions('openai-codex', 'default')).toBeUndefined()
  })

  it('forwards native efforts in the selected provider namespace', () => {
    expect(toThinkingLevel('ultra')).toBeUndefined()
    expect(toProviderOptions('openai-codex', 'ultra')).toEqual({
      'openai-codex': { reasoning_effort: 'ultra' },
    })
  })

  it('keeps known levels on the generic compatibility field too', () => {
    expect(toThinkingLevel('high')).toBe('high')
    expect(toProviderOptions('openai-codex', 'high')).toEqual({
      'openai-codex': { reasoning_effort: 'high' },
    })
  })
})

describe('harness send error formatting', () => {
  it('keeps the useful message from a structured SDK error', () => {
    const error = toHarnessSendError({
      code: 'invocation_failed',
      message:
        'handler error: {"code":"enqueue_error","message":"serialization error: unknown field namespace"}',
    })

    expect(error.message).toBe(
      'enqueue_error: serialization error: unknown field namespace',
    )
    expect(error.message).not.toContain('[object Object]')
  })
})
