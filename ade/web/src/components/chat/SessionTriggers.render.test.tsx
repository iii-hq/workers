import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import { clearAllDescription, SessionTriggers } from './SessionTriggers'

function trigger(overrides: Partial<SessionTriggerInfo>): SessionTriggerInfo {
  return {
    id: 'sub_1',
    triggerType: 'state',
    delivery: { kind: 'notify' },
    config: { scope: 'review', key: 'agent-1/findings' },
    once: true,
    ...overrides,
  }
}

/** The rendered copy, tags stripped — the header noun spans a breakpoint. */
function textOf(html: string): string {
  return html.replace(/<[^>]+>/g, '')
}

const rows: SessionTriggerInfo[] = [
  trigger({ id: 'sub_live', label: 'Agent 1 findings' }),
  trigger({
    id: 'sub_ghost',
    label: 'Agent 1 ready',
    fired: true,
    retirementReason: 'once_consumed',
  }),
]

describe('SessionTriggers', () => {
  it('renders nothing when the session owns no subscriptions', () => {
    expect(
      renderToStaticMarkup(
        <SessionTriggers triggers={[]} onUnregister={() => {}} />,
      ),
    ).toBe('')
  })

  it('collapses to a count line with the rows folded away', () => {
    const html = renderToStaticMarkup(
      <SessionTriggers triggers={rows} onUnregister={() => {}} />,
    )
    expect(textOf(html)).toContain('1 trigger registered · 1 inactive')
    expect(html).toContain('aria-expanded="false"')
    // Folded rows stay mounted for the height transition but take no focus.
    expect(html).toContain('grid-rows-[0fr]')
    expect(html).toContain('inert=""')
    // No clear-all affordance without a handler for it.
    expect(html).not.toContain('clear all triggers')
  })

  it('unfolds one generic row per subscription, ghosts a step fainter', () => {
    const html = renderToStaticMarkup(
      <SessionTriggers
        triggers={rows}
        onUnregister={() => {}}
        onClearAll={() => {}}
        defaultExpanded
      />,
    )
    expect(html).toContain('aria-expanded="true"')
    expect(html).toContain('grid-rows-[1fr]')
    expect(html).not.toContain('inert=""')
    expect(html).toContain('Agent 1 findings')
    expect(textOf(html)).toContain(
      'Agent 1 findings · state · notifies this chat · on review/agent-1/findings · once',
    )
    expect(textOf(html)).toContain(
      'Agent 1 ready · state · notifies this chat · on review/agent-1/findings · once · consumed automatically',
    )
    // A live row unregisters; an inactive ghost only dismisses.
    expect(html).toContain('aria-label="unregister Agent 1 findings"')
    expect(html).toContain('aria-label="dismiss Agent 1 ready"')
    expect(html).toContain('aria-label="clear all triggers"')
  })
})

describe('clearAllDescription', () => {
  it('names what is torn down and what merely leaves the view', () => {
    expect(clearAllDescription(3, 6)).toBe(
      '3 triggers will be unregistered — nothing will notify this chat afterwards. 6 inactive rows will be dismissed.',
    )
    expect(clearAllDescription(1, 0)).toBe(
      '1 trigger will be unregistered — nothing will notify this chat afterwards.',
    )
    expect(clearAllDescription(0, 1)).toBe('1 inactive row will be dismissed.')
  })
})
