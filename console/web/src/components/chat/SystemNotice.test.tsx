import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { SystemMessage } from '@/types/chat'
import { Message } from './Message'
import { SystemNotice } from './SystemNotice'

const html = (message: SystemMessage) =>
  renderToStaticMarkup(<SystemNotice message={message} />)

describe('SystemNotice · working directory', () => {
  const changed: SystemMessage = {
    id: 'wd-1',
    role: 'system',
    kind: 'working-dir',
    tone: 'info',
    content:
      'working directory changed to /work/harness — applies to the messages that follow',
    scope: {
      path: '/work/harness',
      previousPath: '/work',
      cause: 'selected',
    },
    createdAt: 0,
  }

  it('renders as an activity row with the folder trail, not a stripe', () => {
    const out = html(changed)
    expect(out).toContain('data-message-role="working-dir"')
    expect(out).toContain('data-timeline-activity-kind="working-dir"')
    expect(out).toContain('stroke-workdir')
    // The noun is a separate span so phones can drop it for the folder glyph.
    expect(out).toContain('Working directory </span>changed to')
    expect(out).toContain('/work/harness')
    expect(out).not.toContain('uppercase')
    expect(out).not.toContain('border-l-2')
  })

  it('lays out the before/after scope in the disclosure', () => {
    const out = html(changed)
    expect(out).toContain('Before')
    expect(out).toContain('/work<')
    expect(out).toContain('Now')
    expect(out).toContain('Earlier turns keep the scope they ran in.')
  })

  it('marks a vanished scope with a warning status', () => {
    const out = html({
      ...changed,
      content:
        'working directory /work/tmp is no longer available; this session is now unscoped — applies to the messages that follow',
      scope: { path: null, previousPath: '/work/tmp', cause: 'unavailable' },
    })
    expect(out).toContain('data-working-dir-cause="unavailable"')
    expect(out).toContain('data-status="error"')
    expect(out).toContain('Working directory </span>gone')
    expect(out).toContain('unscoped')
    expect(out).toContain('Working directory unavailable')
  })

  it('is what Message renders for the working-dir kind', () => {
    expect(renderToStaticMarkup(<Message message={changed} />)).toContain(
      'data-message-role="working-dir"',
    )
  })
})

describe('SystemNotice · turn failure', () => {
  it('leads with who has to act for a credentials failure', () => {
    const out = html({
      id: 'e_t1_error',
      role: 'system',
      kind: 'turn-failure',
      tone: 'error',
      content: 'The provider authentication needs attention.',
      failure: { summary: 'The provider authentication needs attention.' },
      nextActions: ['Update the provider credentials in LLM Router settings.'],
      technicalDetails: {
        code: 'router/provider_auth_expired',
        class: 'llm.auth_expired',
        detail: '401 invalid_api_key: Incorrect API key provided',
        provider: 'openai',
        model: 'gpt-5.4',
      },
      createdAt: 0,
    })
    expect(out).toContain('data-message-role="turn-failure"')
    expect(out).toContain('data-failure-category="auth"')
    expect(out).toContain('data-failure-owner="user"')
    expect(out).toContain('Provider credentials rejected')
    expect(out).toContain('Needs your attention')
    expect(out).toContain('not with iii or the console')
    expect(out).toContain('What you can do')
    expect(out).toContain('Technical details')
    expect(out).toContain('router/provider_auth_expired')
    expect(out).toContain('Incorrect API key provided')
    expect(out).not.toContain('uppercase')
  })

  it('presents a dropped stream as something that can happen', () => {
    const out = html({
      id: 'e_t2_error',
      role: 'system',
      kind: 'turn-failure',
      tone: 'error',
      content:
        'The provider disconnected before completing the response. A partial response was preserved in this conversation and may be incomplete.',
      failure: {
        summary: 'The provider disconnected before completing the response.',
        retryable: true,
        partialResultAvailable: true,
        recoveryAttempted: 1,
        recoveryMaxAttempts: 1,
      },
      technicalDetails: {
        code: 'router/stream_incomplete',
        class: 'llm.transient',
        detail: 'stream ended without a terminal frame',
        provider: 'zai',
        model: 'glm-5',
      },
      createdAt: 0,
    })
    expect(out).toContain('data-failure-owner="environment"')
    expect(out).toContain('Connection to the provider dropped')
    expect(out).toContain('Can happen · retry')
    expect(out).toContain('Retryable')
    expect(out).toContain('kept as evidence')
    expect(out).toContain('retried automatically 1 of 1 time')
    expect(out).toContain('bg-warn-muted')
  })

  it('renders the durable provider-family failure the Console e2e asserts on', () => {
    // Mirrors e2e/provider-family-errors.spec.ts: a permanent rejection whose
    // detail names a billing wall, recorded with the transport code.
    const out = html({
      id: 'e_t4_error',
      role: 'system',
      kind: 'turn-failure',
      tone: 'error',
      content: 'The provider rejected this request.',
      failure: { summary: 'The provider rejected this request.' },
      nextActions: [
        'Review the selected model and provider settings, then try again.',
      ],
      technicalDetails: {
        code: 'invocation_failed',
        class: 'llm.permanent',
        detail: 'openai chat completions: insufficient quota',
        provider: 'openai',
        model: 'gpt-5.4',
      },
      createdAt: 0,
    })
    expect(out).toContain('data-failure-category="billing"')
    expect(out).toContain('data-failure-owner="user"')
    expect(out).toMatch(/<h3[^>]*>Provider credit or quota exhausted<\/h3>/)
    expect(out).toContain('Needs your attention')
    expect(out).toMatch(
      /data-message-summary[^>]*>The provider rejected this request\.</,
    )
    expect(out).toMatch(
      /data-failure-ownership[^>]*>[^<]*not an iii or console failure/,
    )
    expect(out.match(/<li>/g)).toHaveLength(2)
    expect(out).toMatch(/<li>Add credit/)
    expect(out).toMatch(/data-technical-detail="code"[^>]*>invocation_failed</)
    expect(out).toMatch(/data-technical-detail="class"[^>]*>llm\.permanent</)
    expect(out).toMatch(
      /data-technical-detail="detail"[^>]*>openai chat completions: insufficient quota</,
    )
    // Collapsed by default; the e2e opens it by clicking the summary.
    expect(out).not.toMatch(/<details[^>]*\sopen/)
  })

  it('classifies a live fallback from its summary alone', () => {
    const out = html({
      id: 'e_t3_error',
      role: 'system',
      kind: 'turn-failure',
      tone: 'error',
      content: 'response failed: harness::send failed — trigger timed out',
      failure: { summary: 'harness::send failed — trigger timed out' },
      provisional: true,
      createdAt: 0,
    })
    expect(out).toContain('data-failure-owner="iii"')
    expect(out).toContain('iii could not complete the turn')
    expect(out).toContain('iii error')
  })
})

describe('SystemNotice · one-line notices', () => {
  it('splits an authored "headline — detail" notice on a tinted row', () => {
    const out = html({
      id: 'n-1',
      role: 'system',
      content: 'could not attach spec.pdf — file exceeds the 20 MB limit',
      tone: 'warn',
      createdAt: 0,
    })
    expect(out).toContain('data-message-role="system-notice"')
    expect(out).toContain('data-message-tone="warn"')
    expect(out).toContain('bg-warn-muted')
    expect(out).toContain('Could not attach spec.pdf')
    expect(out).toContain('file exceeds the 20 MB limit')
    expect(out).not.toContain('uppercase')
    expect(out).not.toContain('border-l-2')
  })

  it('renders an operational error as a status row, not a diagnosis card', () => {
    const out = html({
      id: 'n-2',
      role: 'system',
      kind: 'notice',
      content: 'could not unregister the trigger — subscription not found',
      tone: 'error',
      createdAt: 0,
    })
    expect(out).toContain('data-message-role="system-notice"')
    expect(out).toContain('bg-alert-muted')
    expect(out).not.toContain('data-message-role="turn-failure"')
  })

  it('keeps slash commands as authored in the headline', () => {
    const out = html({
      id: 'n-3',
      role: 'system',
      content: '/compact not supported by this backend.',
      tone: 'error',
      createdAt: 0,
    })
    expect(out).toContain('/compact not supported by this backend.')
  })
})
