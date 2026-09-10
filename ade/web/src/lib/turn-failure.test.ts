import { describe, expect, it } from 'vitest'
import {
  categorizeTurnFailure,
  classifyTurnFailure,
  SEND_FAILED_CODE,
} from './turn-failure'

describe('categorizeTurnFailure', () => {
  it('reads expired credentials from the harness class or router code', () => {
    expect(
      categorizeTurnFailure({
        code: 'llm.auth_expired',
        class: 'llm.auth_expired',
        text: 'upstream 401',
      }),
    ).toBe('auth')
    expect(
      categorizeTurnFailure({
        code: 'router/provider_auth_expired',
        text: 'Authentication for provider "openai" needs attention.',
      }),
    ).toBe('auth')
  })

  it('reads a billing wall out of a generic permanent rejection', () => {
    expect(
      categorizeTurnFailure({
        code: 'router/provider_rejected',
        class: 'llm.permanent',
        text: 'openai responses: credit balance exhausted',
      }),
    ).toBe('billing')
    expect(
      categorizeTurnFailure({
        code: 'llm.permanent',
        class: 'llm.permanent',
        text: 'anthropic messages: 400 invalid_request_error: Your credit balance is too low to access the Anthropic API.',
      }),
    ).toBe('billing')
  })

  it('treats a quota 429 as billing, not as a rate limit', () => {
    expect(
      categorizeTurnFailure({
        code: 'router/provider_rate_limited',
        class: 'llm.rate_limited',
        text: 'insufficient_quota: You exceeded your current quota, please check your plan and billing details.',
      }),
    ).toBe('billing')
    expect(
      categorizeTurnFailure({
        code: 'router/provider_rate_limited',
        class: 'llm.rate_limited',
        text: 'Provider "openai" is busy right now.',
      }),
    ).toBe('rate-limit')
  })

  it('maps stream and transient codes to a dropped connection', () => {
    expect(
      categorizeTurnFailure({ code: 'router/stream_incomplete', text: '' }),
    ).toBe('connection')
    expect(
      categorizeTurnFailure({
        code: 'llm.transient',
        class: 'llm.transient',
        text: 'stream ended without a terminal frame',
      }),
    ).toBe('connection')
    expect(
      categorizeTurnFailure({
        code: 'router/provider_unavailable',
        text: 'provider zai unavailable',
      }),
    ).toBe('connection')
  })

  it('maps router setup codes to configuration and overflow to context', () => {
    expect(
      categorizeTurnFailure({ code: 'router/not_configured', text: '' }),
    ).toBe('configuration')
    expect(
      categorizeTurnFailure({ code: 'router/no_provider_for_model', text: '' }),
    ).toBe('configuration')
    expect(
      categorizeTurnFailure({ code: 'router/context_overflow', text: '' }),
    ).toBe('context')
    expect(
      categorizeTurnFailure({
        class: 'llm.context_overflow',
        text: 'prompt is too long',
      }),
    ).toBe('context')
  })

  it('blames iii for harness-internal codes and kickoff failures', () => {
    expect(
      categorizeTurnFailure({
        code: 'harness.turn_internal',
        text: 'state::put_turn failed',
      }),
    ).toBe('internal')
    expect(
      categorizeTurnFailure({
        text: 'response failed: harness::send failed — trigger timed out',
      }),
    ).toBe('internal')
  })

  it('recognises the console send failure code', () => {
    expect(
      categorizeTurnFailure({
        code: SEND_FAILED_CODE,
        text: 'send failed — socket closed',
      }),
    ).toBe('send')
  })

  it('falls back to unknown when nothing identifies the cause', () => {
    expect(
      categorizeTurnFailure({ text: 'The response could not be completed.' }),
    ).toBe('unknown')
  })
})

describe('classifyTurnFailure', () => {
  it('names the owner and the provider in plain words for a credentials failure', () => {
    const presentation = classifyTurnFailure({
      content: 'The provider authentication needs attention.',
      technicalDetails: {
        code: 'router/provider_auth_expired',
        class: 'llm.auth_expired',
        detail: '401 invalid_api_key',
        provider: 'openai',
        model: 'gpt-5.4',
      },
      nextActions: ['Update the provider credentials in LLM Router settings.'],
    })
    expect(presentation.category).toBe('auth')
    expect(presentation.owner).toBe('user')
    expect(presentation.ownerLabel).toBe('Needs your attention')
    expect(presentation.title).toBe('Provider credentials rejected')
    expect(presentation.ownership).toContain('openai')
    expect(presentation.ownership).toContain('not with iii or the console')
    // Console steps are more specific than the harness's generic advice here.
    expect(presentation.actions[0]).toContain('openai')
    expect(presentation.actions[0]).toContain('Configure provider')
  })

  it('keeps the harness next actions for transport failures', () => {
    const presentation = classifyTurnFailure({
      content: 'The provider disconnected before completing the response.',
      failure: {
        summary: 'The provider disconnected before completing the response.',
      },
      technicalDetails: {
        code: 'router/stream_incomplete',
        class: 'llm.transient',
        detail: 'stream ended without a terminal frame',
        provider: 'zai',
      },
      nextActions: ['Retry the turn to continue.'],
    })
    expect(presentation.category).toBe('connection')
    expect(presentation.owner).toBe('environment')
    expect(presentation.ownerLabel).toBe('Can happen · retry')
    expect(presentation.title).toBe('Connection to the provider dropped')
    expect(presentation.ownership).toContain('zai')
    expect(presentation.ownership).toContain('not caused by anything you did')
    expect(presentation.actions).toEqual(['Retry the turn to continue.'])
  })

  it('describes a billing wall as the provider account, not iii', () => {
    const presentation = classifyTurnFailure({
      content: 'The provider rejected this request.',
      technicalDetails: {
        code: 'router/provider_rejected',
        class: 'llm.permanent',
        detail: 'Your credit balance is too low to access the Anthropic API.',
        provider: 'anthropic',
      },
      nextActions: [
        'Review the selected model and provider settings, then try again.',
      ],
    })
    expect(presentation.category).toBe('billing')
    expect(presentation.title).toBe('Provider credit or quota exhausted')
    expect(presentation.ownership).toContain('anthropic account')
    expect(presentation.ownership).toContain('not an iii or console failure')
    expect(presentation.actions[0]).toMatch(/Add credit/)
  })

  it('owns internal failures as iii errors', () => {
    const presentation = classifyTurnFailure({
      content: 'turn failed',
      technicalDetails: {
        code: 'harness.turn_internal',
        detail: 'store unavailable',
      },
    })
    expect(presentation.owner).toBe('iii')
    expect(presentation.ownerLabel).toBe('iii error')
    expect(presentation.title).toBe('iii could not complete the turn')
  })
})
