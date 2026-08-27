import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import {
  RegisterTriggerView,
  TriggerRegisteredDisplay,
} from '@/components/chat/engine/RegisterTriggerView'
import {
  engineFunctionsListEmpty,
  engineRegisterTriggerSubscribe,
} from '@/stories/fixtures/engine-fixtures'
import { FunctionTriggerCard } from './FunctionTriggerCard'
import { FIRST_PARTY_RENDERERS } from './renderer-registry'

describe('featured first-party renderers', () => {
  it('marks only trigger registration as prominent', () => {
    const register = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/engine-register-trigger',
    )
    const harnessFamily = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/harness',
    )
    const engineFamily = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/engine',
    )

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
