// An environment's Overview (U6, Mng6): Serving, Health, Machines, Address, and the writes behind confirms.
import { describe, expect, it, vi } from 'vitest'
import { clock } from '../asked'
import { boards } from '../fixtures'
import { apiPath, problems, T, TEAM } from '../fixtures/base'
import { ANA, cloudDeploy, DEV, devDeploys, home, LIVE_SHA, NEW_SHA } from '../fixtures/manage'
import type { Status } from '../types'
import { board, btn, byText, click, cloudAnswers, ctx, el, flush, mount, text, type, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const settle = async () => {
  for (let i = 0; i < 5; i++) await flush()
}
const panel = (title: string) => el.querySelector(`section[aria-label="${title}"]`)
const overview = boards['manage-overview']
const writes = (h: Awaited<ReturnType<typeof mount>>) => h.calls.filter((c) => c.id === 'iii-cloud::api::write')
const open = async (status: Status = overview.status, cloud: Record<string, unknown> = {}, answers = {}) => {
  const h = await mount(status, ctx(overview.context), {
    ...cloudAnswers({ ...overview.cloud, ...cloud }),
    ...answers,
  })
  await settle()
  return h
}

describe('Overview (Mng6)', () => {
  it('Serving, Health, Machines and Address from iii cloud; the old journey is gone', async () => {
    await board('manage-overview')
    await settle()
    const serving = panel('Serving')?.textContent ?? ''
    expect(serving).toContain(`Commit${LIVE_SHA.slice(0, 7)} Search: rank by stars`)
    expect(serving).toContain(`Deploy#6 · live since ${clock(T(5))} · by you from this machine`)
    expect(serving).toContain('App fileapi/worker-compose-prod.yaml · 11 workers in 3 groups · Change…')
    expect(serving).toContain('Checkedon iii cloud and here, before the deploy')
    expect(panel('Health')?.textContent).toBe(`Healthiii cloud: 11 of 11 workers upchecked ${clock(T(1))}`)
    const machines = [...(panel('Machines')?.querySelectorAll('tbody tr') ?? [])].map((r) => r.textContent)
    expect(machines).toEqual(['app1 × 1 GiBrunning', 'http1 × 500 MiBrunning', 'jobs1 × 1 GiBresizing to 2 GiB'])
    const address = panel('Address')?.textContent ?? ''
    expect(address).toContain('Defaultregistry-dev.iii.cloud')
    expect(address).toContain('Your domainsnone')
    expect(text()).not.toContain('Make it ready')
  })

  it('the waiting panel: one commit passed the check and dev still serves the older one', async () => {
    await board('manage-overview')
    await settle()
    const wait = byText('[role=status] b', 'Waiting for you: 1 commit passed the check')?.parentElement
    expect(wait?.textContent).toContain(
      `${NEW_SHA.slice(0, 7)} "Search: tweak weights" · checked on iii cloud and here at ${clock(T(1))} · dev still serves ${LIVE_SHA.slice(0, 7)}.`,
    )
  })

  it('"by you" only when you started it: a teammate is named, and a deploy made elsewhere is not "checked here"', async () => {
    const hers = cloudDeploy(7, 'aa11bb2', 'Hotfix', 'ready', 3, ANA, { current: true })
    const h = await open(overview.status, {
      [apiPath('/deployments', DEV.id)]: { deployments: [hers, ...devDeploys.map((d) => ({ ...d, current: false }))] },
    })
    const serving = panel('Serving')?.textContent ?? ''
    expect(serving).toContain(`Deploy#7 · live since ${clock(T(2))} · by ana`)
    expect(serving).not.toContain('by you')
    expect(serving).not.toContain('from this machine')
    expect(serving).toContain('Checkednot checked here first')
    expect(h.calls.some((c) => c.payload?.path === `/teams/${TEAM.id}/members`)).toBe(true)
  })

  it('nothing live yet: no health is read, and Serving says so', async () => {
    const h = await open(overview.status, { [apiPath('/deployments', DEV.id)]: { deployments: [] } })
    expect(panel('Serving')?.textContent).toContain('Nothing is live on dev yet.')
    expect(panel('Health')?.textContent).toContain('Nothing runs on dev yet.')
    expect(h.calls.some((c) => String(c.payload?.path).endsWith('/health'))).toBe(false)
  })

  it('Restart a group: nothing is written until the confirm; then POST …/groups/app/restart as the person', async () => {
    const path = apiPath('/machines/groups/app/restart', DEV.id)
    const h = await open(overview.status, { [path]: null })
    await click(byText('[role=menuitem]', 'app'))
    const dialog = el.querySelector('[role=alertdialog]')
    expect(dialog?.textContent).toContain('Restart app on dev?')
    expect(writes(h)).toHaveLength(0)
    await click(byText('[role=alertdialog] button', 'Restart app'))
    await settle()
    expect(writes(h)).toHaveLength(1)
    expect(writes(h)[0].payload).toMatchObject({ method: 'POST', path })
    expect(typeof writes(h)[0].payload?.idempotency_key).toBe('string')
    expect(el.querySelector('[role=alertdialog]')).toBeNull()
    expect(text()).toContain('app is restarting')
  })

  it('Stop dev: a danger confirm, then POST …/machines/stop; the cloud’s refusal is shown as it says', async () => {
    const path = apiPath('/machines/stop', DEV.id)
    const refuse = () => ({ status: 403, body: { code: 'FORBIDDEN', message: 'Only an Owner or Admin can stop it.' } })
    const h = await open(overview.status, {}, { 'iii-cloud::api::write': refuse })
    await click(btn('Stop dev'))
    expect(el.querySelector('[role=alertdialog]')?.textContent).toContain('Stop dev?')
    await click(byText('[role=alertdialog] button', 'Stop dev'))
    await settle()
    expect(writes(h).map((c) => c.payload?.path)).toEqual([path])
    expect(text()).toContain('Only an Owner or Admin can stop it.')
  })

  it('Add a domain: POST /domains with the hostname, then the TXT record to add', async () => {
    const added = {
      id: 'dom-1',
      hostname: 'api.example.com',
      status: 'pending_dns',
      verification: { type: 'TXT', name: '_iii-cloud.api.example.com', value: 'iii-verify=7f3a9c1e' },
      created_at: T(0),
    }
    const h = await open(
      overview.status,
      {},
      {
        'iii-cloud::api::write': () => ({ status: 201, body: added }),
      },
    )
    await click(btn('Add a domain'))
    expect(el.querySelector('[role=dialog]')?.textContent).toContain('Add a domain to dev')
    expect((btn('Add domain') as HTMLButtonElement).disabled).toBe(true)
    await type(el.querySelector('#iii-cloud-domain'), ' api.example.com ')
    await click(btn('Add domain'))
    await settle()
    expect(writes(h).map((c) => c.payload)).toMatchObject([
      { method: 'POST', path: apiPath('/domains', DEV.id), body: { hostname: 'api.example.com' } },
    ])
    const dialog = el.querySelector('[role=dialog]')?.textContent ?? ''
    expect(dialog).toContain('Name_iii-cloud.api.example.com')
    expect(dialog).toContain('Valueiii-verify=7f3a9c1e')
  })

  it('a domain still waiting for its TXT record shows the record in the list', async () => {
    const pending = {
      id: 'dom-1',
      hostname: 'api.example.com',
      status: 'pending_dns',
      verification: { type: 'TXT', name: '_iii-cloud.api.example.com', value: 'iii-verify=7f3a9c1e' },
      created_at: T(0),
    }
    await open(overview.status, { [apiPath('/domains', DEV.id)]: { domains: [pending] } })
    expect(panel('Address')?.textContent).toContain(
      'api.example.com · waiting for its TXT record · TXT _iii-cloud.api.example.com iii-verify=7f3a9c1e',
    )
  })

  it('locked while iii cloud wants a fresh sign-in: no restart, stop, domain or deploy', async () => {
    const problem = { ...problems[0], code: 'SIGN_IN_REQUIRED' }
    await open({ ...home, check: { ...home.check!, problems: [problem] } })
    for (const label of [
      'Restart a group…',
      'Stop dev',
      'Add a domain',
      'Review and deploy',
      'Deploy from this machine',
    ]) {
      expect((btn(label) as HTMLButtonElement).disabled, label).toBe(true)
    }
  })
})
