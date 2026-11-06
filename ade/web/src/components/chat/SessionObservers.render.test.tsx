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
    expect(textOf(html)).toContain(
      '1 observer active · 1 inactive · Agent 1 findings',
    )
    expect(html).toContain('aria-expanded="false"')
    // Folded rows stay mounted for the height transition but take no focus.
    expect(html).toContain('grid-rows-[0fr]')
    expect(html).toContain('inert=""')
    // No clear-all affordance without a handler for it.
    expect(html).not.toContain('clear all observers')
  })

  it('shows how long the oldest live observer has been waiting', () => {
    const fiveMinutesAgo = Date.now() - 5 * 60_000 - 1_000
    const html = renderToStaticMarkup(
      <SessionObservers
        observers={[
          observer({ id: 'a', label: 'Oldest', createdAt: fiveMinutesAgo }),
          observer({ id: 'b', label: 'Newer', createdAt: Date.now() }),
        ]}
        onDispose={() => {}}
        defaultExpanded
      />,
    )
    expect(textOf(html)).toContain('2 observers active · for 5m · Oldest')
    // The numbers come first: a narrow strip truncates only the name.
    const withInactive = renderToStaticMarkup(
      <SessionObservers
        observers={[
          observer({ id: 'a', label: 'Oldest', createdAt: fiveMinutesAgo }),
          observer({ id: 'g', label: 'Done', fired: true }),
        ]}
        onDispose={() => {}}
      />,
    )
    expect(textOf(withInactive)).toContain(
      '1 observer active · for 5m · 1 inactive · Oldest',
    )
    // Each live row says how long it has been waiting too, ahead of the facts
    // a long state key can truncate.
    expect(textOf(html)).toContain('Oldest · for 5m · state::on-changed')
    expect(textOf(html)).toContain('Newer · just now · state::on-changed')
  })

  it('names the one-shot observer, not an older recurring one', () => {
    const hourAgo = Date.now() - 60 * 60_000
    const html = renderToStaticMarkup(
      <SessionObservers
        observers={[
          observer({ id: 'c', label: 'Cron', once: false, createdAt: hourAgo }),
          observer({ id: 'w', label: 'Wake', createdAt: hourAgo }),
        ]}
        onDispose={() => {}}
      />,
    )
    expect(textOf(html)).toContain('2 observers active · for 1h · Wake')
  })

  it('names nothing when no observer is live', () => {
    const html = renderToStaticMarkup(
      <SessionObservers
        observers={[observer({ fired: true, label: 'Done' })]}
        onDispose={() => {}}
      />,
    )
    expect(textOf(html)).toContain('0 observers active · 1 inactive')
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

describe('SessionObservers inactive group', () => {
  it('folds the inactive rows under the live ones, with a Clear beside them', () => {
    const html = renderToStaticMarkup(
      <SessionObservers
        observers={rows}
        onDispose={() => {}}
        defaultExpanded
      />,
    )
    const live = html.indexOf('Agent 1 findings')
    const group = html.indexOf('1 inactive</span></button>')
    expect(live).toBeGreaterThan(0)
    expect(group).toBeGreaterThan(live)
    expect(html.indexOf('Agent 1 ready')).toBeGreaterThan(group)
    // Folded by default: the spent rows stay mounted but take no focus.
    expect(html.match(/aria-expanded="false"/g)).toHaveLength(1)
    expect(html).toContain('inert=""')
    expect(html).toContain('aria-label="clear 1 inactive observer"')
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
