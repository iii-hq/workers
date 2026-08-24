import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import {
  RegisterTriggerView,
  TriggerRegisteredDisplay,
} from '@/components/chat/engine/RegisterTriggerView'
import { SpawnActivityCard } from '@/components/chat/harness/SpawnView'
import {
  engineFunctionsListEmpty,
  engineRegisterTriggerSubscribe,
} from '@/stories/fixtures/engine-fixtures'
import { FunctionTriggerCard } from './FunctionTriggerCard'
import { FIRST_PARTY_RENDERERS } from './renderer-registry'

describe('featured first-party renderers', () => {
  it('marks only spawn and trigger registration as prominent', () => {
    const spawn = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/harness-spawn',
    )
    const register = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/engine-register-trigger',
    )
    const harnessFamily = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/harness',
    )
    const engineFamily = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/engine',
    )

    expect(spawn?.metadata).toEqual({ display: true })
    expect(spawn?.tryRenderDisplay).toBeTypeOf('function')
    expect(register?.metadata).toEqual({
      display: true,
      displayAction: 'expand',
    })
    expect(register?.tryRenderDisplay).toBeTypeOf('function')
    expect(harnessFamily?.metadata?.display).not.toBe(true)
    expect(engineFamily?.metadata?.display).not.toBe(true)
  })

  it('does not let register_trigger display metadata promote other engine calls', () => {
    const register = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/engine-register-trigger',
    )
    const engineFamily = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/engine',
    )

    expect(register?.tryRender(engineFunctionsListEmpty)).toBeNull()
    expect(engineFamily?.tryRender(engineFunctionsListEmpty)).not.toBeNull()

    const collapsed = renderToStaticMarkup(
      <FunctionTriggerCard message={engineFunctionsListEmpty} />,
    )
    const expanded = renderToStaticMarkup(
      <FunctionTriggerCard message={engineFunctionsListEmpty} defaultOpen />,
    )
    expect(collapsed).not.toContain('no functions returned')
    expect(expanded).toContain('no functions returned')
  })
})

describe('spawn activity display', () => {
  const now = Date.UTC(2026, 7, 19, 12, 2)

  it.each([
    ['active', 'Active'],
    ['working', 'Working'],
    ['thinking', 'Thinking'],
    ['messaging', 'Sending a message'],
    ['error', 'Needs attention'],
  ] as const)('renders the %s live state', (status, label) => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Review the release"
        task="Audit the release candidate and report blockers."
        status={status}
        activityAt={now - 120_000}
        now={now}
      />,
    )

    expect(html).toContain(`data-subagent-status="${status}"`)
    expect(html).toContain(label)
    expect(html).toContain('Review the release')
    expect(html).toContain('Sub-agent')
    expect(html).toContain(`${label} for 2m`)
  })

  it.each([
    ['completed', 'Completed'],
    ['stopped', 'Stopped'],
  ] as const)('renders the %s terminal state', (status, label) => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Reviewer"
        task="Audit the release candidate."
        status={status}
        activityAt={now - 120_000}
        now={now}
      />,
    )

    expect(html).toContain(label)
    expect(html).toContain(`${label} 2m ago`)
  })

  it('does not present stale activity age as disconnect duration', () => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Reviewer"
        task="Audit the release candidate."
        status="disconnected"
        activityAt={now - 10_800_000}
        now={now}
      />,
    )

    expect(html).toContain('Disconnected')
    expect(html).not.toContain('Disconnected for')
  })

  it('renders the requested metadata and the entire widget as the session destination', () => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Review the release"
        task="Audit the release candidate."
        status="working"
        sessionId="child-1"
        icon="review"
        color="purple"
        createdAt={now - 120_000}
        activityAt={now - 120_000}
        now={now}
        onOpen={vi.fn()}
      />,
    )

    expect(html.match(/<button/g)).toHaveLength(1)
    expect(html).toContain(
      'aria-label="Open Review the release sub-agent in a new panel"',
    )
    expect(html).toContain('data-color="purple"')
    expect(html).toContain('lucide-clipboard-check')
    expect(html).toContain('Open details')
    expect(html).toContain('2m ago')
    expect(html).toContain('ID: child-1')
    expect(html).not.toContain('Created by you')
    expect(html).not.toContain('aria-label="More')
  })

  it('shows the assigned task once when it is also the session title', () => {
    const task = 'Audit the release candidate.'
    const html = renderToStaticMarkup(
      <SpawnActivityCard title={task} task={task} status="working" />,
    )

    expect(html.match(/Audit the release candidate\./g)).toHaveLength(1)
  })

  it('keeps its mobile controls on one overflow-safe row below the content', () => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Review the release"
        task="Audit the release candidate."
        status="active"
        sessionId="child-1"
        onOpen={vi.fn()}
      />,
    )

    expect(html).toContain('@xl:grid-cols-[minmax(0,1fr)_auto]')
    expect(html).toContain('grid-cols-[minmax(0,1fr)_auto]')
    expect(html).toContain('min-w-0')
  })
})

describe('trigger registration display', () => {
  const now = Date.UTC(2026, 7, 20, 12, 5)

  it('makes the label, active state, creation time, and trigger id explicit', () => {
    const html = renderToStaticMarkup(
      <TriggerRegisteredDisplay
        input={{
          trigger_type: 'state',
          label: 'research-progress-watch',
          lifecycle: { once: false },
        }}
        output={{ subscription_id: 'sub-1', once: false }}
        createdAt={now - 300_000}
        now={now}
      />,
    )

    expect(html).toContain('Trigger registered')
    expect(html).toContain('research-progress-watch')
    expect(html).toContain('state · persistent')
    expect(html).toContain('data-trigger-registration-state="active"')
    expect(html).toContain('data-activity-status-tone="positive"')
    expect(html).toContain('Active for 5m')
    expect(html).toContain('5m ago')
    expect(html).toContain('ID: sub-1')
    expect(html).toContain('Open details')
    expect(html).toContain('size-6')
    expect(html).toContain('bg-ok-muted')
    expect(html).toContain('stroke-ok')
    expect(html).toContain('animate-pulse')
  })

  it('keeps action hidden until the trigger fires', () => {
    const html = renderToStaticMarkup(
      <TriggerRegisteredDisplay
        input={{
          trigger_type: 'on-message',
          label: 'explorer-messages',
          metadata: { action: 'new Explorer message received' },
        }}
        output={{ subscription_id: 'sub-1', once: false }}
      />,
    )

    expect(html).toContain('data-trigger-registration-label=""')
    expect(html).toContain('explorer-messages')
    expect(html).not.toContain('new Explorer message received')
  })

  it('omits action from readable registration details while preserving other metadata', () => {
    const html = renderToStaticMarkup(
      <RegisterTriggerView
        messageId="register-1"
        input={{
          trigger_type: 'on-message',
          label: 'explorer-messages',
          metadata: {
            action: 'new Explorer message received',
            source: 'explorer',
          },
        }}
        output={{ subscription_id: 'sub-1', once: false }}
      />,
    )

    expect(html).not.toContain('new Explorer message received')
    expect(html).toContain('Registration metadata')
    expect(html).toContain('explorer')
  })

  it('places status above open details only at the desktop container breakpoint', () => {
    const html = renderToStaticMarkup(
      <TriggerRegisteredDisplay
        input={{ trigger_type: 'cron', label: 'nightly cleanup' }}
        output={{ id: 'trg-1' }}
      />,
    )

    expect(html).toContain('grid-cols-[minmax(0,1fr)_auto]')
    expect(html).toContain('@xl:flex')
    expect(html).toContain('@xl:flex-col')
    expect(html.indexOf('role="status"')).toBeLessThan(
      html.indexOf('Open details'),
    )
  })

  it('does not claim an unsuccessful registration', () => {
    expect(
      renderToStaticMarkup(
        <TriggerRegisteredDisplay
          input={{ trigger_type: 'state', label: 'watch' }}
          output={{}}
        />,
      ),
    ).toBe('')
  })

  it('keeps the registration receipt as the primary card when details expand', () => {
    const collapsed = renderToStaticMarkup(
      <FunctionTriggerCard message={engineRegisterTriggerSubscribe} />,
    )
    const expanded = renderToStaticMarkup(
      <FunctionTriggerCard
        message={engineRegisterTriggerSubscribe}
        defaultOpen
      />,
    )

    for (const html of [collapsed, expanded]) {
      expect(html.match(/Trigger registered/g)).toHaveLength(1)
      expect(html).toContain('research-progress-watch')
      expect(html).toContain('data-trigger-registration-details')
      expect(html).toContain('data-trigger-execution-trace')
      expect(html).toContain('data-trigger-flow-card="when"')
      expect(html).toContain('data-trigger-flow-card="then"')
      expect(html).toContain('Terminal')
      expect(html).toContain('Raw JSON')
      expect(html).toContain('p-4 select-none sm:p-3')
      expect(html).not.toContain('fcall-chrome')
      expect(html).not.toContain('copy function id')
    }

    expect(collapsed).toContain('aria-expanded="false"')
    expect(collapsed).toContain('aria-hidden="true"')
    expect(expanded).toContain('aria-expanded="true"')
    expect(expanded).toContain('aria-hidden="false"')
  })
})
