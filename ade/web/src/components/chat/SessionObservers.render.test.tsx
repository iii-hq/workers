import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import { clearAllDescription, SessionObservers } from './SessionObservers'

function observer(
  overrides: Partial<SessionObserverInfo>,
): SessionObserverInfo {
  return {
    id: 'obs_1',
    functionId: 'state::on-changed',
    delivery: { kind: 'notify' },
    filters: {
      'input.scope': { $eq: 'review' },
      'input.key': { $eq: 'agent-1/findings' },
    },
    once: true,
    ...overrides,
  }
}

/** The rendered copy, tags stripped — the header noun spans a breakpoint. */
function textOf(html: string): string {
  return html.replace(/<[^>]+>/g, '')
}

const rows: SessionObserverInfo[] = [
  observer({ id: 'obs_live', label: 'Agent 1 findings' }),
  observer({
    id: 'obs_ghost',
    label: 'Agent 1 ready',
    fired: true,
    retirementReason: 'once_consumed',
  }),
]

describe('SessionObservers', () => {
  it('renders nothing when the session owns no observers', () => {
    expect(
      renderToStaticMarkup(
        <SessionObservers observers={[]} onDispose={() => {}} />,
      ),
    ).toBe('')
  })

  it('collapses to a count line with the rows folded away', () => {
    const html = renderToStaticMarkup(
      <SessionObservers observers={rows} onDispose={() => {}} />,
    )
    expect(textOf(html)).toContain('1 observer active · 1 inactive')
    expect(html).toContain('aria-expanded="false"')
    // Folded rows stay mounted for the height transition but take no focus.
    expect(html).toContain('grid-rows-[0fr]')
    expect(html).toContain('inert=""')
    // No clear-all affordance without a handler for it.
    expect(html).not.toContain('clear all observers')
  })

  it('unfolds one generic row per observer, ghosts a step fainter', () => {
    const html = renderToStaticMarkup(
      <SessionObservers
        observers={rows}
        onDispose={() => {}}
        onClearAll={() => {}}
        defaultExpanded
      />,
    )
    expect(html).toContain('aria-expanded="true"')
    expect(html).toContain('grid-rows-[1fr]')
    expect(html).not.toContain('inert=""')
    expect(html).toContain('Agent 1 findings')
    expect(textOf(html)).toContain(
      'Agent 1 findings · state::on-changed · notifies this chat · on review/agent-1/findings · once',
    )
    expect(textOf(html)).toContain(
      'Agent 1 ready · state::on-changed · notifies this chat · on review/agent-1/findings · once · consumed automatically',
    )
    // A live row is removed; an inactive ghost only dismisses.
    expect(html).toContain('aria-label="remove Agent 1 findings"')
    expect(html).toContain('aria-label="dismiss Agent 1 ready"')
    expect(html).toContain('aria-label="clear all observers"')
  })
})

describe('clearAllDescription', () => {
  it('names what is torn down and what merely leaves the view', () => {
    expect(clearAllDescription(3, 6)).toBe(
      '3 observers will be removed — nothing will notify this chat afterwards. 6 inactive rows will be dismissed.',
    )
    expect(clearAllDescription(1, 0)).toBe(
      '1 observer will be removed — nothing will notify this chat afterwards.',
    )
    expect(clearAllDescription(0, 1)).toBe('1 inactive row will be dismissed.')
  })
})
