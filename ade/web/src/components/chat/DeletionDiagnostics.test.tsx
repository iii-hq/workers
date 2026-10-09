// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  canForceDelete,
  type DeletionBlocker,
  type SessionTreeDeletionSnapshot,
} from '@/lib/sessions/delete-tree'
import { DeletionDiagnostics } from './DeletionDiagnostics'
import {
  groupDeletionBlockers,
  pageBlockerGroups,
} from './deletion-diagnostics'

let host: HTMLDivElement
let root: Root
const clipboard = vi.fn().mockResolvedValue(undefined)
const snapshot = (
  blockers: DeletionBlocker[] = [],
): SessionTreeDeletionSnapshot => ({
  operation_id: 'op',
  attempt: 1,
  session_id: 'root',
  status: 'failed',
  mode: 'normal',
  failure_code: 'blocked',
  force_eligible: true,
  data_retained: true,
  deleted_session_ids: [],
  remaining_session_ids: ['root'],
  unconfirmed_session_ids: [],
  blockers,
})
function many(count: number, distinct = false): DeletionBlocker[] {
  return Array.from({ length: count }, (_, index) => ({
    kind: (
      [
        'active_processing',
        'unconfirmed_cancellation',
        'unknown_completion',
      ] as const
    )[index % 3],
    session_id: distinct ? `chat-${index}` : `chat-${Math.floor(index / 12)}`,
    function_id: index % 2 ? 'browser::fetch' : 'shell::exec',
    call_id: `call-${index}`,
    started_at: index,
  }))
}
const render = async (value: SessionTreeDeletionSnapshot) =>
  act(async () => root.render(<DeletionDiagnostics snapshot={value} />))
function button(name: string, region: ParentNode = host) {
  const result = [...region.querySelectorAll('button')].find(
    (node) => (node.getAttribute('aria-label') ?? node.textContent) === name,
  )
  if (!result) throw new Error(`Missing button ${name}`)
  return result
}
const click = async (name: string, region: ParentNode = host) =>
  act(async () => button(name, region).click())
const rows = () =>
  host.querySelectorAll<HTMLDetailsElement>('[data-deletion-blocker]')
const groups = () => host.querySelectorAll('[data-deletion-session-group]')
async function expand(selector: string) {
  await act(async () => {
    const details = host.querySelector<HTMLDetailsElement>(selector)
    if (!details) throw new Error(`Missing details ${selector}`)
    details.open = true
    details.dispatchEvent(new Event('toggle'))
  })
}
async function filter(value: string) {
  await act(async () => {
    const input = host.querySelector('input')
    if (!input) throw new Error('Missing search field')
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    if (!setter) throw new Error('Missing input value setter')
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
function counts() {
  return [...host.querySelectorAll('[data-deletion-outcome-counts] > div')].map(
    (node) =>
      `${node.querySelector('dt')?.textContent}: ${node.querySelector('dd')?.textContent}`,
  )
}
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  clipboard.mockClear()
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText: clipboard },
  })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.unstubAllGlobals()
})

describe('DeletionDiagnostics — bounded read-only presentation', () => {
  it.each([1, 24, 1000])(
    'renders %s blockers without unbounded rows or automatic expansion',
    async (count) => {
      await render(snapshot(many(count)))
      expect(rows()).toHaveLength(Math.min(count, 24))
      expect(groups()).toHaveLength(Math.min(Math.ceil(count / 12), 2))
      expect(host.textContent).toContain(
        `${Math.ceil(count / 12)} affected chat`,
      )
      expect(host.textContent).toContain(
        `Showing 1–${Math.min(count, 24)} of ${count} blockers`,
      )
      expect(host.querySelectorAll('details[open]')).toHaveLength(0)
      expect(host.querySelector('[data-deletion-blocker-detail]')).toBeNull()
      expect(host.querySelector('[data-deletion-outcome-list]')).toBeNull()
      expect(host.querySelector('summary button, button button')).toBeNull()
    },
  )
  it('caps 1000 distinct groups and reaches the final row by paging or searching', async () => {
    await render(snapshot(many(1000, true)))
    expect(groups()).toHaveLength(24)
    expect(host.textContent).toContain('1000 affected chats')
    for (let page = 1; page <= 41; page++) await click('Next')
    expect(rows()).toHaveLength(16)
    expect(host.textContent).toContain('Showing 985–1000 of 1000 blockers')
    expect(button('Next').getAttribute('aria-disabled')).toBe('true')
    await filter('chat-999 call-999')
    expect(rows()).toHaveLength(1)
    expect(host.textContent).toContain(
      '1 of 1000 blockers match · 1 identified chats match',
    )
    expect(host.textContent).toContain('1000 affected chats')
    await click('Clear')
    expect(rows()).toHaveLength(24)
    expect(host.textContent).toContain('Showing 1–24 of 1000 blockers')
  })
  it('does not lose rows when one chat spans pages, including duplicate/missing call IDs', async () => {
    const blockers = many(1000).map((blocker) => ({
      ...blocker,
      session_id: 'same',
      call_id: undefined,
    }))
    const value = snapshot(blockers)
    await render(value)
    expect(groups()).toHaveLength(1)
    expect(host.textContent).toContain('24 of 1000 blockers shown here')
    await click('Next')
    expect(rows()).toHaveLength(24)
    expect(rows()[0].textContent).toContain('blocker 25')
    expect(groupDeletionBlockers(blockers, '')[0].entries).toHaveLength(1000)
    expect(
      pageBlockerGroups(groupDeletionBlockers(blockers, ''), 41)[0].entries,
    ).toHaveLength(16)
    await render(snapshot([blockers[0]]))
    expect(rows()).toHaveLength(1)
    expect(host.textContent).toContain('Showing 1–1 of 1 blockers')
  })
  it('filters full IDs and typed labels across all pages without changing safety eligibility', async () => {
    const value = snapshot(many(1000))
    await render(value)
    await filter('UNKNOWN_COMPLETION browser::fetch')
    expect(host.textContent).toContain('166 of 1000 blockers match')
    expect(
      [...rows()].every((row) =>
        row.textContent?.includes('Unknown completion after timeout'),
      ),
    ).toBe(true)
    await filter('no such operation')
    expect(rows()).toHaveLength(0)
    expect(host.textContent).toContain('No matching blockers')
    expect(host.textContent).toContain('0 of 1000 blockers match')
    expect(canForceDelete(value)).toBe(true)
    expect(host.querySelector('[data-force-delete]')).toBeNull()
    const input = host.querySelector('input')
    if (!input) throw new Error('Missing search field')
    input.focus()
    await act(async () =>
      input.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
      ),
    )
    expect(input.value).toBe('')
    expect(document.activeElement).toBe(input)
    expect(rows()).toHaveLength(24)
  })
  it('reveals selectable full machine fields, raw observed timestamp and accessible copy feedback', async () => {
    const session = `session-${'s'.repeat(500)}`
    const fn = `worker::${'f'.repeat(500)}`
    const call = `call-${'c'.repeat(500)}`
    await render(
      snapshot([
        {
          kind: 'unknown_completion',
          session_id: session,
          function_id: fn,
          call_id: call,
          started_at: 0,
        },
      ]),
    )
    expect(host.querySelector('h4')?.textContent).toContain(session)
    await expand('[data-deletion-blocker]')
    const detail = host.querySelector('[data-deletion-blocker-detail]')
    if (!detail) throw new Error('Missing blocker details')
    expect(
      [...detail.querySelectorAll('dd')].map((node) => node.textContent),
    ).toEqual([fn, session, call, '1970-01-01T00:00:00.000Z'])
    const copy = button('Copy details')
    copy.focus()
    await click('Copy details')
    expect(clipboard).toHaveBeenCalledWith(
      `Blocker: Unknown completion after timeout\nFunction: ${fn}\nSession: ${session}\nCall: ${call}\nObserved: 1970-01-01T00:00:00.000Z`,
    )
    expect(detail.querySelector('[role="status"]')?.textContent).toBe('Copied')
    expect(document.activeElement).toBe(copy)
  })
  it('labels missing fields and invalid timestamps without inventing completion evidence', async () => {
    await render(
      snapshot([
        { kind: 'active_processing', session_id: '' },
        {
          kind: 'unconfirmed_cancellation',
          session_id: 'known',
          started_at: 9e15,
        },
      ]),
    )
    expect(host.textContent).toContain('1 identified affected chat')
    expect(host.textContent).toContain('1 blocker has no session ID')
    expect(host.textContent).toContain('Active processing')
    expect(host.textContent).toContain('Unconfirmed cancellation')
    expect(host.textContent).not.toContain('after timeout')
    await expand('[data-deletion-blocker]')
    expect(
      [...host.querySelectorAll('dd')].filter(
        (node) => node.textContent === 'Not reported',
      ),
    ).toHaveLength(4)
    await expand(
      '[data-deletion-session-group]:last-child [data-deletion-blocker]',
    )
    expect(host.textContent).toContain('Invalid timestamp (9000000000000000)')
  })
  it('keeps zero blockers independent of unknown outcomes and force eligibility', async () => {
    const value = {
      ...snapshot(),
      remaining_session_ids: [],
      unconfirmed_session_ids: [],
    }
    await render(value)
    expect(counts()).toEqual([
      'Confirmed deleted: 0',
      'Not deleted: unknown',
      'Unconfirmed: unknown',
    ])
    expect(host.textContent).toContain('No blockers reported')
    expect(host.textContent).toContain('This does not confirm deletion')
    expect(canForceDelete(value)).toBe(false)
    expect(host.querySelector('input')).toBeNull()
  })
  it('shows known mixed outcomes only on expansion and pages all reported IDs', async () => {
    const deleted = Array.from(
      { length: 1000 },
      (_, index) => `deleted-${index}`,
    )
    const value = {
      ...snapshot(),
      mode: 'force' as const,
      deleted_session_ids: deleted,
      remaining_session_ids: ['retained', 'uncertain'],
      unconfirmed_session_ids: ['uncertain'],
    }
    await render(value)
    expect(counts()).toEqual([
      'Confirmed deleted: 1000',
      'Not deleted: 1',
      'Unconfirmed: 1',
    ])
    expect(host.textContent).not.toContain('deleted-999')
    await expand('[data-deletion-outcomes]')
    const region = host.querySelector('[data-deletion-outcomes]')
    if (!region) throw new Error('Missing outcome details')
    expect(region.querySelectorAll('li')).toHaveLength(24)
    for (let page = 1; page <= 41; page++) await click('Next', region)
    expect(region.querySelectorAll('li')).toHaveLength(18)
    expect(region.textContent).toContain('Deleted sessions: deleted-999')
    expect(region.textContent).toContain('Not deleted sessions: retained')
    expect(region.textContent).toContain(
      'Deletion outcome unconfirmed: uncertain',
    )
    expect(canForceDelete(value)).toBe(false)
  })
  it.each(['blockers', 'outcomes'] as const)(
    'preserves focused %s boundary controls and announces each result change once',
    async (kind) => {
      await render({
        ...snapshot(kind === 'blockers' ? many(25) : []),
        remaining_session_ids: Array.from(
          { length: 25 },
          (_, i) => `chat-${i}`,
        ),
      })
      if (kind === 'outcomes') {
        await expand('[data-deletion-outcomes]')
        const list = host.querySelector<HTMLUListElement>(
          '[data-deletion-outcome-list]',
        )
        if (!list) throw new Error('Missing outcome list')
        expect(list.tabIndex).toBe(0)
        expect(list.getAttribute('aria-label')).toBe(
          'Reported deletion outcomes',
        )
        expect(list.getAttribute('role')).toBeNull()
        expect(list.children).toHaveLength(24)
        list.focus()
        expect(document.activeElement).toBe(list)
      }
      const nav = host.querySelector(
        `[aria-label="${kind === 'blockers' ? 'Blocker' : 'Outcome'} pages"]`,
      )
      const resultRegion = host.querySelector(
        kind === 'blockers'
          ? '[data-deletion-blockers]'
          : '[data-deletion-outcomes]',
      )
      if (!nav || !resultRegion) throw new Error('Missing pagination region')
      expect(resultRegion.querySelectorAll('[aria-live]')).toHaveLength(1)
      expect(nav.querySelector('[aria-live]')).toBeNull()
      const previous = button('Previous', nav)
      const next = button('Next', nav)
      previous.focus()
      expect(previous.disabled).toBe(false)
      expect(previous.getAttribute('aria-disabled')).toBe('true')
      await click('Previous', nav)
      expect(document.activeElement).toBe(previous)
      expect(nav.textContent).toContain('Page 1 of 2')
      next.focus()
      await click('Next', nav)
      expect(document.activeElement).toBe(next)
      expect(next.disabled).toBe(false)
      expect(next.getAttribute('aria-disabled')).toBe('true')
      await click('Next', nav)
      expect(document.activeElement).toBe(next)
      expect(nav.textContent).toContain('Page 2 of 2')
      expect(resultRegion.textContent).toContain('Showing 25–25 of 25')
      previous.focus()
      await click('Previous', nav)
      await click('Previous', nav)
      expect(document.activeElement).toBe(previous)
      expect(nav.textContent).toContain('Page 1 of 2')
    },
  )
  it('keeps legacy outcomes unconfirmed and never derives counts from blocker groups', async () => {
    await render({
      ...snapshot(many(24)),
      remaining_session_ids: ['legacy'],
      unconfirmed_session_ids: undefined,
    })
    expect(counts()).toEqual([
      'Confirmed deleted: 0',
      'Not deleted: 0',
      'Unconfirmed: 1',
    ])
    expect(host.textContent).toContain('2 affected chats')
    await expand('[data-deletion-outcomes]')
    expect(host.textContent).toContain('Deletion outcome unconfirmed: legacy')
    await render({
      ...snapshot(many(24)),
      remaining_session_ids: undefined,
      unconfirmed_session_ids: undefined,
    })
    expect(counts()).toEqual([
      'Confirmed deleted: 0',
      'Not deleted: unknown',
      'Unconfirmed: unknown',
    ])
  })
})
