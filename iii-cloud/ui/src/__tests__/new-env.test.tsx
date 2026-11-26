// New environment (U10, Mng8): the form, what iii cloud will create, the role hint, then the deploy steps.
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { CloudPage } from '../CloudPage'
import { boards } from '../fixtures'
import { apiPath, ENV, TEAM } from '../fixtures/base'
import { DEV } from '../fixtures/new-env'
import type { Status } from '../types'
import { type Call, fakeHost } from './host'
import { board, btn, click, cloudAnswers, ctx, el, flush, mount, root, text, type, ui, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const b = boards['manage-new-env']
const radios = () => [...el.querySelectorAll('[role=radiogroup] [role=radio]')]
const checked = () => el.querySelector('[role=radio][aria-checked=true]')?.textContent
const nameInput = () => el.querySelector('#iii-cloud-ne-name')
const branchInput = () => el.querySelector('#iii-cloud-ne-branch') as HTMLInputElement
const plan = () => el.querySelector('[aria-label="What iii cloud will create"]')?.textContent ?? ''
const created = (calls: Call[]) => calls.find((c) => c.id === 'iii-cloud::project::create_environment')?.payload
/** The page's reads of `env`'s machines and resources. */
const reads = (calls: Call[], env: string) =>
  calls.filter(
    (c) =>
      c.id === 'iii-cloud::api' &&
      [apiPath('/machines', env), apiPath('/resources', env)].includes(String(c.payload?.path)),
  ).length

describe('Mng8 new environment', () => {
  it('opens on the checkout branch; dev is the default copy and its machines and resources are listed', async () => {
    const h = await board('manage-new-env')
    expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects / registry / New environment')
    expect(branchInput().value).toBe('feat/search-rank')
    expect(text()).toContain('This machine deploys it after the check. A push from anywhere else deploys right away.')
    expect(radios().map((r) => r.textContent)).toEqual([
      'devrecommended for a trial',
      'productioncopies production secrets',
      'Start emptyyou set everything',
    ])
    expect(checked()).toBe('devrecommended for a trial')
    expect(plan()).toContain('as in dev')
    expect(plan()).toContain('main: a new Postgres database, shared')
    expect(plan()).toContain('app 1 × 500 MiB · http 1 × 500 MiB · jobs 1 × 500 MiB')
    expect(plan()).not.toContain('Cost')
    expect(reads(h.calls, ENV.id)).toBe(0)
    expect(text()).not.toContain('Owners and admins create environments')
    expect(btn('Create environment')?.hasAttribute('disabled')).toBe(true)
    await type(branchInput(), 'main')
    expect(text()).toContain('It deploys when a pull request into main is merged.')
  })

  it('creates it as a copy of dev, then opens the deploy steps of the new environment', async () => {
    let status: Status = b.status
    const made = { id: 'e-trial', name: 'trial-search', production: false, branch: 'feat/search-rank' }
    const h = fakeHost(() => status, {
      ...cloudAnswers(b.cloud ?? {}),
      'iii-cloud::project::create_environment': () => {
        const p = status.project!
        status = {
          ...status,
          project: {
            ...p,
            environment: { ...made, manifest: p.environment?.manifest ?? null, url: null },
            environments: [...p.environments, made],
          },
        }
        return { environment_id: 'e-trial', project_id: 'p1' }
      },
    })
    await act(async () => root.render(<CloudPage host={h.host} panelSide="right" tabId="t" {...ctx(b.context)} />))
    await flush()
    await type(nameInput(), 'trial-search')
    await click(btn('Create trial-search'))
    await flush()
    expect(created(h.calls)).toEqual({
      cloud_project_id: 'p1',
      name: 'trial-search',
      branch: 'feat/search-rank',
      manifest: 'api/worker-compose-prod.yaml',
      copy_from: DEV.id,
    })
    expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects / registry / trial-search / Deploy')
    expect(el.querySelector('aside[aria-label="Deploy steps"]')).not.toBeNull()
    // create_environment selects it already: no second project::use.
    expect(h.calls.some((c) => c.id === 'iii-cloud::project::use')).toBe(false)
  })

  it('production reads its own machines; Start empty copies nothing and sends start_empty without copy_from', async () => {
    const h = await board('manage-new-env', {}, { 'iii-cloud::project::create_environment': {} })
    await click(radios()[1])
    await flush()
    expect(checked()).toBe('productioncopies production secrets')
    expect(plan()).toContain('as in production')
    expect(plan()).toContain('app 1 × 2 GiB · jobs 1 × 4 GiB')
    expect(plan()).not.toContain('old')
    await click(radios()[2])
    expect(plan()).toContain('Nothing is copied')
    expect(plan()).not.toContain('as in')
    await type(nameInput(), 'scratch')
    await click(btn('Create scratch'))
    expect(created(h.calls)).toEqual({
      cloud_project_id: 'p1',
      name: 'scratch',
      branch: 'feat/search-rank',
      manifest: 'api/worker-compose-prod.yaml',
      start_empty: true,
    })
  })

  it('a developer sees the role hint, can still create, and reads the cloud 403 in full', async () => {
    const s: Status = { ...b.status, connection: { ...b.status.connection, teams: [{ ...TEAM, role: 'developer' }] } }
    const err = Object.assign(
      new Error("Your role in this team can't do that. (Only owners and admins can add environments.)"),
      { code: 'TEAM_FORBIDDEN' },
    )
    const h = await mount(s, ctx(b.context), {
      ...cloudAnswers(b.cloud ?? {}),
      'iii-cloud::project::create_environment': err,
    })
    expect(text()).toContain('Owners and admins create environments')
    expect(text()).toContain('Your role on team iii is developer.')
    await type(nameInput(), 'qa')
    expect(btn('Create qa')?.hasAttribute('disabled')).toBe(false)
    await click(btn('Create qa'))
    expect(created(h.calls)).toBeTruthy()
    expect(text()).toContain('Only an owner or admin of the team can create this.')
    expect(text()).toContain('iii cloud says: Only owners and admins can add environments.')
    expect(text()).not.toContain('TEAM_FORBIDDEN')
    expect(btn('Create qa')?.hasAttribute('disabled')).toBe(false)
  })

  it('a name iii cloud would refuse says why and keeps Create off', async () => {
    await board('manage-new-env')
    await type(nameInput(), 'Trial Search')
    expect(text()).toContain('Lowercase letters, digits and hyphens, at most 30.')
    expect(btn('Create Trial Search')?.hasAttribute('disabled')).toBe(true)
    await type(nameInput(), 'trial-search')
    expect(text()).not.toContain('Lowercase letters, digits and hyphens')
  })

  it("dev's machines can't be read: a warning with Try again, which reads again", async () => {
    const { [apiPath('/machines', DEV.id)]: _gone, ...rest } = b.cloud ?? {}
    const h = await mount(b.status, ctx(b.context), cloudAnswers(rest))
    expect(plan()).toContain("Can't read what dev runs")
    expect(plan()).toContain("iii cloud can't find that")
    const before = reads(h.calls, DEV.id)
    await click(btn('Try again'))
    await flush()
    expect(reads(h.calls, DEV.id)).toBeGreaterThan(before)
  })

  it('on a phone the page has a way back to the project', async () => {
    ui.narrow = true
    await board('manage-new-env')
    expect(el.querySelector('aside')).toBeNull()
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('New environment')
    await click(btn('Back to registry'))
    expect(el.querySelector('[aria-label="New environment in iii cloud"]')).toBeNull()
    expect(el.querySelector('[role=toolbar] h2')?.textContent).toBe('registry')
  })

  it('Cancel goes back to the project with no call', async () => {
    const h = await board('manage-new-env')
    const before = h.calls.filter((c) => c.id !== 'iii-cloud::status' && c.id !== 'iii-cloud::api').length
    await click(btn('Cancel'))
    expect(el.querySelector('[role=toolbar] nav')?.textContent).toBe('Projects / registry')
    expect(h.calls.filter((c) => c.id !== 'iii-cloud::status' && c.id !== 'iii-cloud::api')).toHaveLength(before)
  })
})
