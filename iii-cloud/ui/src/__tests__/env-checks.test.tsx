// The Checks tab (U9, Mng4): the Now line, the current check through CheckStage, the earlier checks by
// hand-off, and an earlier one opened read-only. CheckStage (U2) is stubbed: this suite checks what it is given.
import { describe, expect, it, vi } from 'vitest'
import type { CheckStageProps } from '../CheckStage'
import { boards } from '../fixtures'
import { agent, agentStatus, ready } from '../fixtures/base'
import { DEV } from '../fixtures/deploys'
import type { Status } from '../types'
import { board, btn, click, ctx, el, lastCall, mount, text, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))
vi.mock('../CheckStage', () => ({
  CheckStage: (p: CheckStageProps) => (
    <div
      data-check-stage={p.scenario}
      data-check-id={p.checkId ?? ''}
      data-commit={p.status.check?.commit ?? ''}
      data-agent={p.status.agent ? 'yes' : 'no'}
    />
  ),
}))

usePage()

const s = boards['manage-checks'].status
const stage = () => el.querySelector('[data-check-stage]')
const rows = () => [...el.querySelectorAll('[aria-label="Earlier checks of dev"] [role=row]')]
const checksTab = ctx({ at: 'env', env: DEV.id, tab: 'checks' })
/** An earlier check as `iii-cloud::check::get` returns it. */
const kept = (p: Record<string, unknown>) => ({
  ...ready,
  id: p.id,
  environment_id: DEV.id,
  commit: p.id === 'chk-5b' ? 'd738d2f'.padEnd(40, '0') : '9c04e7f'.padEnd(40, '0'),
  handoff_id: p.id === 'chk-5b' ? 'ho-2' : 'ho-3',
})

describe('Mng4: now', () => {
  it('a passed check dev does not serve waits for you: only the Now line, and Review and deploy opens its review', async () => {
    await board('manage-checks')
    const now = el.querySelector('section[aria-label=Now]')?.textContent ?? ''
    expect(now).toContain('Deploy 2be71d0 to dev')
    expect(now).toMatch(/Check passed on iii cloud and here at .+ · no agent work needed · waiting for you/)
    expect(stage()).toBeNull()
    expect(btn('Review and deploy')?.getAttribute('data-variant')).toBe('primary')
    await click(btn('Review and deploy'))
    expect(el.querySelector('aside[aria-label="Deploy steps"] [aria-current=step]')?.textContent).toContain(
      'Review and deploy',
    )
  })

  it('a context with no check id (an old run context) opens Checks, and Open the check opens the current one', async () => {
    const blocked = { ...s, check: { ...s.check!, ready: false } }
    await mount(blocked, ctx({ tab: 'run' }))
    expect(el.querySelector('[role=tab][aria-selected=true]')?.textContent).toBe('Checks')
    expect(stage()).toBeNull()
    expect(btn('Open the check')?.getAttribute('data-variant')).toBe('secondary')
    await click(btn('Open the check'))
    expect(stage()?.getAttribute('data-check-stage')).toBe('change')
    expect(stage()?.getAttribute('data-check-id')).toBe('')
    expect(stage()?.getAttribute('data-commit')).toBe(s.check?.commit)
  })

  it('the line follows the check: problems, the agent fixing them, already live, none yet', async () => {
    const blocked: Status = { ...agentStatus, project: s.project, checks: [] }
    await mount(blocked, checksTab)
    expect(text()).toContain('4 problems in 3c4d2e0')
    await mount({ ...blocked, agent: agent('working') }, ctx({ at: 'env', env: DEV.id, tab: 'checks' }, 2))
    expect(text()).toContain('Your agent is fixing 4 problems in 3c4d2e0')
    expect(text()).toContain('Bumping the sdk and regenerating the lock')
    const live = { ...s, check: { ...s.check!, commit: s.deploys[0].commit.sha } }
    await mount(live, ctx({ at: 'env', env: DEV.id, tab: 'checks' }, 3))
    expect(text()).toContain('9c04e7f is live on dev')
    await mount({ ...s, check: null }, ctx({ at: 'env', env: DEV.id, tab: 'checks' }, 4))
    expect(text()).toContain('No check of dev yet')
    expect(stage()).toBeNull()
    expect(btn('Open the check')).toBeFalsy()
    expect(btn('Review and deploy')).toBeFalsy()
  })

  it('a check dev already serves is opened, not reviewed', async () => {
    const live = { ...s, check: { ...s.check!, commit: s.deploys[0].commit.sha } }
    await mount(live, checksTab)
    expect(btn('Review and deploy')).toBeFalsy()
    await click(btn('Open the check'))
    expect(el.querySelector('aside[aria-label="Deploy steps"] [aria-current=step]')?.textContent).toContain(
      'Check before deploy',
    )
  })

  it('the note on what an item opens shows only when there are items', async () => {
    await board('manage-checks')
    expect(text()).toContain('Each item opens its check')
    await mount({ ...s, checks: [] }, ctx({ at: 'env', env: DEV.id, tab: 'checks' }, 2))
    expect(text()).not.toContain('Each item opens its check')
  })
})

describe('Mng4: earlier', () => {
  it('groups the kept checks by hand-off, newest first, without the current one, with their counts', async () => {
    await board('manage-checks')
    expect(rows().map((r) => r.textContent)).toEqual([
      expect.stringContaining(
        'Deploy 9c04e7f3 checks · your agent fixed 2 problems · you saved 1 secret · deployed as #6',
      ),
      expect.stringContaining('Deploy d738d2f2 checks · your agent fixed 1 problem · deployed as #5'),
      expect.stringContaining(
        'Deploy 58e516a4 checks · your agent fixed 5 problems · you saved 1 secret · deployed as #3',
      ),
    ])
    expect(text()).not.toContain('Deploy 2be71d0 · ')
  })

  it('a hand-off this page still knows is titled by its kind', async () => {
    const brief = { ...agentStatus.handoff_brief!, handoff_id: 'ho-2', kind: 'deploy_fix' as const }
    const fixing: Status = {
      ...s,
      handoff_brief: {
        ...brief,
        deploy: { id: 'dep-2', stage: 'workers', worker: 'http', code: null, hint: null, log_tail: [] },
      },
    }
    await mount(fixing, checksTab)
    expect(rows()[1].textContent).toContain('Fix deploy #2')
  })

  it('opening one reads it with check::get and shows it read-only, its agent only when the hand-off is still open', async () => {
    const h = await board('manage-checks', {}, { 'iii-cloud::check::get': kept })
    await click(rows()[1])
    expect(lastCall(h.calls)).toEqual({ id: 'iii-cloud::check::get', payload: { id: 'chk-5b' } })
    expect(stage()?.getAttribute('data-check-id')).toBe('chk-5b')
    expect(stage()?.getAttribute('data-commit')).toBe('d738d2f'.padEnd(40, '0'))
    expect(stage()?.getAttribute('data-agent')).toBe('no')
    expect([...el.querySelectorAll('[role=toolbar]')].at(-1)?.querySelector('h3')?.textContent).toBe('Check d738d2f')
    await click(btn('Back to Checks'))
    expect(stage()).toBeNull()
    await click(rows()[0])
    expect(stage()?.getAttribute('data-check-id')).toBe('chk-6c')
    expect(stage()?.getAttribute('data-agent')).toBe('yes')
  })

  it('the current check opens live, without reading it again', async () => {
    const h = await mount(s, ctx({ at: 'env', env: DEV.id, tab: 'checks', check: 'chk-now' }))
    expect(h.calls.some((c) => c.id === 'iii-cloud::check::get')).toBe(false)
    expect(stage()?.getAttribute('data-check-id')).toBe('')
    expect(stage()?.getAttribute('data-commit')).toBe(s.check?.commit)
  })

  it('a check no longer kept says so and leads back', async () => {
    await mount(s, ctx({ at: 'env', env: DEV.id, tab: 'checks', check: 'chk-gone' }), {
      'iii-cloud::check::get': () => {
        throw new Error('no such check (only the latest 20 are kept)')
      },
    })
    expect(text()).toContain('no such check (only the latest 20 are kept)')
    expect(stage()).toBeNull()
    await click(btn('All checks'))
    expect(rows()).toHaveLength(3)
  })
})
