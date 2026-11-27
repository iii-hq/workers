// Deployments on the environment page (Env1) and a deployment's record (Env3, Phone-Failed): every deployment reads
// the same here as everywhere (deployOrigin, deployState), a failed one opens its record, whose action hands off.
import { describe, expect, it, vi } from 'vitest'
import { agent, apiPath, T } from '../fixtures/base'
import { DEV, devDeploys, devList } from '../fixtures/deploys'
import { envPageStatus } from '../fixtures/overview'
import { deployOrigin, deployState, servingId, when } from '../model'
import type { CloudDeployment, Status } from '../types'
import { board, btn, byText, click, cloudAnswers, ctx, el, flush, lastCall, mount, text, ui, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

const settle = async () => {
  for (let i = 0; i < 5; i++) await flush()
}
const path = () => el.querySelector('[role=toolbar] nav, [role=toolbar] h2')?.textContent
const rows = () => [...el.querySelectorAll('li.iii-cloud-dep')]
const rowOf = (n: number) => rows().find((r) => r.textContent?.startsWith(`#${n}`))
const cloud = cloudAnswers({
  [apiPath('/deployments', DEV.id)]: devList,
  [apiPath('/deployments')]: { deployments: [] },
})

describe('Deployments', () => {
  it('one row per deployment: its word, its title and its line are the ones every other place uses', async () => {
    await board('env-page')
    await settle()
    expect(rows().map((r) => r.querySelector('.iii-cloud-mono')?.textContent)).toEqual([
      '#6',
      '#5',
      '#4',
      '#3',
      '#2',
      '#1',
    ])
    const live = servingId(envPageStatus, DEV.id, devList.deployments as CloudDeployment[])
    for (const d of devDeploys) {
      const row = rowOf(d.number)
      // The word is deployState's (never Ready or Superseded), the line is deployOrigin's.
      expect(row?.querySelector('[data-chip]')?.textContent).toBe(deployState(d, live).label)
      const o = deployOrigin(envPageStatus, d, devList.deployments.find((x) => x.id === d.id) as never)
      expect(row?.textContent).toContain(`${when(d.created_at)} · ${o.line}`)
    }
    expect(rowOf(6)?.textContent).toContain('Live')
    expect(rowOf(6)?.textContent).toContain('Search: rank by stars')
    expect(rowOf(5)?.textContent).toContain('Replaced')
    expect(rowOf(2)?.textContent).toContain('Failed')
    expect(text()).not.toMatch(/Ready|Superseded|Cancelled/)
  })

  it('a failed one says its cause in a sentence and has Open; the others have no Open', async () => {
    await board('env-page')
    await settle()
    expect(rowOf(2)?.textContent).toContain(
      'Your app starts its own web server; iii cloud only sends traffic to its http package',
    )
    expect(rowOf(2)?.querySelector('button')?.textContent).toBe('Open')
    expect(rowOf(6)?.querySelector('button')).toBeNull()
  })

  it('says "your agent fixed 2" only where the worker records the check came from a hand-off', async () => {
    const status: Status = {
      ...envPageStatus,
      checks: [
        {
          id: 'chk-6c',
          environment_id: DEV.id,
          commit: 'a',
          checked_at: T(7),
          duration_ms: 1,
          ready: true,
          problems: 0,
          handoff_id: 'ho-1',
          manifest: null,
        },
        {
          id: 'chk-5b',
          environment_id: DEV.id,
          commit: 'b',
          checked_at: T(50),
          duration_ms: 1,
          ready: true,
          problems: 0,
          handoff_id: null,
          manifest: null,
        },
      ],
      handoff_brief: {
        handoff_id: 'ho-1',
        kind: 'ready',
        agent_problems: [1, 2].map((n) => ({ title: `p${n}`, file: null, line: null, detail: null, fix: null })),
        person_problems: [],
      } as never,
    }
    await mount(status, ctx({ at: 'env', env: DEV.id }), cloud)
    await settle()
    expect(rowOf(6)?.textContent).toContain('you, from this machine · checked, your agent fixed 2')
    expect(rowOf(5)?.textContent).toContain('you, from this machine · checked')
    expect(rowOf(5)?.textContent).not.toContain('your agent')
    // A check of a hand-off that is no longer on file: said without a count.
    await mount({ ...status, handoff_brief: null }, ctx({ at: 'env', env: DEV.id }, 2), cloud)
    await settle()
    expect(rowOf(6)?.textContent).toContain("checked, with your agent's fixes")
  })

  it('a canceled one names the deployment that replaced it', async () => {
    const canceled = {
      ...devDeploys[2],
      status: 'canceled' as const,
      outcome: 'canceled' as const,
      replaced_by: 'dep-5',
    }
    const status = { ...envPageStatus, deploys: [devDeploys[0], devDeploys[1], canceled] }
    await mount(status, ctx({ at: 'env', env: DEV.id }), cloud)
    await settle()
    expect(rowOf(4)?.textContent).toContain('Canceled')
    expect(rowOf(4)?.textContent).toContain('#5 replaced it before approval')
  })

  it('its header goes to iii cloud for the full list and the logs; with none yet it says so', async () => {
    await board('env-page')
    await settle()
    expect(byText('a', 'All deployments and logs in iii cloud')?.getAttribute('href')).toBe(
      'https://cloud.iii.dev/iii/registry/dev/deployments',
    )
    await mount({ ...envPageStatus, deploys: [] }, ctx({ at: 'env', env: DEV.id }, 2), cloudAnswers({}))
    await settle()
    expect(text()).toContain('No deployments of dev yet')
  })

  it('a failed row opens its record, and Back to the environment is the path', async () => {
    await board('env-page')
    await settle()
    await click(rowOf(2)?.querySelector('button'))
    expect(path()).toBe('Projects / registry · dev / Deployment #2')
    await click(byText('button', 'registry · dev'))
    expect(path()).toBe('Projects / registry · dev')
  })
})

describe("a failed deployment's record (Env3)", () => {
  it('the cause first, what still serves, the three cells, where it stopped and the full log', async () => {
    await board('manage-deploys-failed')
    await settle()
    const t = text()
    const title = 'Your app starts its own web server; iii cloud only sends traffic to its http package'
    expect(el.querySelector('h2.iii-cloud-h')?.textContent).toBe(title)
    expect(t).toContain('dev still serves #6. Nothing changed for your users.')
    const cells = [...el.querySelectorAll('.iii-cloud-cell')].map((c) => c.textContent)
    expect(cells[0]).toBe('Commit86c49e1 Ready the registry')
    expect(cells[1]).toMatch(/^Started.* · a push to GitHub by a teammate$/)
    expect(cells[2]).toBe('Checked firstNo.')
    expect(t).toContain('Where it stopped')
    expect(t).toContain('Plan ✓ · Build ✓ · Database and secrets ✓ · Workers ✕ · Address not run')
    expect(el.querySelector('pre.iii-cloud-log')?.textContent).toContain('group app has no http package')
    expect(byText('a', 'Full log in iii cloud')?.getAttribute('href')).toContain('/deployments/2')
    expect(el.querySelector('aside')).toBeNull()
    expect(el.querySelector('[role=toolbar]')?.textContent).toContain('Failed')
  })

  it('nothing was live then: nobody saw an error', async () => {
    const only = { ...envPageStatus, deploys: [devDeploys[4]] }
    await mount(
      only,
      ctx({ at: 'env', env: DEV.id, deploy: 'dep-2' }),
      cloudAnswers({ [apiPath('/deployments', DEV.id)]: { deployments: [] } }),
    )
    await settle()
    expect(text()).toContain('Nothing was live on dev then, so nobody saw an error.')
  })

  it('Fix with your agent hands the failure to the agent, then goes to the Deploy place on that deployment', async () => {
    const h = await board('manage-deploys-failed')
    await settle()
    await click(btn('Fix with your agent'))
    await settle()
    expect(h.calls.find((c) => c.id === 'iii-cloud::handoff')?.payload).toMatchObject({
      kind: 'deploy_fix',
      deployment_id: 'dep-2',
    })
    expect(h.compose.mock.calls[0][0].text).toContain('failed at Workers')
    expect(path()).toBe('Projects / registry · dev / Deploy')
    expect(lastCall(h.calls).id).not.toBe('iii-cloud::deploy')
  })

  it('with the agent already on it, the action opens its chat instead', async () => {
    const status = { ...envPageStatus, agent: { ...agent('working'), kind: 'deploy_fix' as const } }
    await mount(status, ctx({ at: 'env', env: DEV.id, deploy: 'dep-2' }), cloud)
    await settle()
    expect(btn('Fix with your agent')).toBeNull()
    expect(btn("Open your agent's chat")).not.toBeNull()
  })

  it('a deployment that did not fail shows its facts and the link, and no agent action', async () => {
    await mount(envPageStatus, ctx({ at: 'env', env: DEV.id, deploy: 'dep-6' }), cloud)
    await settle()
    expect(el.querySelector('h2.iii-cloud-h')?.textContent).toBe('Search: rank by stars')
    const cells = [...el.querySelectorAll('.iii-cloud-cell')].map((c) => c.textContent)
    expect(cells[1]).toContain('you, from this machine')
    expect(cells[2]).toBe('Checked firstYes, before it was deployed.')
    expect(byText('a', 'Full log in iii cloud')).not.toBeUndefined()
    expect(btn('Fix with your agent')).toBeNull()
    expect(text()).not.toContain('Where it stopped')
  })

  it('a deployment that is not among the latest 20 says so and goes back', async () => {
    await mount(envPageStatus, ctx({ at: 'env', env: DEV.id, deploy: 'dep-99' }), cloud)
    await settle()
    expect(text()).toContain('Deployment not found')
    await click(btn('Back to dev'))
    expect(path()).toBe('Projects / registry · dev')
  })

  it('no rollback, no log tabs, no stages list: iii cloud has them', async () => {
    const h = await board('manage-deploys-failed')
    await settle()
    for (const gone of ['Roll back', 'Errors only', 'Open that deployment', 'Stages'])
      expect(text()).not.toContain(gone)
    expect(
      h.calls.filter((c) => c.id === 'iii-cloud::api').some((c) => /logs|rollback/.test(String(c.payload?.path))),
    ).toBe(false)
  })
})

describe('on a narrow pane (Phone-Failed)', () => {
  it('one column: the facts with where it stopped, and the action in a bar at the bottom', async () => {
    ui.narrow = true
    await board('manage-deploys-failed')
    await settle()
    const cells = [...el.querySelectorAll('.iii-cloud-cell')].map((c) => c.textContent)
    expect(cells).toHaveLength(4)
    expect(cells[3]).toBe('Stopped atWorkers, the fourth of five stages')
    const bar = el.querySelector('.iii-cloud-recbar')
    expect(bar?.querySelector('button')?.textContent).toBe('Fix with your agent')
    expect(el.querySelector('.iii-cloud-rec-head button')).toBeNull()
    expect(bar?.textContent).toContain('Its fix goes through the check, and you confirm the deploy.')
  })

  it('the environment page on a phone keeps the deployments as rows with the word at the end', async () => {
    ui.narrow = true
    await board('env-page')
    await settle()
    expect(rows()).toHaveLength(6)
    expect(rowOf(2)?.textContent).toContain('Failed')
  })
})

describe('Open on a row', () => {
  it('each Open names its deployment; a held one opens the Deploy screen, where it says what to do; a failed one its record', async () => {
    const held = { ...devDeploys[0], status: 'needs_action' as const, outcome: 'running' as const, finished_at: null }
    const status = { ...envPageStatus, deploys: [held, ...devDeploys.slice(1)] }
    await mount(status, ctx({ at: 'env', env: DEV.id }), cloud)
    await settle()
    expect(btn(`Open deployment #${held.number}`)).toBeTruthy()
    await click(btn(`Open deployment #${held.number}`))
    await settle()
    expect(path()).toBe('Projects / registry · dev / Deploy')
    // No passed check matches it: it says to check first, with no Confirm to press.
    expect(btn('Confirm deploy')).toBeNull()
    expect(btn(`Check ${held.commit.sha.slice(0, 7)} here`)).toBeTruthy()
    await mount(envPageStatus, ctx({ at: 'env', env: DEV.id }, 2), cloud)
    await settle()
    await click(btn('Open deployment #2'))
    await settle()
    expect(path()).toBe('Projects / registry · dev / Deployment #2')
  })
})
